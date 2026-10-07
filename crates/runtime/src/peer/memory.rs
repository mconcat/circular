use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::num::NonZeroUsize;

use super::adapter::*;
use super::ids::*;
use super::ingress::*;
use super::peer::*;
use super::protocol::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoryPeerContactCounts {
    pub discover: usize,
    pub bind: usize,
    pub send: usize,
    pub receive: usize,
    pub acknowledge: usize,
    pub unbind: usize,
    pub provider_inject: usize,
}

#[derive(Debug)]
struct MemoryBinding {
    public: PeerBinding,
    inbound_policy: InboundPolicy,
    inbox_capacity: NonZeroUsize,
    pending: VecDeque<PeerEventEnvelope>,
}

impl MemoryBinding {
    fn admit_inbound(&self, source: &PeerAddress) -> Result<(), PeerFailure> {
        let pending_inbound = self
            .pending
            .iter()
            .filter(|event| matches!(event.event(), PeerEvent::Inbound(_)))
            .count();
        if pending_inbound >= self.inbox_capacity.get() {
            return Err(PeerFailure::InboxFull {
                binding: self.public.id().clone(),
                capacity: self.inbox_capacity,
            });
        }
        if !self.inbound_policy.allows(source) {
            return Err(PeerFailure::InboundRefused(source.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
struct MemoryRealm {
    external_peers: BTreeMap<PeerId, Peer>,
    external_inboxes: BTreeMap<PeerId, Vec<PeerMessage>>,
    bindings: BTreeMap<PeerBindingId, MemoryBinding>,
}

/// Deterministic reference adapter used by the cross-provider conformance suite.
#[derive(Debug)]
pub struct MemoryPeerAdapter {
    receive_waker: Option<std::task::Waker>,
    name: PeerAdapterName,
    capabilities: PeerAdapterCapabilities,
    realms: BTreeMap<PeerRealmId, MemoryRealm>,
    retired_bindings: BTreeSet<PeerBindingId>,
    retired_addresses: BTreeSet<PeerAddress>,
    outbound_message_ids: BTreeSet<PeerMessageId>,
    provider_dedupe: BTreeMap<(PeerAddress, ProviderMessageId), PeerCursor>,
    next_binding: u64,
    next_peer: u64,
    next_message: u64,
    next_stamp: u64,
    counts: MemoryPeerContactCounts,
}

impl MemoryPeerAdapter {
    #[must_use]
    pub fn new(name: PeerAdapterName) -> Self {
        Self {
            receive_waker: None,
            name,
            capabilities: PeerAdapterCapabilities::new(
                PeerDiscoveryMode::Snapshot,
                PeerAdvertisementMode::NativePeer,
                true,
                true,
                PeerReplyAddressMode::Native,
                [
                    DeliveryDisposition::Accepted,
                    DeliveryDisposition::Delivered,
                ],
                PeerReceiveCursorMode::AdapterOwned,
                true,
                false,
            ),
            realms: BTreeMap::new(),
            retired_bindings: BTreeSet::new(),
            retired_addresses: BTreeSet::new(),
            outbound_message_ids: BTreeSet::new(),
            provider_dedupe: BTreeMap::new(),
            next_binding: 1,
            next_peer: 1,
            next_message: 1,
            next_stamp: 1,
            counts: MemoryPeerContactCounts::default(),
        }
    }

    pub fn register_external_peer(
        &mut self,
        realm: PeerRealmId,
        display_name: PeerDisplayName,
        kind: PeerKind,
        availability: PeerAvailability,
        capabilities: PeerCapabilities,
    ) -> Peer {
        let peer_id = generated_id(b"peer", self.next_peer);
        self.next_peer = self.next_peer.saturating_add(1);
        let address = PeerAddress::new(self.name.clone(), realm.clone(), peer_id);
        let peer = Peer::new(address, display_name, kind, availability, capabilities);
        let state = self.realms.entry(realm).or_default();
        state
            .external_peers
            .insert(peer.address().peer().clone(), peer.clone());
        state
            .external_inboxes
            .insert(peer.address().peer().clone(), Vec::new());
        peer
    }

    #[must_use]
    pub fn external_inbox(&self, address: &PeerAddress) -> Option<&[PeerMessage]> {
        self.realms
            .get(address.realm())?
            .external_inboxes
            .get(address.peer())
            .map(Vec::as_slice)
    }

    #[must_use]
    pub const fn contact_counts(&self) -> MemoryPeerContactCounts {
        self.counts
    }

    pub fn inject_external(
        &mut self,
        request: ExternalPeerSend,
    ) -> Result<ProviderIngressReceipt, PeerFailure> {
        self.counts.provider_inject += 1;
        self.validate_address(&request.from)?;
        self.validate_address(&request.to)?;
        if request.from.realm() != request.to.realm() {
            return Err(PeerFailure::WrongRealm {
                expected: request.to.realm().clone(),
                actual: request.from.realm().clone(),
            });
        }
        if self.retired_addresses.contains(&request.to) {
            return Err(PeerFailure::AddressStale(request.to));
        }

        let realm_id = request.to.realm().clone();
        let dedupe_source = request.from.clone();
        if let Some(provider_id) = request.provider_id.as_ref()
            && let Some(cursor) = self
                .provider_dedupe
                .get(&(dedupe_source.clone(), provider_id.clone()))
        {
            return Ok(ProviderIngressReceipt::Duplicate {
                original_cursor: *cursor,
            });
        }

        let binding_id = {
            let realm = self
                .realms
                .get(&realm_id)
                .ok_or_else(|| PeerFailure::PeerNotFound(request.from.clone()))?;
            if !realm.external_peers.contains_key(request.from.peer()) {
                return Err(PeerFailure::PeerNotFound(request.from));
            }
            realm
                .bindings
                .iter()
                .find(|(_, binding)| binding.public.address() == &request.to)
                .map(|(id, _)| id.clone())
                .ok_or_else(|| PeerFailure::PeerNotFound(request.to.clone()))?
        };

        self.binding_mut(&realm_id, &binding_id)?
            .admit_inbound(&request.from)?;

        let message_id = generated_id(b"inbound", self.next_message);
        self.next_message = self.next_message.saturating_add(1);
        let cursor = self.next_cursor();
        let message = PeerMessage::external(
            message_id,
            request.provider_id.clone(),
            request.from.clone(),
            request.to,
            request.reply_to.or(Some(request.from)),
            request.correlation,
            request.body,
            request.provider_fields,
        );
        let envelope = PeerEventEnvelope {
            binding: binding_id.clone(),
            cursor,
            event: PeerEvent::Inbound(message),
        };
        self.binding_mut(&realm_id, &binding_id)?
            .pending
            .push_back(envelope);
        if let Some(provider_id) = request.provider_id {
            self.provider_dedupe
                .insert((dedupe_source, provider_id), cursor);
        }
        if let Some(waker) = &self.receive_waker {
            waker.wake_by_ref();
        }
        Ok(ProviderIngressReceipt::Accepted { cursor })
    }

    fn binding_mut(
        &mut self,
        realm: &PeerRealmId,
        binding: &PeerBindingId,
    ) -> Result<&mut MemoryBinding, PeerFailure> {
        self.realms
            .get_mut(realm)
            .and_then(|realm| realm.bindings.get_mut(binding))
            .ok_or_else(|| PeerFailure::BindingNotFound(binding.clone()))
    }

    fn validate_address(&self, address: &PeerAddress) -> Result<(), PeerFailure> {
        if address.adapter() == &self.name {
            Ok(())
        } else {
            Err(PeerFailure::WrongAdapter {
                expected: self.name.clone(),
                actual: address.adapter().clone(),
            })
        }
    }

    fn next_cursor(&mut self) -> PeerCursor {
        let cursor = PeerCursor::new(self.next_stamp);
        self.next_stamp = self.next_stamp.saturating_add(1);
        cursor
    }

    fn next_stamp(&mut self) -> PeerStamp {
        let stamp = PeerStamp::new(self.next_stamp);
        self.next_stamp = self.next_stamp.saturating_add(1);
        stamp
    }

    fn find_binding_realm(&self, binding: &PeerBindingId) -> Result<PeerRealmId, PeerFailure> {
        if let Some(realm) = self
            .realms
            .iter()
            .find_map(|(realm, state)| state.bindings.contains_key(binding).then(|| realm.clone()))
        {
            Ok(realm)
        } else if self.retired_bindings.contains(binding) {
            Err(PeerFailure::BindingStale(binding.clone()))
        } else {
            Err(PeerFailure::BindingNotFound(binding.clone()))
        }
    }
}

impl PeerAdapter for MemoryPeerAdapter {
    fn set_receive_waker(&mut self, waker: std::task::Waker) {
        self.receive_waker = Some(waker);
    }
    fn name(&self) -> &PeerAdapterName {
        &self.name
    }

    fn capabilities(&self) -> &PeerAdapterCapabilities {
        &self.capabilities
    }

    fn discover(&mut self, request: DiscoverRequest) -> Result<PeerSnapshot, PeerFailure> {
        self.counts.discover += 1;
        if request.adapter() != &self.name {
            return Err(PeerFailure::WrongAdapter {
                expected: self.name.clone(),
                actual: request.adapter().clone(),
            });
        }
        let observed_at = self.next_stamp();
        let mut peers = Vec::new();
        if let Some(realm) = self.realms.get(request.realm()) {
            peers.extend(realm.external_peers.values().cloned());
            peers.extend(realm.bindings.values().map(|binding| {
                Peer::new(
                    binding.public.address().clone(),
                    binding.public.effective_name().clone(),
                    PeerKind::Other,
                    PeerAvailability::Idle,
                    PeerCapabilities::new(true, true, true),
                )
            }));
        }
        if let Some(name) = request.display_name() {
            peers.retain(|peer| peer.display_name() == name);
        }
        peers.sort_by(|left, right| left.address().cmp(right.address()));
        Ok(PeerSnapshot::new(request.realm, peers, observed_at))
    }

    fn bind(&mut self, request: BindRequest) -> Result<PeerBinding, PeerFailure> {
        self.counts.bind += 1;
        if let Some(name) = request.requested_name.as_ref()
            && let Some(realm) = self.realms.get(&request.realm)
            && realm
                .bindings
                .values()
                .any(|binding| binding.public.effective_name() == name)
        {
            return Err(PeerFailure::NameConflict {
                realm: request.realm,
                name: request.requested_name.expect("checked above"),
            });
        }
        let binding_id: PeerBindingId = generated_id(b"binding", self.next_binding);
        let peer_id: PeerId = generated_id(b"circular", self.next_binding);
        let default_name = format!("circular-{}", self.next_binding);
        self.next_binding = self.next_binding.saturating_add(1);
        let effective_name = match request.requested_name {
            Some(name) => name,
            None => {
                PeerDisplayName::try_new(default_name).expect("generated peer names are normalized")
            }
        };
        let address = PeerAddress::new(self.name.clone(), request.realm.clone(), peer_id);
        let binding = PeerBinding::new(
            binding_id.clone(),
            request.actor,
            address,
            effective_name,
            BindingLease::Process,
            PeerBindingCapabilities::new(true, true),
        );
        self.realms
            .entry(request.realm)
            .or_default()
            .bindings
            .insert(
                binding_id,
                MemoryBinding {
                    public: binding.clone(),
                    inbound_policy: request.inbound_policy,
                    inbox_capacity: request.inbox_capacity,
                    pending: VecDeque::new(),
                },
            );
        Ok(binding)
    }

    fn send(&mut self, request: SendRequest) -> Result<SubmissionReceipt, PeerFailure> {
        self.counts.send += 1;
        self.validate_address(request.target())?;
        let binding_realm = self.find_binding_realm(request.binding())?;
        if &binding_realm != request.target().realm() {
            return Err(PeerFailure::WrongRealm {
                expected: binding_realm,
                actual: request.target().realm().clone(),
            });
        }
        if self.outbound_message_ids.contains(request.message()) {
            return Err(PeerFailure::DuplicateMessage(request.message));
        }
        let binding_id = request.binding.clone();
        let message_id = request.message.clone();
        let target = request.target.clone();

        let source = self
            .realms
            .get(&binding_realm)
            .and_then(|realm| realm.bindings.get(request.binding()))
            .expect("binding realm was resolved")
            .public
            .address()
            .clone();
        let realm = self
            .realms
            .get(&binding_realm)
            .expect("binding realm was resolved");
        let target_binding = realm
            .bindings
            .values()
            .find(|binding| binding.public.address() == request.target());
        if let Some(binding) = target_binding {
            binding.admit_inbound(&source)?;
        } else if !realm.external_peers.contains_key(request.target().peer()) {
            if self.retired_addresses.contains(request.target()) {
                return Err(PeerFailure::AddressStale(request.target));
            }
            return Err(PeerFailure::PeerNotFound(request.target));
        }
        let target_binding = target_binding.map(|binding| binding.public.id().clone());

        let accepted_at = self.next_stamp();
        let provider_id = ProviderMessageId::try_new(generated_bytes(b"provider", accepted_at.0))
            .expect("generated provider IDs are nonempty");
        let message = PeerMessage::new(
            request.message.clone(),
            Some(provider_id.clone()),
            source.clone(),
            request.target.clone(),
            Some(source),
            request.correlation,
            request.body,
            PeerProvenance::CircularActor,
            request.provider_fields,
        );
        if let Some(target_binding) = target_binding {
            let cursor = self.next_cursor();
            self.binding_mut(&binding_realm, &target_binding)?
                .pending
                .push_back(PeerEventEnvelope {
                    binding: target_binding,
                    cursor,
                    event: PeerEvent::Inbound(message),
                });
        } else {
            self.realms
                .get_mut(&binding_realm)
                .and_then(|realm| realm.external_inboxes.get_mut(target.peer()))
                .expect("target existence was checked")
                .push(message);
        }
        self.outbound_message_ids.insert(message_id.clone());

        let cursor = self.next_cursor();
        self.realms
            .get_mut(&binding_realm)
            .and_then(|realm| realm.bindings.get_mut(&binding_id))
            .expect("binding realm was resolved")
            .pending
            .push_back(PeerEventEnvelope {
                binding: binding_id,
                cursor,
                event: PeerEvent::DeliveryChanged {
                    message: message_id.clone(),
                    state: DeliveryDisposition::Delivered,
                },
            });
        if let Some(waker) = &self.receive_waker {
            waker.wake_by_ref();
        }
        Ok(SubmissionReceipt::try_new(
            message_id,
            Some(provider_id),
            DeliveryDisposition::Accepted,
            accepted_at,
        )
        .expect("Accepted is a valid submission disposition"))
    }

    fn receive(
        &mut self,
        binding: &PeerBindingId,
        after: Option<PeerCursor>,
    ) -> Result<PeerEventStream, PeerFailure> {
        self.counts.receive += 1;
        let realm = self.find_binding_realm(binding)?;
        let events = self
            .realms
            .get(&realm)
            .and_then(|realm| realm.bindings.get(binding))
            .expect("binding realm was resolved")
            .pending
            .iter()
            .filter(|event| after.is_none_or(|cursor| event.cursor() > cursor))
            .cloned()
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(PeerEventStream::new(events))
    }

    fn acknowledge_receive(
        &mut self,
        committed: DurablyCommittedPeerEvent,
    ) -> Result<AdmittedPeerEvent, PeerAcknowledgeError> {
        self.counts.acknowledge += 1;
        let binding_id = committed.binding().clone();
        let realm = match self.find_binding_realm(&binding_id) {
            Ok(realm) => realm,
            Err(PeerFailure::BindingStale(binding)) => {
                return Err(PeerAcknowledgeError {
                    committed: Box::new(committed),
                    reason: PeerAcknowledgeFailure::BindingStale(binding),
                });
            }
            Err(_) => {
                return Err(PeerAcknowledgeError {
                    committed: Box::new(committed),
                    reason: PeerAcknowledgeFailure::BindingNotFound(binding_id),
                });
            }
        };
        let state = self
            .realms
            .get_mut(&realm)
            .and_then(|realm| realm.bindings.get_mut(&binding_id))
            .expect("binding realm was resolved")
            .pending
            .front();
        let Some(expected) = state else {
            return Err(PeerAcknowledgeError {
                committed: Box::new(committed),
                reason: PeerAcknowledgeFailure::NoPendingEvent,
            });
        };
        if expected.cursor() != committed.cursor() {
            return Err(PeerAcknowledgeError {
                reason: PeerAcknowledgeFailure::OutOfOrder {
                    expected: expected.cursor(),
                    actual: committed.cursor(),
                },
                committed: Box::new(committed),
            });
        }
        if expected != &committed.envelope {
            return Err(PeerAcknowledgeError {
                committed: Box::new(committed),
                reason: PeerAcknowledgeFailure::EventMismatch,
            });
        }
        let envelope = self
            .realms
            .get_mut(&realm)
            .and_then(|realm| realm.bindings.get_mut(&binding_id))
            .expect("binding realm was resolved")
            .pending
            .pop_front()
            .expect("front event was just checked");
        Ok(AdmittedPeerEvent(envelope))
    }

    fn unbind(&mut self, binding: &PeerBindingId) -> Result<UnbindReceipt, PeerFailure> {
        self.counts.unbind += 1;
        let realm_id = self.find_binding_realm(binding)?;
        let pending_count = self
            .realms
            .get(&realm_id)
            .and_then(|realm| realm.bindings.get(binding))
            .expect("binding realm was resolved")
            .pending
            .len();
        if pending_count != 0 {
            return Err(PeerFailure::BindingHasPendingEvents {
                binding: binding.clone(),
                count: pending_count,
            });
        }
        let removed = self
            .realms
            .get_mut(&realm_id)
            .expect("binding realm was resolved")
            .bindings
            .remove(binding)
            .expect("binding realm was resolved");
        let closed_at = self.next_stamp();
        self.retired_bindings.insert(binding.clone());
        self.retired_addresses
            .insert(removed.public.address().clone());
        Ok(UnbindReceipt::new(
            binding.clone(),
            removed.public.address().clone(),
            closed_at,
        ))
    }
}

fn generated_bytes(prefix: &[u8], value: u64) -> Box<[u8]> {
    let mut bytes = Vec::with_capacity(prefix.len() + std::mem::size_of::<u64>());
    bytes.extend_from_slice(prefix);
    bytes.extend_from_slice(&value.to_be_bytes());
    bytes.into_boxed_slice()
}

fn generated_id<T>(prefix: &[u8], value: u64) -> T
where
    T: GeneratedPeerId,
{
    T::from_generated(generated_bytes(prefix, value))
}

trait GeneratedPeerId {
    fn from_generated(bytes: Box<[u8]>) -> Self;
}

macro_rules! generated_id_impl {
    ($($name:ident),+ $(,)?) => {
        $(
            impl GeneratedPeerId for $name {
                fn from_generated(bytes: Box<[u8]>) -> Self {
                    Self::try_new(bytes).expect("generated peer IDs are nonempty")
                }
            }
        )+
    };
}

generated_id_impl!(PeerId, PeerBindingId, PeerMessageId);
