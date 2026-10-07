//! Versioned bridge boundaries for provider-native peer messaging.
//!
//! These traits name the provider operations the bridge must implement without
//! claiming that undocumented transport details are stable public APIs. In
//! particular, the Codex surface is independent-thread messaging; collab child
//! orchestration has no representation here.

use std::error::Error;
use std::fmt;

use crate::{
    AdmittedPeerEvent, BindRequest, DiscoverRequest, DurablyCommittedPeerEvent, InboundPolicy,
    PeerAcknowledgeError, PeerAdapter, PeerAdapterCapabilities, PeerAdapterName, PeerBinding,
    PeerBindingId, PeerCursor, PeerEvent, PeerEventStream, PeerFailure, PeerSnapshot, SendRequest,
    SubmissionReceipt, UnbindReceipt,
};

/// Version of the local bridge/plugin protocol used to reach one provider harness.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerBridgeContractVersion(Box<str>);

impl PeerBridgeContractVersion {
    pub fn try_new(value: impl Into<Box<str>>) -> Result<Self, InvalidBridgeContractVersion> {
        let value = value.into();
        if value.is_empty() {
            return Err(InvalidBridgeContractVersion::Empty);
        }
        if value.trim() != value.as_ref() {
            return Err(InvalidBridgeContractVersion::SurroundingWhitespace);
        }
        if value.chars().any(char::is_control) {
            return Err(InvalidBridgeContractVersion::ContainsControl);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidBridgeContractVersion {
    Empty,
    SurroundingWhitespace,
    ContainsControl,
}

impl fmt::Display for InvalidBridgeContractVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "peer bridge contract version must not be empty",
            Self::SurroundingWhitespace => {
                "peer bridge contract version must not have surrounding whitespace"
            }
            Self::ContainsControl => {
                "peer bridge contract version must not contain control characters"
            }
        })
    }
}

impl Error for InvalidBridgeContractVersion {}

/// Shared metadata every provider bridge must publish before registration.
pub trait VersionedPeerBridge {
    fn contract_version(&self) -> &PeerBridgeContractVersion;

    fn adapter_name(&self) -> &PeerAdapterName;

    fn capabilities(&self) -> &PeerAdapterCapabilities;
}

pub trait ProviderPeerBridge: VersionedPeerBridge {
    fn set_receive_waker(&mut self, _waker: std::task::Waker) {}

    fn discover(&mut self, request: DiscoverRequest) -> Result<PeerSnapshot, PeerFailure>;

    fn bind(&mut self, request: BindRequest) -> Result<PeerBinding, PeerFailure>;

    fn send(&mut self, request: SendRequest) -> Result<SubmissionReceipt, PeerFailure>;

    fn receive(
        &mut self,
        binding: &PeerBindingId,
        after: Option<PeerCursor>,
    ) -> Result<PeerEventStream, PeerFailure>;

    fn acknowledge(
        &mut self,
        committed: DurablyCommittedPeerEvent,
    ) -> Result<AdmittedPeerEvent, PeerAcknowledgeError>;

    fn unbind(&mut self, binding: &PeerBindingId) -> Result<UnbindReceipt, PeerFailure>;
}

pub struct BridgePeerAdapter<B> {
    bridge: B,
    inbound_policies: std::collections::BTreeMap<PeerBindingId, InboundPolicy>,
}

impl<B: ProviderPeerBridge> BridgePeerAdapter<B> {
    pub fn new(bridge: B) -> Self {
        Self {
            bridge,
            inbound_policies: std::collections::BTreeMap::new(),
        }
    }

    fn admits(&self, envelope: &crate::PeerEventEnvelope) -> Result<(), PeerFailure> {
        let PeerEvent::Inbound(message) = envelope.event() else {
            return Ok(());
        };
        let Some(policy) = self.inbound_policies.get(envelope.binding()) else {
            return Err(PeerFailure::BindingNotFound(envelope.binding().clone()));
        };
        if policy.allows(message.from()) {
            Ok(())
        } else {
            Err(PeerFailure::InboundRefused(message.from().clone()))
        }
    }

    #[must_use]
    pub fn contract_version(&self) -> &PeerBridgeContractVersion {
        self.bridge.contract_version()
    }

    #[must_use]
    pub const fn bridge(&self) -> &B {
        &self.bridge
    }

    #[must_use]
    pub const fn bridge_mut(&mut self) -> &mut B {
        &mut self.bridge
    }
}

impl<B: ProviderPeerBridge> PeerAdapter for BridgePeerAdapter<B> {
    fn set_receive_waker(&mut self, waker: std::task::Waker) {
        self.bridge.set_receive_waker(waker);
    }
    fn name(&self) -> &PeerAdapterName {
        self.bridge.adapter_name()
    }

    fn capabilities(&self) -> &PeerAdapterCapabilities {
        self.bridge.capabilities()
    }

    fn discover(&mut self, request: DiscoverRequest) -> Result<PeerSnapshot, PeerFailure> {
        self.bridge.discover(request)
    }

    fn bind(&mut self, request: BindRequest) -> Result<PeerBinding, PeerFailure> {
        let policy = request.inbound_policy().clone();
        let binding = self.bridge.bind(request)?;
        self.inbound_policies.insert(binding.id().clone(), policy);
        Ok(binding)
    }

    fn send(&mut self, request: SendRequest) -> Result<SubmissionReceipt, PeerFailure> {
        self.bridge.send(request)
    }

    fn receive(
        &mut self,
        binding: &PeerBindingId,
        after: Option<PeerCursor>,
    ) -> Result<PeerEventStream, PeerFailure> {
        let stream = self.bridge.receive(binding, after)?;
        if let Some(envelope) = stream.events().first() {
            self.admits(envelope)?;
        }
        Ok(stream)
    }

    fn acknowledge_receive(
        &mut self,
        committed: DurablyCommittedPeerEvent,
    ) -> Result<AdmittedPeerEvent, PeerAcknowledgeError> {
        self.bridge.acknowledge(committed)
    }

    fn unbind(&mut self, binding: &PeerBindingId) -> Result<UnbindReceipt, PeerFailure> {
        let receipt = self.bridge.unbind(binding)?;
        self.inbound_policies.remove(binding);
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        BindingLease, CrashDurablePeerOutcomeArrival, DeliveryDisposition, ExternalPeerSend,
        InboundPolicy, MemoryPeerAdapter, OpaqueProviderFields, Peer, PeerActorIncarnation,
        PeerAvailability, PeerBindingCapabilities, PeerBody, PeerCapabilities, PeerDisplayName,
        PeerEvent, PeerFailureKind, PeerKind, PeerMessageId, PeerOutcomeArrivalReceipt,
        PeerOutcomeArrivalStore, PeerRealmId, PeerStamp, ProviderMessageId, commit_peer_event,
    };
    use std::convert::Infallible;
    use std::num::NonZeroUsize;

    struct RecordingBridge {
        version: PeerBridgeContractVersion,
        inner: MemoryPeerAdapter,
        calls: Vec<&'static str>,
    }

    impl RecordingBridge {
        fn new(adapter: &str) -> Self {
            Self {
                version: PeerBridgeContractVersion::try_new("bridge-v1").unwrap(),
                inner: MemoryPeerAdapter::new(PeerAdapterName::try_new(adapter).unwrap()),
                calls: Vec::new(),
            }
        }

        fn push(&mut self, call: &'static str) {
            self.calls.push(call);
        }
    }

    impl VersionedPeerBridge for RecordingBridge {
        fn contract_version(&self) -> &PeerBridgeContractVersion {
            &self.version
        }

        fn adapter_name(&self) -> &PeerAdapterName {
            self.inner.name()
        }

        fn capabilities(&self) -> &PeerAdapterCapabilities {
            self.inner.capabilities()
        }
    }

    impl ProviderPeerBridge for RecordingBridge {
        fn discover(&mut self, request: DiscoverRequest) -> Result<PeerSnapshot, PeerFailure> {
            self.push("discover");
            self.inner.discover(request)
        }

        fn bind(&mut self, request: BindRequest) -> Result<PeerBinding, PeerFailure> {
            self.push("bind");
            self.inner.bind(request)
        }

        fn send(&mut self, request: SendRequest) -> Result<SubmissionReceipt, PeerFailure> {
            self.push("send");
            self.inner.send(request)
        }

        fn receive(
            &mut self,
            binding: &PeerBindingId,
            after: Option<PeerCursor>,
        ) -> Result<PeerEventStream, PeerFailure> {
            self.push("receive");
            self.inner.receive(binding, after)
        }

        fn acknowledge(
            &mut self,
            committed: DurablyCommittedPeerEvent,
        ) -> Result<AdmittedPeerEvent, PeerAcknowledgeError> {
            self.push("acknowledge");
            self.inner.acknowledge_receive(committed)
        }

        fn unbind(&mut self, binding: &PeerBindingId) -> Result<UnbindReceipt, PeerFailure> {
            self.push("unbind");
            self.inner.unbind(binding)
        }
    }

    #[derive(Default)]
    struct DurableOutcomeArrival;

    impl PeerOutcomeArrivalStore for DurableOutcomeArrival {
        type Error = Infallible;

        fn outcome_arrival_receipt(
            &mut self,
            event: &crate::PeerEventEnvelope,
        ) -> Result<PeerOutcomeArrivalReceipt, Self::Error> {
            Ok(PeerOutcomeArrivalReceipt::new(
                event.binding().clone(),
                event.cursor(),
            ))
        }
    }

    impl CrashDurablePeerOutcomeArrival for DurableOutcomeArrival {}

    fn display(value: &str) -> PeerDisplayName {
        PeerDisplayName::try_new(value).unwrap()
    }

    fn exercise(adapter: &mut BridgePeerAdapter<RecordingBridge>, expected_calls: &[&str]) {
        let realm = PeerRealmId::try_new(&b"realm"[..]).unwrap();
        let external = adapter.bridge_mut().inner.register_external_peer(
            realm.clone(),
            display("external"),
            PeerKind::Thread,
            PeerAvailability::Idle,
            PeerCapabilities::new(true, true, true),
        );

        let peers = adapter
            .discover(DiscoverRequest::new(
                adapter.name().clone(),
                realm.clone(),
                None,
            ))
            .unwrap();
        assert_eq!(peers.peers(), std::slice::from_ref(&external));
        let binding = adapter
            .bind(BindRequest::new(
                adapter.name().clone(),
                PeerActorIncarnation::try_new(&b"actor"[..], &b"incarnation"[..]).unwrap(),
                realm,
                Some(display("Circular")),
                InboundPolicy::AnyKnownPeer,
                NonZeroUsize::new(4).unwrap(),
            ))
            .unwrap();
        assert_eq!(binding.lease(), BindingLease::Process);
        assert_eq!(
            binding.capabilities(),
            PeerBindingCapabilities::new(true, true)
        );

        let outbound = PeerMessageId::try_new(&b"outbound"[..]).unwrap();
        let receipt = adapter
            .send(SendRequest::new(
                binding.id().clone(),
                outbound.clone(),
                external.address().clone(),
                PeerBody::try_new("hello").unwrap(),
                None,
                OpaqueProviderFields::empty(),
            ))
            .unwrap();
        assert_eq!(receipt.disposition(), DeliveryDisposition::Accepted);
        assert_eq!(receipt.message(), &outbound);

        adapter
            .bridge_mut()
            .inner
            .inject_external(ExternalPeerSend::new(
                Some(ProviderMessageId::try_new(&b"provider-inbound"[..]).unwrap()),
                external.address().clone(),
                binding.address().clone(),
                Some(external.address().clone()),
                Some(outbound),
                PeerBody::try_new("reply").unwrap(),
                OpaqueProviderFields::empty(),
            ))
            .unwrap();

        let events = adapter.receive(binding.id(), None).unwrap();
        assert_eq!(events.events().len(), 2);
        assert!(matches!(
            events.events()[0].event(),
            PeerEvent::DeliveryChanged { .. }
        ));
        assert!(matches!(events.events()[1].event(), PeerEvent::Inbound(_)));
        let mut outcome_arrival = DurableOutcomeArrival;
        for event in events.events() {
            let committed = commit_peer_event(&mut outcome_arrival, event.clone()).unwrap();
            adapter.acknowledge_receive(committed).unwrap();
        }
        let unbound = adapter.unbind(binding.id()).unwrap();
        assert_eq!(unbound.binding(), binding.id());
        assert_eq!(adapter.bridge_mut().calls, expected_calls);
    }

    const SHARED_CONTRACT_CALLS: &[&str] = &[
        "discover",
        "bind",
        "send",
        "receive",
        "acknowledge",
        "acknowledge",
        "unbind",
    ];

    const BRIDGED: &str = "bridged";

    #[test]
    fn a_bridge_drives_the_shared_contract() {
        let mut adapter = BridgePeerAdapter::new(RecordingBridge::new(BRIDGED));
        assert_eq!(adapter.contract_version().as_str(), "bridge-v1");
        exercise(&mut adapter, SHARED_CONTRACT_CALLS);
    }

    struct PermissiveBridge {
        version: PeerBridgeContractVersion,
        inner: MemoryPeerAdapter,
    }

    impl VersionedPeerBridge for PermissiveBridge {
        fn contract_version(&self) -> &PeerBridgeContractVersion {
            &self.version
        }
        fn adapter_name(&self) -> &PeerAdapterName {
            self.inner.name()
        }
        fn capabilities(&self) -> &PeerAdapterCapabilities {
            self.inner.capabilities()
        }
    }

    impl ProviderPeerBridge for PermissiveBridge {
        fn discover(&mut self, request: DiscoverRequest) -> Result<PeerSnapshot, PeerFailure> {
            self.inner.discover(request)
        }
        fn bind(&mut self, request: BindRequest) -> Result<PeerBinding, PeerFailure> {
            self.inner.bind(BindRequest::new(
                request.adapter().clone(),
                request.actor().clone(),
                request.realm().clone(),
                request.requested_name().cloned(),
                InboundPolicy::AnyKnownPeer,
                request.inbox_capacity(),
            ))
        }
        fn send(&mut self, request: SendRequest) -> Result<SubmissionReceipt, PeerFailure> {
            self.inner.send(request)
        }
        fn receive(
            &mut self,
            binding: &PeerBindingId,
            after: Option<PeerCursor>,
        ) -> Result<PeerEventStream, PeerFailure> {
            self.inner.receive(binding, after)
        }
        fn acknowledge(
            &mut self,
            committed: DurablyCommittedPeerEvent,
        ) -> Result<AdmittedPeerEvent, PeerAcknowledgeError> {
            self.inner.acknowledge_receive(committed)
        }
        fn unbind(&mut self, binding: &PeerBindingId) -> Result<UnbindReceipt, PeerFailure> {
            self.inner.unbind(binding)
        }
    }

    fn permissive_adapter(provider: &str) -> (BridgePeerAdapter<PermissiveBridge>, PeerRealmId) {
        let realm = PeerRealmId::try_new(&b"realm"[..]).unwrap();
        let adapter = BridgePeerAdapter::new(PermissiveBridge {
            version: PeerBridgeContractVersion::try_new("bridge-v1").unwrap(),
            inner: MemoryPeerAdapter::new(PeerAdapterName::try_new(provider).unwrap()),
        });
        (adapter, realm)
    }

    fn external(
        adapter: &mut BridgePeerAdapter<PermissiveBridge>,
        realm: &PeerRealmId,
        name: &str,
    ) -> Peer {
        adapter.bridge_mut().inner.register_external_peer(
            realm.clone(),
            display(name),
            PeerKind::Session,
            PeerAvailability::Idle,
            PeerCapabilities::new(true, true, true),
        )
    }

    fn deliver(
        adapter: &mut BridgePeerAdapter<PermissiveBridge>,
        from: &Peer,
        to: &PeerBinding,
        body: &str,
    ) {
        adapter
            .bridge_mut()
            .inner
            .inject_external(ExternalPeerSend::new(
                None,
                from.address().clone(),
                to.address().clone(),
                None,
                None,
                PeerBody::try_new(body).unwrap(),
                OpaqueProviderFields::empty(),
            ))
            .unwrap();
    }

    struct ScriptedInboundBridge {
        version: PeerBridgeContractVersion,
        name: PeerAdapterName,
        capabilities: PeerAdapterCapabilities,
        envelope: crate::PeerEventEnvelope,
    }

    impl VersionedPeerBridge for ScriptedInboundBridge {
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

    impl ProviderPeerBridge for ScriptedInboundBridge {
        fn discover(&mut self, _request: DiscoverRequest) -> Result<PeerSnapshot, PeerFailure> {
            Err(PeerFailure::UnsupportedCapability)
        }
        fn bind(&mut self, _request: BindRequest) -> Result<PeerBinding, PeerFailure> {
            Err(PeerFailure::UnsupportedCapability)
        }
        fn send(&mut self, _request: SendRequest) -> Result<SubmissionReceipt, PeerFailure> {
            Err(PeerFailure::UnsupportedCapability)
        }
        fn receive(
            &mut self,
            _binding: &PeerBindingId,
            _after: Option<PeerCursor>,
        ) -> Result<PeerEventStream, PeerFailure> {
            Ok(PeerEventStream::new(vec![self.envelope.clone()]))
        }
        fn acknowledge(
            &mut self,
            _committed: DurablyCommittedPeerEvent,
        ) -> Result<AdmittedPeerEvent, PeerAcknowledgeError> {
            panic!("a refused envelope never reaches acknowledgement")
        }
        fn unbind(&mut self, _binding: &PeerBindingId) -> Result<UnbindReceipt, PeerFailure> {
            Err(PeerFailure::UnsupportedCapability)
        }
    }

    #[test]
    fn an_inbound_envelope_without_a_remembered_policy_is_not_admitted() {
        let name = PeerAdapterName::try_new(BRIDGED).unwrap();
        let realm = PeerRealmId::try_new(&b"realm"[..]).unwrap();
        let mut memory = MemoryPeerAdapter::new(name.clone());
        let sender = memory.register_external_peer(
            realm.clone(),
            display("sender"),
            PeerKind::Session,
            PeerAvailability::Idle,
            PeerCapabilities::new(true, true, true),
        );
        let binding = memory
            .bind(BindRequest::new(
                name.clone(),
                PeerActorIncarnation::try_new(&b"actor"[..], &b"incarnation"[..]).unwrap(),
                realm,
                Some(display("Circular")),
                InboundPolicy::AnyKnownPeer,
                NonZeroUsize::new(4).unwrap(),
            ))
            .unwrap();
        memory
            .inject_external(ExternalPeerSend::new(
                None,
                sender.address().clone(),
                binding.address().clone(),
                None,
                None,
                PeerBody::try_new("hello").unwrap(),
                OpaqueProviderFields::empty(),
            ))
            .unwrap();
        let envelope = memory.receive(binding.id(), None).unwrap().events()[0].clone();
        let capabilities = memory.capabilities().clone();
        let mut adapter = BridgePeerAdapter::new(ScriptedInboundBridge {
            version: PeerBridgeContractVersion::try_new("bridge-v1").unwrap(),
            name,
            capabilities,
            envelope,
        });
        assert_eq!(
            adapter.receive(binding.id(), None).unwrap_err(),
            PeerFailure::BindingNotFound(binding.id().clone())
        );
    }

    #[test]
    fn contract_versions_reject_ambiguous_labels() {
        assert_eq!(
            PeerBridgeContractVersion::try_new(""),
            Err(InvalidBridgeContractVersion::Empty)
        );
        assert_eq!(
            PeerBridgeContractVersion::try_new(" v1"),
            Err(InvalidBridgeContractVersion::SurroundingWhitespace)
        );
    }

    #[test]
    fn submission_receipt_fixture_never_claims_delivery() {
        let receipt = SubmissionReceipt::try_new(
            PeerMessageId::try_new(&b"message"[..]).unwrap(),
            None,
            DeliveryDisposition::Accepted,
            PeerStamp::new(1),
        )
        .unwrap();
        assert_ne!(receipt.disposition(), DeliveryDisposition::Delivered);
    }
}
