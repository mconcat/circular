use super::socket::{
    ClaudeInboundConnection, ClaudeSocketConfig, ClaudeSocketPeer, live_registry, protocol_uuid,
};
use super::{ClaudeMessage, ClaudeRegistryEntry};
use circular_runtime::{
    AdmittedPeerEvent, BindRequest, BindingLease, BridgePeerAdapter, DeliveryDisposition,
    DiscoverRequest, DurablyCommittedPeerEvent, OpaqueProviderFields, Peer, PeerAcknowledgeError,
    PeerAcknowledgeFailure, PeerActorIncarnation, PeerAdapterCapabilities, PeerAdapterName,
    PeerAddress, PeerAdvertisementMode, PeerAvailability, PeerBinding, PeerBindingCapabilities,
    PeerBindingId, PeerBody, PeerBridgeContractVersion, PeerCapabilities, PeerCursor,
    PeerDiagnostic, PeerDiscoveryMode, PeerDisplayName, PeerEvent, PeerEventEnvelope,
    PeerEventStream, PeerFailure, PeerId, PeerKind, PeerMessage, PeerMessageId, PeerRealmId,
    PeerReceiveCursorMode, PeerReplyAddressMode, PeerSnapshot, PeerStamp, ProviderMessageId,
    ProviderPeerBridge, SendRequest, SubmissionReceipt, UnbindReceipt, VersionedPeerBridge,
};
use std::collections::{BTreeSet, VecDeque};
use std::num::NonZeroUsize;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The measured reply-address prefix. An inbound `from` and a registry
/// `messagingSocketPath` name the same endpoint through it.
const REPLY_SCHEME: &str = "uds:";

/// The provider's own message ceilings are about one million
/// characters, and one line completed within thirty seconds. The byte ceiling is
/// that character count at the widest UTF-8 encoding. These bound this transport's
/// frames; they are not Circular contract values and no config key exposes them.
const MAX_LINE_BYTES: usize = 4_000_000;
const LINE_TIMEOUT: Duration = Duration::from_secs(30);

/// A healthy receiver closes with zero bytes. Anything else is unmeasured, so a
/// small capture is kept to name it in the failure rather than to interpret it.
const MAX_RESPONSE_BYTES: usize = 64;

#[derive(Clone, Debug, Default)]
pub struct ClaudeAdvertisementSlot(Arc<Mutex<Option<AdvertisementHolder>>>);

#[derive(Clone, Debug)]
struct AdvertisementHolder {
    actor: PeerActorIncarnation,
    realm: PeerRealmId,
    name: PeerDisplayName,
    socket: Option<PathBuf>,
}

impl ClaudeAdvertisementSlot {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Option<AdvertisementHolder>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn claim(
        &self,
        actor: PeerActorIncarnation,
        realm: PeerRealmId,
        name: PeerDisplayName,
    ) -> Option<ClaudeAdvertisementClaim> {
        let mut held = self.held();
        if held.is_some() {
            return None;
        }
        *held = Some(AdvertisementHolder {
            actor,
            realm,
            name,
            socket: None,
        });
        drop(held);
        Some(ClaudeAdvertisementClaim(self.clone()))
    }

    fn advertised_socket(&self) -> Option<PathBuf> {
        self.held().as_ref().and_then(|held| held.socket.clone())
    }

    #[must_use]
    pub fn holder(&self) -> Option<(PeerActorIncarnation, PeerRealmId, PeerDisplayName)> {
        self.held()
            .as_ref()
            .map(|held| (held.actor.clone(), held.realm.clone(), held.name.clone()))
    }
}

#[derive(Debug)]
struct ClaudeAdvertisementClaim(ClaudeAdvertisementSlot);

impl ClaudeAdvertisementClaim {
    fn attach(&self, socket: &Path) {
        if let Some(held) = self.0.held().as_mut() {
            held.socket = Some(socket.to_owned());
        }
    }
}

impl Drop for ClaudeAdvertisementClaim {
    fn drop(&mut self) {
        *self.0.held() = None;
    }
}

/// One inbound envelope and, while it is still an inbound message, the open
/// connection whose close is the provider's acknowledgement.
struct PendingEvent {
    envelope: PeerEventEnvelope,
    connection: Option<ClaudeInboundConnection>,
}

struct BoundPeer {
    binding: PeerBinding,
    realm: PeerRealmId,
    capacity: NonZeroUsize,
    waiter: ReceiveWaiter,
    peer: ClaudeSocketPeer,
    claim: ClaudeAdvertisementClaim,
    pending: VecDeque<PendingEvent>,
}

/// The live Claude peer adapter. It never holds provider credentials, and it does
/// not own the advertisement: it borrows this process's single slot for exactly
/// the peer actor that claimed it (see the module header).
pub struct ClaudePeerBridge {
    config: ClaudeSocketConfig,
    slot: ClaudeAdvertisementSlot,
    version: PeerBridgeContractVersion,
    name: PeerAdapterName,
    capabilities: PeerAdapterCapabilities,
    bound: Option<BoundPeer>,
    sent: BTreeSet<PeerMessageId>,
    next: u64,
    waker: Option<std::task::Waker>,
}

impl ClaudePeerBridge {
    pub fn register(
        name: PeerAdapterName,
        config: ClaudeSocketConfig,
        slot: ClaudeAdvertisementSlot,
    ) -> BridgePeerAdapter<Self> {
        BridgePeerAdapter::new(Self::new(name, config, slot))
    }

    fn new(
        name: PeerAdapterName,
        config: ClaudeSocketConfig,
        slot: ClaudeAdvertisementSlot,
    ) -> Self {
        Self {
            config,
            slot,
            version: PeerBridgeContractVersion::try_new(format!(
                "claude-{}/peerProtocol-1",
                super::VERSION
            ))
            .expect("the measured Claude version is a nonempty contract version"),
            name,
            capabilities: PeerAdapterCapabilities::new(
                PeerDiscoveryMode::Snapshot,
                PeerAdvertisementMode::NativePeer,
                false,
                false,
                PeerReplyAddressMode::Native,
                [DeliveryDisposition::Accepted],
                PeerReceiveCursorMode::AdapterOwned,
                false,
                false,
            ),
            bound: None,
            sent: BTreeSet::new(),
            next: 1,
            waker: None,
        }
    }

    fn stamp(&mut self) -> PeerStamp {
        let stamp = PeerStamp::new(self.next);
        self.next = self.next.saturating_add(1);
        stamp
    }

    fn cursor(&mut self) -> PeerCursor {
        let cursor = PeerCursor::new(self.next);
        self.next = self.next.saturating_add(1);
        cursor
    }

    fn bound_mut(&mut self, binding: &PeerBindingId) -> Result<&mut BoundPeer, PeerFailure> {
        match &mut self.bound {
            Some(bound) if bound.binding.id() == binding => Ok(bound),
            _ => Err(PeerFailure::BindingNotFound(binding.clone())),
        }
    }

    fn arm(&mut self, binding: &PeerBindingId) -> Result<(), PeerFailure> {
        let Some(waker) = self.waker.clone() else {
            return Ok(());
        };
        let bound = self.bound_mut(binding)?;
        let room = bound.capacity.get().saturating_sub(bound.pending.len());
        let (fds, deadline) = bound
            .peer
            .readiness(room, LINE_TIMEOUT)
            .map_err(|_| PeerFailure::AdapterUnavailable)?;
        if fds.is_empty() && deadline.is_none() {
            return Ok(());
        }
        bound.waiter.arm(Armed {
            fds,
            deadline,
            waker,
        })
    }
}

struct Armed {
    fds: Vec<OwnedFd>,
    deadline: Option<Instant>,
    waker: std::task::Waker,
}

struct ReceiveWaiter {
    arms: Option<mpsc::Sender<Armed>>,
    wake: Arc<crate::wake::Wake>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ReceiveWaiter {
    fn start() -> std::io::Result<Self> {
        let wake = Arc::new(crate::wake::Wake::new()?);
        let (arms, armed) = mpsc::channel();
        let worker_wake = Arc::clone(&wake);
        let worker = std::thread::Builder::new()
            .name("claude-peer-wait".to_owned())
            .spawn(move || wait_for_arrivals(&armed, &worker_wake))?;
        Ok(Self {
            arms: Some(arms),
            wake,
            worker: Some(worker),
        })
    }

    fn arm(&self, armed: Armed) -> Result<(), PeerFailure> {
        self.arms
            .as_ref()
            .ok_or(PeerFailure::AdapterUnavailable)?
            .send(armed)
            .map_err(|_| PeerFailure::AdapterUnavailable)?;
        self.wake.notify();
        Ok(())
    }
}

impl Drop for ReceiveWaiter {
    fn drop(&mut self) {
        drop(self.arms.take());
        self.wake.notify();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn wait_for_arrivals(armed: &mpsc::Receiver<Armed>, wake: &crate::wake::Wake) {
    let mut current: Option<Armed> = None;
    loop {
        wake.drain();
        loop {
            match armed.try_recv() {
                Ok(next) => current = Some(next),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        let Some(waiting) = current.as_ref() else {
            match armed.recv() {
                Ok(next) => current = Some(next),
                Err(_) => return,
            }
            continue;
        };
        let mut fds = Vec::with_capacity(waiting.fds.len() + 1);
        fds.push(wake.as_fd());
        fds.extend(waiting.fds.iter().map(AsFd::as_fd));
        let answered = crate::wake::wait_readable_until(&fds, waiting.deadline);
        let deadline = waiting.deadline;
        let descriptors = fds.len();
        drop(fds);
        let arrived = match answered {
            Ok(woke) => {
                !woke.at(0)
                    || (1..descriptors).any(|index| woke.at(index))
                    || deadline.is_some_and(|deadline| Instant::now() >= deadline)
            }
            Err(_) => {
                if let Some(done) = current.take() {
                    done.waker.wake();
                }
                return;
            }
        };
        if arrived {
            if let Some(done) = current.take() {
                drop(done.fds);
                done.waker.wake();
            }
        }
    }
}

/// `uds:/path/to/<pid>.sock` — the exact bytes the provider uses for a sender.
fn reply_address(socket: &Path) -> Option<String> {
    Some(format!("{REPLY_SCHEME}{}", socket.to_str()?))
}

fn peer_id(socket: &Path) -> Option<PeerId> {
    PeerId::try_new(reply_address(socket)?.into_bytes()).ok()
}

/// The inverse: a peer identity is only addressable if it is a reply address.
fn peer_socket(peer: &PeerId) -> Option<PathBuf> {
    let text = std::str::from_utf8(peer.as_bytes()).ok()?;
    let path = text.strip_prefix(REPLY_SCHEME)?;
    Path::new(path).is_absolute().then(|| PathBuf::from(path))
}

fn diagnostic(code: &str, detail: impl AsRef<str>) -> PeerEvent {
    PeerEvent::AdapterDiagnostic(PeerDiagnostic::new(code, detail.as_ref()))
}

impl VersionedPeerBridge for ClaudePeerBridge {
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

impl ProviderPeerBridge for ClaudePeerBridge {
    fn set_receive_waker(&mut self, waker: std::task::Waker) {
        self.waker = Some(waker);
    }

    fn discover(&mut self, request: DiscoverRequest) -> Result<PeerSnapshot, PeerFailure> {
        if request.adapter() != &self.name {
            return Err(PeerFailure::WrongAdapter {
                expected: self.name.clone(),
                actual: request.adapter().clone(),
            });
        }
        let own = self.slot.advertised_socket();
        let candidates = live_registry(&self.config.sessions_dir)
            .map_err(|_| PeerFailure::AdapterUnavailable)?;
        let realm = request.realm().clone();
        let mut peers = Vec::new();
        for (_, candidate) in candidates {
            let Ok(entry) = candidate else { continue };
            if own.as_deref() == Some(entry.socket()) {
                continue;
            }
            let (Some(address), Ok(display)) = (
                peer_id(entry.socket()),
                PeerDisplayName::try_new(entry.name()),
            ) else {
                continue;
            };
            peers.push(Peer::new(
                PeerAddress::new(self.name.clone(), realm.clone(), address),
                display,
                PeerKind::Session,
                PeerAvailability::Unknown,
                PeerCapabilities::new(true, false, false),
            ));
        }
        if let Some(name) = request.display_name() {
            peers.retain(|peer| peer.display_name() == name);
        }
        peers.sort_by(|left, right| left.address().cmp(right.address()));
        let observed_at = self.stamp();
        Ok(PeerSnapshot::new(realm, peers, observed_at))
    }

    fn bind(&mut self, request: BindRequest) -> Result<PeerBinding, PeerFailure> {
        let effective_name = match request.requested_name().cloned() {
            Some(name) => name,
            None => PeerDisplayName::try_new(self.config.display_name.clone())
                .map_err(|_| PeerFailure::AdapterUnavailable)?,
        };
        let Some(claim) = self.slot.claim(
            request.actor().clone(),
            request.realm().clone(),
            effective_name.clone(),
        ) else {
            return Err(PeerFailure::UnsupportedCapability);
        };
        let peer = ClaudeSocketPeer::advertise(self.config.clone(), Some(effective_name.as_str()))
            .map_err(|_| PeerFailure::AdapterUnavailable)?;
        let Some(address) = peer_id(peer.socket_path()) else {
            return Err(PeerFailure::AdapterUnavailable);
        };
        let waiter = ReceiveWaiter::start().map_err(|_| PeerFailure::AdapterUnavailable)?;
        claim.attach(peer.socket_path());
        let sequence = self.next;
        self.next = self.next.saturating_add(1);
        let id = PeerBindingId::try_new(
            format!("claude:{}:{sequence}", std::process::id()).into_bytes(),
        )
        .expect("a formatted binding identity is nonempty");
        let binding = PeerBinding::new(
            id,
            request.actor().clone(),
            PeerAddress::new(self.name.clone(), request.realm().clone(), address),
            effective_name,
            BindingLease::Process,
            PeerBindingCapabilities::new(true, true),
        );
        self.bound = Some(BoundPeer {
            binding: binding.clone(),
            realm: request.realm().clone(),
            capacity: request.inbox_capacity(),
            waiter,
            peer,
            claim,
            pending: VecDeque::new(),
        });
        Ok(binding)
    }

    fn send(&mut self, request: SendRequest) -> Result<SubmissionReceipt, PeerFailure> {
        if request.target().adapter() != &self.name {
            return Err(PeerFailure::WrongAdapter {
                expected: self.name.clone(),
                actual: request.target().adapter().clone(),
            });
        }
        if self.sent.contains(request.message()) {
            return Err(PeerFailure::DuplicateMessage(request.message().clone()));
        }
        let sessions = self.config.sessions_dir.clone();
        let bound = self.bound_mut(request.binding())?;
        if &bound.realm != request.target().realm() {
            return Err(PeerFailure::WrongRealm {
                expected: bound.realm.clone(),
                actual: request.target().realm().clone(),
            });
        }
        let Some(socket) = peer_socket(request.target().peer()) else {
            return Err(PeerFailure::PeerNotFound(request.target().clone()));
        };
        let entry = live_registry(&sessions)
            .map_err(|_| PeerFailure::AdapterUnavailable)?
            .into_iter()
            .filter_map(|(_, candidate)| candidate.ok())
            .find(|entry: &ClaudeRegistryEntry| entry.socket() == socket)
            .ok_or_else(|| PeerFailure::PeerNotFound(request.target().clone()))?;
        let provider_id = protocol_uuid().map_err(|_| PeerFailure::AdapterUnavailable)?;
        let response = bound
            .peer
            .send(
                &entry,
                &provider_id,
                request.body().as_str(),
                LINE_TIMEOUT,
                MAX_RESPONSE_BYTES,
            )
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                    PeerFailure::PeerNotFound(request.target().clone())
                }
                _ => PeerFailure::AdapterUnavailable,
            })?;
        if !response.is_empty() {
            return Err(PeerFailure::SubmissionUnknown);
        }
        let provider_id = ProviderMessageId::try_new(provider_id.into_bytes())
            .expect("a formatted provider message id is nonempty");
        let accepted_at = self.stamp();
        self.sent.insert(request.message().clone());
        SubmissionReceipt::try_new(
            request.message().clone(),
            Some(provider_id),
            DeliveryDisposition::Accepted,
            accepted_at,
        )
        .map_err(|_| PeerFailure::AdapterUnavailable)
    }

    fn receive(
        &mut self,
        binding: &PeerBindingId,
        after: Option<PeerCursor>,
    ) -> Result<PeerEventStream, PeerFailure> {
        {
            let bound = self.bound_mut(binding)?;
            let room = bound.capacity.get().saturating_sub(bound.pending.len());
            if room > 0 {
                let address = bound.binding.address().clone();
                let realm = bound.realm.clone();
                let ready = bound.peer.poll(room, MAX_LINE_BYTES, LINE_TIMEOUT);
                let mut arrivals = Vec::with_capacity(ready.len());
                for connection in ready {
                    arrivals.push(match connection {
                        Ok(connection) => {
                            match inbound_message(&connection.message, &address, &realm) {
                                Some(message) => (PeerEvent::Inbound(message), Some(connection)),
                                None => (
                                    diagnostic(
                                        "claude_inbound_address",
                                        "Claude inbound reply address is not an addressable peer",
                                    ),
                                    None,
                                ),
                            }
                        }
                        Err(error) => (diagnostic("claude_inbound_line", error.to_string()), None),
                    });
                }
                for (event, connection) in arrivals {
                    let cursor = self.cursor();
                    let bound = self.bound_mut(binding)?;
                    bound.pending.push_back(PendingEvent {
                        envelope: PeerEventEnvelope::new(binding.clone(), cursor, event),
                        connection,
                    });
                }
            }
        }
        let bound = self.bound_mut(binding)?;
        let events = bound
            .pending
            .iter()
            .filter(|pending| after.is_none_or(|cursor| pending.envelope.cursor() > cursor))
            .map(|pending| pending.envelope.clone())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        if events.is_empty() {
            self.arm(binding)?;
        }
        Ok(PeerEventStream::new(events))
    }

    fn acknowledge(
        &mut self,
        committed: DurablyCommittedPeerEvent,
    ) -> Result<AdmittedPeerEvent, PeerAcknowledgeError> {
        let binding = committed.binding().clone();
        let Ok(bound) = self.bound_mut(&binding) else {
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
        if front.envelope.cursor() != committed.cursor() {
            let expected = front.envelope.cursor();
            let actual = committed.cursor();
            return Err(PeerAcknowledgeError::refused(
                committed,
                PeerAcknowledgeFailure::OutOfOrder { expected, actual },
            ));
        }
        if &front.envelope != committed.envelope() {
            return Err(PeerAcknowledgeError::refused(
                committed,
                PeerAcknowledgeFailure::EventMismatch,
            ));
        }
        let admitted = bound.pending.pop_front().expect("front was just inspected");
        if let Some(connection) = admitted.connection {
            connection.acknowledge();
        }
        Ok(AdmittedPeerEvent::admit(committed))
    }

    fn unbind(&mut self, binding: &PeerBindingId) -> Result<UnbindReceipt, PeerFailure> {
        let bound = self.bound_mut(binding)?;
        if !bound.pending.is_empty() {
            return Err(PeerFailure::BindingHasPendingEvents {
                binding: binding.clone(),
                count: bound.pending.len(),
            });
        }
        let address = bound.binding.address().clone();
        self.bound = None;
        let closed_at = self.stamp();
        Ok(UnbindReceipt::new(binding.clone(), address, closed_at))
    }
}

/// Build the graph-visible message from one measured line. The sender is its own
/// reply address, which is also where a reply goes.
fn inbound_message(
    line: &ClaudeMessage,
    to: &PeerAddress,
    realm: &PeerRealmId,
) -> Option<PeerMessage> {
    let socket = Path::new(line.from.strip_prefix(REPLY_SCHEME)?);
    let from = PeerAddress::new(to.adapter().clone(), realm.clone(), peer_id(socket)?);
    let provider_id = ProviderMessageId::try_new(line.provider_id.clone().into_bytes()).ok()?;
    let id = PeerMessageId::try_new(line.provider_id.clone().into_bytes()).ok()?;
    let body = PeerBody::try_new(line.body.clone()).ok()?;
    Some(PeerMessage::external(
        id,
        Some(provider_id),
        from.clone(),
        to.clone(),
        Some(from),
        None,
        body,
        OpaqueProviderFields::try_new([
            ("from_name", line.from_name.as_bytes()),
            ("from_mode", line.from_mode.as_bytes()),
        ])
        .ok()?,
    ))
}

