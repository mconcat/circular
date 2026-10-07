use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;

use crate::capability::{
    Capability, CapabilitySet, Granted, PeerAdvertise, PeerDiscover, PeerReceive, PeerSend,
};
use crate::effect::EffectCtor;

use super::ids::*;
use super::peer::*;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerStamp(pub(super) u64);

impl PeerStamp {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerCursor(u64);

impl PeerCursor {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerDiscoveryMode {
    Snapshot,
    Streaming,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerAdvertisementMode {
    NativePeer,
    RelayPeer,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerReplyAddressMode {
    Native,
    Derived,
    None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerReceiveCursorMode {
    Native,
    AdapterOwned,
    None,
}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum DeliveryDisposition: u8 {
        Accepted = 1 => "accepted",
        Queued = 2 => "queued",
        Held = 3 => "held",
        Delivered = 4 => "delivered",
        Refused = 5 => "refused",
        Expired = 6 => "expired",
        Unreachable = 7 => "unreachable",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoverRequest {
    adapter: PeerAdapterName,
    pub(super) realm: PeerRealmId,
    display_name: Option<PeerDisplayName>,
}

impl DiscoverRequest {
    #[must_use]
    pub const fn new(
        adapter: PeerAdapterName,
        realm: PeerRealmId,
        display_name: Option<PeerDisplayName>,
    ) -> Self {
        Self {
            adapter,
            realm,
            display_name,
        }
    }

    #[must_use]
    pub const fn adapter(&self) -> &PeerAdapterName {
        &self.adapter
    }

    #[must_use]
    pub const fn realm(&self) -> &PeerRealmId {
        &self.realm
    }

    #[must_use]
    pub const fn display_name(&self) -> Option<&PeerDisplayName> {
        self.display_name.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerSnapshot {
    realm: PeerRealmId,
    peers: Box<[Peer]>,
    observed_at: PeerStamp,
}

impl PeerSnapshot {
    #[must_use]
    pub fn new(realm: PeerRealmId, peers: impl Into<Box<[Peer]>>, observed_at: PeerStamp) -> Self {
        Self {
            realm,
            peers: peers.into(),
            observed_at,
        }
    }

    #[must_use]
    pub const fn realm(&self) -> &PeerRealmId {
        &self.realm
    }

    #[must_use]
    pub const fn peers(&self) -> &[Peer] {
        &self.peers
    }

    #[must_use]
    pub const fn observed_at(&self) -> PeerStamp {
        self.observed_at
    }

    pub fn resolve_unique(
        &self,
        display_name: &PeerDisplayName,
    ) -> Result<&Peer, PeerResolveError> {
        let matches = self
            .peers
            .iter()
            .filter(|peer| peer.display_name() == display_name)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Err(PeerResolveError::NotFound(display_name.clone())),
            [peer] => Ok(peer),
            _ => Err(PeerResolveError::Ambiguous {
                display_name: display_name.clone(),
                candidates: matches
                    .into_iter()
                    .map(|peer| peer.address().clone())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerResolveError {
    NotFound(PeerDisplayName),
    Ambiguous {
        display_name: PeerDisplayName,
        candidates: Box<[PeerAddress]>,
    },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerActorIncarnation {
    actor: Box<[u8]>,
    incarnation: Box<[u8]>,
}

impl PeerActorIncarnation {
    pub fn try_new(
        actor: impl Into<Box<[u8]>>,
        incarnation: impl Into<Box<[u8]>>,
    ) -> Result<Self, EmptyPeerIdentifier> {
        let actor = actor.into();
        let incarnation = incarnation.into();
        if actor.is_empty() || incarnation.is_empty() {
            Err(EmptyPeerIdentifier)
        } else {
            Ok(Self { actor, incarnation })
        }
    }

    #[must_use]
    pub const fn actor(&self) -> &[u8] {
        &self.actor
    }

    #[must_use]
    pub const fn incarnation(&self) -> &[u8] {
        &self.incarnation
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InboundPolicy {
    AnyKnownPeer,
    Exact(BTreeSet<PeerAddress>),
}

impl InboundPolicy {
    #[must_use]
    pub fn allows(&self, address: &PeerAddress) -> bool {
        match self {
            Self::AnyKnownPeer => true,
            Self::Exact(addresses) => addresses.contains(address),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindRequest {
    pub(super) adapter: PeerAdapterName,
    pub(super) actor: PeerActorIncarnation,
    pub(super) realm: PeerRealmId,
    pub(super) requested_name: Option<PeerDisplayName>,
    pub(super) inbound_policy: InboundPolicy,
    pub(super) inbox_capacity: NonZeroUsize,
}

impl BindRequest {
    #[must_use]
    pub const fn new(
        adapter: PeerAdapterName,
        actor: PeerActorIncarnation,
        realm: PeerRealmId,
        requested_name: Option<PeerDisplayName>,
        inbound_policy: InboundPolicy,
        inbox_capacity: NonZeroUsize,
    ) -> Self {
        Self {
            adapter,
            actor,
            realm,
            requested_name,
            inbound_policy,
            inbox_capacity,
        }
    }

    #[must_use]
    pub const fn adapter(&self) -> &PeerAdapterName {
        &self.adapter
    }

    #[must_use]
    pub const fn actor(&self) -> &PeerActorIncarnation {
        &self.actor
    }

    #[must_use]
    pub const fn realm(&self) -> &PeerRealmId {
        &self.realm
    }

    #[must_use]
    pub const fn requested_name(&self) -> Option<&PeerDisplayName> {
        self.requested_name.as_ref()
    }

    #[must_use]
    pub const fn inbound_policy(&self) -> &InboundPolicy {
        &self.inbound_policy
    }

    #[must_use]
    pub const fn inbox_capacity(&self) -> NonZeroUsize {
        self.inbox_capacity
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingLease {
    Process,
    Until(PeerStamp),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerBindingCapabilities {
    receive_text: bool,
    provider_ack: bool,
}

impl PeerBindingCapabilities {
    #[must_use]
    pub const fn new(receive_text: bool, provider_ack: bool) -> Self {
        Self {
            receive_text,
            provider_ack,
        }
    }

    #[must_use]
    pub const fn receive_text(self) -> bool {
        self.receive_text
    }

    #[must_use]
    pub const fn provider_ack(self) -> bool {
        self.provider_ack
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerBinding {
    id: PeerBindingId,
    actor: PeerActorIncarnation,
    address: PeerAddress,
    effective_name: PeerDisplayName,
    lease: BindingLease,
    capabilities: PeerBindingCapabilities,
}

impl PeerBinding {
    #[must_use]
    pub const fn new(
        id: PeerBindingId,
        actor: PeerActorIncarnation,
        address: PeerAddress,
        effective_name: PeerDisplayName,
        lease: BindingLease,
        capabilities: PeerBindingCapabilities,
    ) -> Self {
        Self {
            id,
            actor,
            address,
            effective_name,
            lease,
            capabilities,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &PeerBindingId {
        &self.id
    }

    #[must_use]
    pub const fn actor(&self) -> &PeerActorIncarnation {
        &self.actor
    }

    #[must_use]
    pub const fn address(&self) -> &PeerAddress {
        &self.address
    }

    #[must_use]
    pub const fn effective_name(&self) -> &PeerDisplayName {
        &self.effective_name
    }

    #[must_use]
    pub const fn lease(&self) -> BindingLease {
        self.lease
    }

    #[must_use]
    pub const fn capabilities(&self) -> PeerBindingCapabilities {
        self.capabilities
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SendRequest {
    pub(super) binding: PeerBindingId,
    pub(super) message: PeerMessageId,
    pub(super) target: PeerAddress,
    pub(super) body: PeerBody,
    pub(super) correlation: Option<PeerMessageId>,
    pub(super) provider_fields: OpaqueProviderFields,
}

impl SendRequest {
    #[must_use]
    pub const fn new(
        binding: PeerBindingId,
        message: PeerMessageId,
        target: PeerAddress,
        body: PeerBody,
        correlation: Option<PeerMessageId>,
        provider_fields: OpaqueProviderFields,
    ) -> Self {
        Self {
            binding,
            message,
            target,
            body,
            correlation,
            provider_fields,
        }
    }

    #[must_use]
    pub const fn binding(&self) -> &PeerBindingId {
        &self.binding
    }

    #[must_use]
    pub const fn message(&self) -> &PeerMessageId {
        &self.message
    }

    #[must_use]
    pub const fn target(&self) -> &PeerAddress {
        &self.target
    }

    #[must_use]
    pub const fn body(&self) -> &PeerBody {
        &self.body
    }

    #[must_use]
    pub const fn correlation(&self) -> Option<&PeerMessageId> {
        self.correlation.as_ref()
    }

    #[must_use]
    pub const fn provider_fields(&self) -> &OpaqueProviderFields {
        &self.provider_fields
    }
}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum PeerProvenance: u8 {
        ExternalAgent = 1 => "external_agent",
        CircularActor = 2 => "circular_actor",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerMessage {
    id: PeerMessageId,
    provider_id: Option<ProviderMessageId>,
    from: PeerAddress,
    to: PeerAddress,
    reply_to: Option<PeerAddress>,
    correlation: Option<PeerMessageId>,
    body: PeerBody,
    provenance: PeerProvenance,
    provider_fields: OpaqueProviderFields,
}

impl PeerMessage {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        id: PeerMessageId,
        provider_id: Option<ProviderMessageId>,
        from: PeerAddress,
        to: PeerAddress,
        reply_to: Option<PeerAddress>,
        correlation: Option<PeerMessageId>,
        body: PeerBody,
        provenance: PeerProvenance,
        provider_fields: OpaqueProviderFields,
    ) -> Self {
        Self {
            id,
            provider_id,
            from,
            to,
            reply_to,
            correlation,
            body,
            provenance,
            provider_fields,
        }
    }

    /// Constructs only the external-agent provenance accepted by an inbound provider bridge.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn external(
        id: PeerMessageId,
        provider_id: Option<ProviderMessageId>,
        from: PeerAddress,
        to: PeerAddress,
        reply_to: Option<PeerAddress>,
        correlation: Option<PeerMessageId>,
        body: PeerBody,
        provider_fields: OpaqueProviderFields,
    ) -> Self {
        Self::new(
            id,
            provider_id,
            from,
            to,
            reply_to,
            correlation,
            body,
            PeerProvenance::ExternalAgent,
            provider_fields,
        )
    }

    #[must_use]
    pub const fn id(&self) -> &PeerMessageId {
        &self.id
    }

    #[must_use]
    pub const fn provider_id(&self) -> Option<&ProviderMessageId> {
        self.provider_id.as_ref()
    }

    #[must_use]
    pub const fn from(&self) -> &PeerAddress {
        &self.from
    }

    #[must_use]
    pub const fn to(&self) -> &PeerAddress {
        &self.to
    }

    #[must_use]
    pub const fn reply_to(&self) -> Option<&PeerAddress> {
        self.reply_to.as_ref()
    }

    #[must_use]
    pub const fn correlation(&self) -> Option<&PeerMessageId> {
        self.correlation.as_ref()
    }

    #[must_use]
    pub const fn body(&self) -> &PeerBody {
        &self.body
    }

    #[must_use]
    pub const fn provenance(&self) -> PeerProvenance {
        self.provenance
    }

    #[must_use]
    pub const fn provider_fields(&self) -> &OpaqueProviderFields {
        &self.provider_fields
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmissionReceipt {
    message: PeerMessageId,
    provider_id: Option<ProviderMessageId>,
    disposition: DeliveryDisposition,
    accepted_at: PeerStamp,
}

impl SubmissionReceipt {
    pub fn try_new(
        message: PeerMessageId,
        provider_id: Option<ProviderMessageId>,
        disposition: DeliveryDisposition,
        accepted_at: PeerStamp,
    ) -> Result<Self, SubmissionReceiptError> {
        if !matches!(
            disposition,
            DeliveryDisposition::Accepted | DeliveryDisposition::Queued | DeliveryDisposition::Held
        ) {
            return Err(SubmissionReceiptError::TerminalDisposition(disposition));
        }
        Ok(Self {
            message,
            provider_id,
            disposition,
            accepted_at,
        })
    }

    #[must_use]
    pub const fn message(&self) -> &PeerMessageId {
        &self.message
    }

    #[must_use]
    pub const fn provider_id(&self) -> Option<&ProviderMessageId> {
        self.provider_id.as_ref()
    }

    #[must_use]
    pub const fn disposition(&self) -> DeliveryDisposition {
        self.disposition
    }

    #[must_use]
    pub const fn accepted_at(&self) -> PeerStamp {
        self.accepted_at
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmissionReceiptError {
    TerminalDisposition(DeliveryDisposition),
}

impl fmt::Display for SubmissionReceiptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TerminalDisposition(disposition) => write!(
                formatter,
                "submission receipt cannot claim terminal delivery state {disposition:?}"
            ),
        }
    }
}

impl Error for SubmissionReceiptError {}

circular_core::closed_table! {
    pub enum PeerBindingState: u8 {
        Advertised = 1 => "advertised",
        Unhealthy = 2 => "unhealthy",
        Closed = 3 => "closed",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerDiagnostic {
    code: Box<str>,
    detail: Box<str>,
}

impl PeerDiagnostic {
    #[must_use]
    pub fn new(code: impl Into<Box<str>>, detail: impl Into<Box<str>>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
        }
    }

    #[must_use]
    pub const fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub const fn detail(&self) -> &str {
        &self.detail
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerEvent {
    Inbound(PeerMessage),
    PeerSnapshotChanged(PeerSnapshot),
    BindingStateChanged {
        binding: PeerBindingId,
        state: PeerBindingState,
    },
    DeliveryChanged {
        message: PeerMessageId,
        state: DeliveryDisposition,
    },
    AdapterDiagnostic(PeerDiagnostic),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerEventEnvelope {
    pub(super) binding: PeerBindingId,
    pub(super) cursor: PeerCursor,
    pub(super) event: PeerEvent,
}

impl PeerEventEnvelope {
    #[must_use]
    pub const fn new(binding: PeerBindingId, cursor: PeerCursor, event: PeerEvent) -> Self {
        Self {
            binding,
            cursor,
            event,
        }
    }

    #[must_use]
    pub const fn binding(&self) -> &PeerBindingId {
        &self.binding
    }

    #[must_use]
    pub const fn cursor(&self) -> PeerCursor {
        self.cursor
    }

    #[must_use]
    pub const fn event(&self) -> &PeerEvent {
        &self.event
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerEventStream {
    events: Box<[PeerEventEnvelope]>,
    next_cursor: Option<PeerCursor>,
}

impl PeerEventStream {
    #[must_use]
    pub fn new(events: impl Into<Box<[PeerEventEnvelope]>>) -> Self {
        let events = events.into();
        let next_cursor = events.last().map(PeerEventEnvelope::cursor);
        Self {
            events,
            next_cursor,
        }
    }

    #[must_use]
    pub const fn events(&self) -> &[PeerEventEnvelope] {
        &self.events
    }

    #[must_use]
    pub const fn next_cursor(&self) -> Option<PeerCursor> {
        self.next_cursor
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnbindReceipt {
    binding: PeerBindingId,
    address: PeerAddress,
    closed_at: PeerStamp,
}

impl UnbindReceipt {
    #[must_use]
    pub const fn new(binding: PeerBindingId, address: PeerAddress, closed_at: PeerStamp) -> Self {
        Self {
            binding,
            address,
            closed_at,
        }
    }

    #[must_use]
    pub const fn binding(&self) -> &PeerBindingId {
        &self.binding
    }

    #[must_use]
    pub const fn address(&self) -> &PeerAddress {
        &self.address
    }

    #[must_use]
    pub const fn closed_at(&self) -> PeerStamp {
        self.closed_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerFailure {
    AdapterUnavailable,
    WrongAdapter {
        expected: PeerAdapterName,
        actual: PeerAdapterName,
    },
    WrongRealm {
        expected: PeerRealmId,
        actual: PeerRealmId,
    },
    PeerNotFound(PeerAddress),
    BindingNotFound(PeerBindingId),
    BindingStale(PeerBindingId),
    AddressStale(PeerAddress),
    InboundRefused(PeerAddress),
    DuplicateMessage(PeerMessageId),
    InboxFull {
        binding: PeerBindingId,
        capacity: NonZeroUsize,
    },
    BindingHasPendingEvents {
        binding: PeerBindingId,
        count: usize,
    },
    UnsupportedCapability,
    SubmissionUnknown,
    NameConflict {
        realm: PeerRealmId,
        name: PeerDisplayName,
    },
}

circular_core::closed_table! {
    pub enum PeerFailureKind: u8 {
        AdapterUnavailable = 1 => "peer_adapter_unavailable",
        WrongAdapter = 2 => "peer_wrong_adapter",
        WrongRealm = 3 => "peer_wrong_realm",
        PeerNotFound = 4 => "peer_not_found",
        BindingNotFound = 5 => "peer_binding_not_found",
        BindingStale = 6 => "peer_binding_stale",
        AddressStale = 7 => "peer_address_stale",
        InboundRefused = 8 => "peer_inbound_refused",
        DuplicateMessage = 9 => "peer_duplicate_message",
        InboxFull = 10 => "peer_inbox_full",
        BindingHasPendingEvents = 11 => "peer_binding_has_pending_events",
        UnsupportedCapability = 12 => "peer_unsupported_capability",
        SubmissionUnknown = 13 => "peer_submission_unknown",
        NameConflict = 14 => "peer_name_conflict",
    }
}

impl PeerFailure {
    #[must_use]
    pub const fn kind(&self) -> PeerFailureKind {
        match self {
            Self::AdapterUnavailable => PeerFailureKind::AdapterUnavailable,
            Self::WrongAdapter { .. } => PeerFailureKind::WrongAdapter,
            Self::WrongRealm { .. } => PeerFailureKind::WrongRealm,
            Self::PeerNotFound(_) => PeerFailureKind::PeerNotFound,
            Self::BindingNotFound(_) => PeerFailureKind::BindingNotFound,
            Self::BindingStale(_) => PeerFailureKind::BindingStale,
            Self::AddressStale(_) => PeerFailureKind::AddressStale,
            Self::InboundRefused(_) => PeerFailureKind::InboundRefused,
            Self::DuplicateMessage(_) => PeerFailureKind::DuplicateMessage,
            Self::InboxFull { .. } => PeerFailureKind::InboxFull,
            Self::BindingHasPendingEvents { .. } => PeerFailureKind::BindingHasPendingEvents,
            Self::UnsupportedCapability => PeerFailureKind::UnsupportedCapability,
            Self::SubmissionUnknown => PeerFailureKind::SubmissionUnknown,
            Self::NameConflict { .. } => PeerFailureKind::NameConflict,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerEffect {
    Discover {
        grant: Granted<PeerDiscover>,
        request: DiscoverRequest,
    },
    Bind {
        advertise: Granted<PeerAdvertise>,
        receive: Granted<PeerReceive>,
        request: BindRequest,
    },
    Send {
        grant: Granted<PeerSend>,
        request: SendRequest,
    },
    Receive {
        grant: Granted<PeerReceive>,
        binding: PeerBindingId,
        after: Option<PeerCursor>,
    },
    Unbind {
        grant: Granted<PeerAdvertise>,
        binding: PeerBindingId,
    },
}

impl PeerEffect {
    #[must_use]
    pub const fn constructor(&self) -> EffectCtor {
        match self {
            Self::Discover { .. } => EffectCtor::PeerDiscover,
            Self::Bind { .. } => EffectCtor::PeerBind,
            Self::Send { .. } => EffectCtor::PeerSend,
            Self::Unbind { .. } => EffectCtor::PeerUnbind,
            Self::Receive { .. } => EffectCtor::PeerReceive,
        }
    }

    /// Unlike the legacy single-capability fold, bind preserves both advertisement and receive
    /// authority. No external message can manufacture either zero-sized proof.
    #[must_use]
    pub fn required_capabilities(&self) -> CapabilitySet {
        match self {
            Self::Discover { .. } => CapabilitySet::singleton(Capability::PeerDiscover),
            Self::Bind { .. } => CapabilitySet::singleton(Capability::PeerAdvertise)
                .join(&CapabilitySet::singleton(Capability::PeerReceive)),
            Self::Send { .. } => CapabilitySet::singleton(Capability::PeerSend),
            Self::Unbind { .. } => CapabilitySet::singleton(Capability::PeerAdvertise),
            Self::Receive { .. } => CapabilitySet::singleton(Capability::PeerReceive),
        }
    }
}

/// Capability evidence erased from the normalized, recordable peer request term.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerEffectTerm {
    Discover(DiscoverRequest),
    Bind(BindRequest),
    Send(SendRequest),
    Unbind(PeerBindingId),
    Receive {
        binding: PeerBindingId,
        after: Option<PeerCursor>,
    },
}

impl From<&PeerEffect> for PeerEffectTerm {
    fn from(effect: &PeerEffect) -> Self {
        match effect {
            PeerEffect::Discover { request, .. } => Self::Discover(request.clone()),
            PeerEffect::Bind { request, .. } => Self::Bind(request.clone()),
            PeerEffect::Send { request, .. } => Self::Send(request.clone()),
            PeerEffect::Unbind { binding, .. } => Self::Unbind(binding.clone()),
            PeerEffect::Receive { binding, after, .. } => Self::Receive {
                binding: binding.clone(),
                after: *after,
            },
        }
    }
}

/// Provider-side text send used by bridge implementations and the in-memory conformance adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalPeerSend {
    pub(super) provider_id: Option<ProviderMessageId>,
    pub(super) from: PeerAddress,
    pub(super) to: PeerAddress,
    pub(super) reply_to: Option<PeerAddress>,
    pub(super) correlation: Option<PeerMessageId>,
    pub(super) body: PeerBody,
    pub(super) provider_fields: OpaqueProviderFields,
}

impl ExternalPeerSend {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        provider_id: Option<ProviderMessageId>,
        from: PeerAddress,
        to: PeerAddress,
        reply_to: Option<PeerAddress>,
        correlation: Option<PeerMessageId>,
        body: PeerBody,
        provider_fields: OpaqueProviderFields,
    ) -> Self {
        Self {
            provider_id,
            from,
            to,
            reply_to,
            correlation,
            body,
            provider_fields,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderIngressReceipt {
    Accepted { cursor: PeerCursor },
    Duplicate { original_cursor: PeerCursor },
}
