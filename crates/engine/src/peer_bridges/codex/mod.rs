//! Codex 0.157 app-server boundary.
//!
//! A submission receipt is the response to turn/start or turn/steer. Agent
//! messages are separate inbound events. No reply tool, provider policy, timer,
//! retry, or turn-completion wait lives here. Registration truthfully reports
//! Unsupported advertisement until the common registration restriction is removed.
mod transport;

use circular_runtime::{
    AdmittedPeerEvent, BindRequest, BindingLease, BridgePeerAdapter, DeliveryDisposition,
    DiscoverRequest, DurablyCommittedPeerEvent, OpaqueProviderFields, Peer, PeerAcknowledgeError,
    PeerAcknowledgeFailure, PeerAdapterCapabilities, PeerAdapterName, PeerAddress,
    PeerAdvertisementMode, PeerAvailability, PeerBinding, PeerBindingCapabilities, PeerBindingId,
    PeerBody, PeerBridgeContractVersion, PeerCapabilities, PeerCursor, PeerDiscoveryMode,
    PeerDisplayName, PeerEvent, PeerEventEnvelope, PeerEventStream, PeerFailure, PeerId, PeerKind,
    PeerMessage, PeerMessageId, PeerReceiveCursorMode, PeerReplyAddressMode, PeerSnapshot,
    PeerStamp, ProviderMessageId, ProviderPeerBridge, SendRequest, SubmissionReceipt,
    UnbindReceipt, VersionedPeerBridge,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::NonZeroUsize;
use std::task::Waker;
use transport::{Proxy, Transport};

/// A provider turn-page cursor and consumed item IDs within that page.
/// `nextCursor: null` means end-of-history, not an opaque cursor for that end.
#[derive(Default)]
struct Position {
    page: Option<String>,
    consumed: BTreeSet<String>,
}

#[derive(Default)]
struct ThreadInbox {
    submitted_turns: BTreeSet<String>,
    scanned: Position,
    dirty: bool,
}

struct Bound {
    binding: PeerBinding,
    capacity: NonZeroUsize,
    threads: BTreeMap<String, ThreadInbox>,
    pending: VecDeque<PeerEventEnvelope>,
    next_cursor: u64,
    sent: BTreeSet<PeerMessageId>,
}

pub struct CodexPeerBridge {
    transport: Box<dyn Transport>,
    version: PeerBridgeContractVersion,
    name: PeerAdapterName,
    capabilities: PeerAdapterCapabilities,
    bound: Option<Bound>,
    next_stamp: u64,
}

pub(crate) fn declared_bridge(
    name: PeerAdapterName,
    _settings: &crate::peer_adapter::PeerSettings,
) -> Result<crate::execution_profile::PeerAdapterFactory, String> {
    let factory: crate::execution_profile::PeerAdapterFactory =
        std::sync::Arc::new(move || Box::new(CodexPeerBridge::register(name.clone())));
    Ok(factory)
}

impl CodexPeerBridge {
    /// Lazy construction: daemon configuration never starts a provider process.
    ///
    /// `name` is the declared adapter name (`peer-adapters/codex.toml`); the bridge
    /// is handed it and never spells it itself.
    pub fn register(name: PeerAdapterName) -> BridgePeerAdapter<Self> {
        BridgePeerAdapter::new(Self::with_transport(name, Box::<Proxy>::default()))
    }

    fn with_transport(name: PeerAdapterName, transport: Box<dyn Transport>) -> Self {
        Self {
            transport,
            version: PeerBridgeContractVersion::try_new("codex-0.157/app-server").unwrap(),
            name,
            capabilities: PeerAdapterCapabilities::new(
                PeerDiscoveryMode::Snapshot,
                PeerAdvertisementMode::Unsupported,
                true,
                true,
                PeerReplyAddressMode::Derived,
                [DeliveryDisposition::Accepted],
                PeerReceiveCursorMode::AdapterOwned,
                false,
                false,
            ),
            bound: None,
            next_stamp: 1,
        }
    }

    fn stamp(&mut self) -> PeerStamp {
        let value = self.next_stamp;
        self.next_stamp += 1;
        PeerStamp::new(value)
    }

    fn check_adapter(&self, adapter: &PeerAdapterName) -> Result<(), PeerFailure> {
        if adapter == &self.name {
            Ok(())
        } else {
            Err(PeerFailure::WrongAdapter {
                expected: self.name.clone(),
                actual: adapter.clone(),
            })
        }
    }

    fn bound(&mut self, binding: &PeerBindingId) -> Result<&mut Bound, PeerFailure> {
        self.bound
            .as_mut()
            .filter(|bound| bound.binding.id() == binding)
            .ok_or_else(|| PeerFailure::BindingNotFound(binding.clone()))
    }

    fn notifications(&mut self) -> Result<(), PeerFailure> {
        for notification in self.transport.notifications()? {
            if !matches!(
                notification["method"].as_str(),
                Some("item/completed" | "turn/completed")
            ) {
                continue;
            }
            let params = &notification["params"];
            let Some(thread) = params["threadId"].as_str() else {
                continue;
            };
            let Some(inbox) = self
                .bound
                .as_mut()
                .and_then(|bound| bound.threads.get_mut(thread))
            else {
                continue;
            };
            inbox.dirty = true;
        }
        Ok(())
    }

    fn read_thread(&mut self, binding: &PeerBindingId, thread: &str) -> Result<(), PeerFailure> {
        loop {
            let bound = self.bound(binding)?;
            if bound.pending.len() >= bound.capacity.get() {
                return Ok(());
            }
            let inbox = bound.threads.get(thread).expect("owned thread");
            if !inbox.dirty {
                return Ok(());
            }
            let page = inbox.scanned.page.clone();
            let result = self.transport.request(
                "thread/turns/list",
                json!({
                    "threadId": thread, "cursor": page, "sortDirection": "asc", "itemsView": "full"
                }),
            )?;
            let turns = array(&result, "data")?;
            let next = optional_string(&result, "nextCursor")?;
            let bound = self.bound(binding)?;
            let inbox = bound.threads.get_mut(thread).expect("owned thread");
            for turn in turns {
                if !inbox.submitted_turns.contains(string(turn, "id")?) {
                    continue;
                }
                if string(turn, "status")? == "inProgress" {
                    inbox.dirty = false;
                    return Ok(());
                }
                for item in array(turn, "items")? {
                    let id = string(item, "id")?;
                    if item["type"] != "agentMessage" || inbox.scanned.consumed.contains(id) {
                        continue;
                    }
                    let message = inbound_message(item, thread, &bound.binding)?;
                    inbox.scanned.consumed.insert(id.to_owned());
                    let cursor = PeerCursor::new(bound.next_cursor);
                    bound.next_cursor += 1;
                    bound.pending.push_back(PeerEventEnvelope::new(
                        binding.clone(),
                        cursor,
                        PeerEvent::Inbound(message),
                    ));
                    if bound.pending.len() >= bound.capacity.get() {
                        return Ok(());
                    }
                }
            }
            if let Some(next) = next {
                inbox.scanned = Position {
                    page: Some(next),
                    consumed: BTreeSet::new(),
                };
            } else {
                inbox.dirty = false;
                return Ok(());
            }
        }
    }
}

impl VersionedPeerBridge for CodexPeerBridge {
    fn contract_version(&self) -> &PeerBridgeContractVersion {
        &self.version
    }
    fn adapter_name(&self) -> &PeerAdapterName {
        &self.name
    }
    fn capabilities(&self) -> &PeerAdapterCapabilities {
        &self.capabilities
    }
}

impl ProviderPeerBridge for CodexPeerBridge {
    fn set_receive_waker(&mut self, waker: Waker) {
        self.transport.set_waker(waker);
    }

    fn discover(&mut self, request: DiscoverRequest) -> Result<PeerSnapshot, PeerFailure> {
        self.check_adapter(request.adapter())?;
        let mut cursor: Option<String> = None;
        let mut threads = BTreeMap::new();
        loop {
            let page = self
                .transport
                .request("thread/list", json!({"cursor": cursor}))?;
            for thread in array(&page, "data")? {
                threads.insert(string(thread, "id")?.to_owned(), thread.clone());
            }
            cursor = optional_string(&page, "nextCursor")?;
            if cursor.is_none() {
                break;
            }
        }
        let mut peers = Vec::new();
        for (id, thread) in threads {
            let display = thread["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .unwrap_or(&id);
            let display =
                PeerDisplayName::try_new(display).map_err(|_| PeerFailure::AdapterUnavailable)?;
            if request.display_name().is_some_and(|name| name != &display) {
                continue;
            }
            let availability = match thread["status"]["type"].as_str() {
                Some("idle") => PeerAvailability::Idle,
                Some("active") => PeerAvailability::Busy,
                _ => PeerAvailability::Unknown,
            };
            peers.push(Peer::new(
                PeerAddress::new(self.name.clone(), request.realm().clone(), peer_id(&id)?),
                display,
                PeerKind::Thread,
                availability,
                PeerCapabilities::new(true, true, true),
            ));
        }
        Ok(PeerSnapshot::new(
            request.realm().clone(),
            peers,
            self.stamp(),
        ))
    }

    fn bind(&mut self, request: BindRequest) -> Result<PeerBinding, PeerFailure> {
        self.check_adapter(request.adapter())?;
        if self.bound.is_some() {
            return Err(PeerFailure::UnsupportedCapability);
        }
        self.transport.open()?;
        let mut identity = [0_u8; 16];
        getrandom::fill(&mut identity).map_err(|_| PeerFailure::AdapterUnavailable)?;
        let binding = PeerBinding::new(
            PeerBindingId::try_new(identity.as_slice()).unwrap(),
            request.actor().clone(),
            PeerAddress::new(
                self.name.clone(),
                request.realm().clone(),
                PeerId::try_new(identity.as_slice()).unwrap(),
            ),
            request
                .requested_name()
                .cloned()
                .unwrap_or_else(|| PeerDisplayName::try_new("circular").unwrap()),
            BindingLease::Process,
            PeerBindingCapabilities::new(true, false),
        );
        self.bound = Some(Bound {
            binding: binding.clone(),
            capacity: request.inbox_capacity(),
            threads: BTreeMap::new(),
            pending: VecDeque::new(),
            next_cursor: 1,
            sent: BTreeSet::new(),
        });
        Ok(binding)
    }

    fn send(&mut self, request: SendRequest) -> Result<SubmissionReceipt, PeerFailure> {
        self.check_adapter(request.target().adapter())?;
        let bound = self.bound(request.binding())?;
        if bound.binding.address().realm() != request.target().realm() {
            return Err(PeerFailure::WrongRealm {
                expected: bound.binding.address().realm().clone(),
                actual: request.target().realm().clone(),
            });
        }
        if bound.sent.contains(request.message()) {
            return Err(PeerFailure::DuplicateMessage(request.message().clone()));
        }
        let thread = std::str::from_utf8(request.target().peer().as_bytes())
            .map_err(|_| PeerFailure::PeerNotFound(request.target().clone()))?;
        let resumed = self
            .transport
            .request("thread/resume", json!({"threadId": thread}))?;
        let turns = array(&resumed["thread"], "turns")?;
        let active = turns.iter().find(|turn| turn["status"] == "inProgress");
        let input = json!([{"type": "text", "text": request.body().as_str(), "text_elements": []}]);
        let (method, params) = match active {
            Some(turn) => (
                "turn/steer",
                json!({"threadId": thread, "input": input, "expectedTurnId": string(turn, "id")?}),
            ),
            None => ("turn/start", json!({"threadId": thread, "input": input})),
        };
        self.bound(request.binding())?
            .threads
            .entry(thread.to_owned())
            .or_default();
        let accepted = self.transport.request(method, params)?;
        let turn = if active.is_some() {
            string(&accepted, "turnId")?
        } else {
            string(&accepted["turn"], "id")?
        };
        let bound = self.bound(request.binding())?;
        let inbox = bound.threads.get_mut(thread).expect("subscribed above");
        if active.is_none() {
            inbox.submitted_turns.insert(turn.to_owned());
        }
        inbox.dirty = true;
        bound.sent.insert(request.message().clone());
        SubmissionReceipt::try_new(
            request.message().clone(),
            Some(
                ProviderMessageId::try_new(turn.as_bytes())
                    .map_err(|_| PeerFailure::AdapterUnavailable)?,
            ),
            DeliveryDisposition::Accepted,
            self.stamp(),
        )
        .map_err(|_| PeerFailure::AdapterUnavailable)
    }

    fn receive(
        &mut self,
        binding: &PeerBindingId,
        after: Option<PeerCursor>,
    ) -> Result<PeerEventStream, PeerFailure> {
        self.bound(binding)?;
        self.notifications()?;
        let threads = self
            .bound(binding)?
            .threads
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for thread in threads {
            self.read_thread(binding, &thread)?;
        }
        Ok(PeerEventStream::new(
            self.bound(binding)?
                .pending
                .iter()
                .filter(|pending| after.is_none_or(|cursor| pending.cursor() > cursor))
                .cloned()
                .collect::<Vec<_>>(),
        ))
    }

    fn acknowledge(
        &mut self,
        committed: DurablyCommittedPeerEvent,
    ) -> Result<AdmittedPeerEvent, PeerAcknowledgeError> {
        let binding = committed.binding().clone();
        let Ok(bound) = self.bound(&binding) else {
            return Err(PeerAcknowledgeError::refused(
                committed,
                PeerAcknowledgeFailure::BindingNotFound(binding),
            ));
        };
        let Some(front) = bound.pending.front() else {
            return Err(PeerAcknowledgeError::refused(
                committed,
                PeerAcknowledgeFailure::NoPendingEvent,
            ));
        };
        if front.cursor() != committed.cursor() {
            let reason = PeerAcknowledgeFailure::OutOfOrder {
                expected: front.cursor(),
                actual: committed.cursor(),
            };
            return Err(PeerAcknowledgeError::refused(committed, reason));
        }
        if front != committed.envelope() {
            return Err(PeerAcknowledgeError::refused(
                committed,
                PeerAcknowledgeFailure::EventMismatch,
            ));
        }
        bound.pending.pop_front().expect("front inspected");
        Ok(AdmittedPeerEvent::admit(committed))
    }

    fn unbind(&mut self, binding: &PeerBindingId) -> Result<UnbindReceipt, PeerFailure> {
        self.receive(binding, None)?;
        let bound = self.bound(binding)?;
        if !bound.pending.is_empty() {
            return Err(PeerFailure::BindingHasPendingEvents {
                binding: binding.clone(),
                count: bound.pending.len(),
            });
        }
        let address = bound.binding.address().clone();
        let threads = bound.threads.keys().cloned().collect::<Vec<_>>();
        for thread in threads {
            self.transport
                .request("thread/unsubscribe", json!({"threadId": thread}))?;
        }
        self.transport.close();
        self.bound = None;
        Ok(UnbindReceipt::new(binding.clone(), address, self.stamp()))
    }
}

fn inbound_message(
    item: &Value,
    thread: &str,
    binding: &PeerBinding,
) -> Result<PeerMessage, PeerFailure> {
    let id = string(item, "id")?;
    let from = PeerAddress::new(
        binding.address().adapter().clone(),
        binding.address().realm().clone(),
        peer_id(thread)?,
    );
    Ok(PeerMessage::external(
        PeerMessageId::try_new(id.as_bytes()).map_err(|_| PeerFailure::AdapterUnavailable)?,
        Some(
            ProviderMessageId::try_new(id.as_bytes())
                .map_err(|_| PeerFailure::AdapterUnavailable)?,
        ),
        from.clone(),
        binding.address().clone(),
        Some(from),
        None,
        PeerBody::try_new(string(item, "text")?).map_err(|_| PeerFailure::AdapterUnavailable)?,
        OpaqueProviderFields::empty(),
    ))
}

fn peer_id(value: &str) -> Result<PeerId, PeerFailure> {
    PeerId::try_new(value.as_bytes()).map_err(|_| PeerFailure::AdapterUnavailable)
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, PeerFailure> {
    value[key].as_str().ok_or(PeerFailure::AdapterUnavailable)
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], PeerFailure> {
    value[key]
        .as_array()
        .map(Vec::as_slice)
        .ok_or(PeerFailure::AdapterUnavailable)
}
fn optional_string(value: &Value, key: &str) -> Result<Option<String>, PeerFailure> {
    if value[key].is_null() {
        Ok(None)
    } else {
        string(value, key).map(|value| Some(value.to_owned()))
    }
}

