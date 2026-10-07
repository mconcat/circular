use std::collections::BTreeSet;

use super::ids::*;
use super::ingress::*;
use super::protocol::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerAdapterCapabilities {
    discovery: PeerDiscoveryMode,
    advertisement: PeerAdvertisementMode,
    idle_wakeup: bool,
    active_turn_inject: bool,
    reply_address: PeerReplyAddressMode,
    delivery_status: BTreeSet<DeliveryDisposition>,
    receive_cursor: PeerReceiveCursorMode,
    provider_dedupe: bool,
    remote_realms: bool,
}

impl PeerAdapterCapabilities {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        discovery: PeerDiscoveryMode,
        advertisement: PeerAdvertisementMode,
        idle_wakeup: bool,
        active_turn_inject: bool,
        reply_address: PeerReplyAddressMode,
        delivery_status: impl IntoIterator<Item = DeliveryDisposition>,
        receive_cursor: PeerReceiveCursorMode,
        provider_dedupe: bool,
        remote_realms: bool,
    ) -> Self {
        Self {
            discovery,
            advertisement,
            idle_wakeup,
            active_turn_inject,
            reply_address,
            delivery_status: delivery_status.into_iter().collect(),
            receive_cursor,
            provider_dedupe,
            remote_realms,
        }
    }

    #[must_use]
    pub const fn discovery(&self) -> PeerDiscoveryMode {
        self.discovery
    }

    #[must_use]
    pub const fn advertisement(&self) -> PeerAdvertisementMode {
        self.advertisement
    }

    #[must_use]
    pub const fn idle_wakeup(&self) -> bool {
        self.idle_wakeup
    }

    #[must_use]
    pub const fn active_turn_inject(&self) -> bool {
        self.active_turn_inject
    }

    #[must_use]
    pub const fn reply_address(&self) -> PeerReplyAddressMode {
        self.reply_address
    }

    #[must_use]
    pub const fn delivery_status(&self) -> &BTreeSet<DeliveryDisposition> {
        &self.delivery_status
    }

    #[must_use]
    pub const fn receive_cursor(&self) -> PeerReceiveCursorMode {
        self.receive_cursor
    }

    #[must_use]
    pub const fn provider_dedupe(&self) -> bool {
        self.provider_dedupe
    }

    #[must_use]
    pub const fn remote_realms(&self) -> bool {
        self.remote_realms
    }
}

/// One provider adapter surface. Provider-specific registration and callback details stay behind it.
pub trait PeerAdapter {
    /// Wake the receive owner after admitting a provider envelope.
    fn set_receive_waker(&mut self, _waker: std::task::Waker) {}

    fn name(&self) -> &PeerAdapterName;

    fn capabilities(&self) -> &PeerAdapterCapabilities;

    fn discover(&mut self, request: DiscoverRequest) -> Result<PeerSnapshot, PeerFailure>;

    fn bind(&mut self, request: BindRequest) -> Result<PeerBinding, PeerFailure>;

    fn send(&mut self, request: SendRequest) -> Result<SubmissionReceipt, PeerFailure>;

    fn receive(
        &mut self,
        binding: &PeerBindingId,
        after: Option<PeerCursor>,
    ) -> Result<PeerEventStream, PeerFailure>;

    fn acknowledge_receive(
        &mut self,
        committed: DurablyCommittedPeerEvent,
    ) -> Result<AdmittedPeerEvent, PeerAcknowledgeError>;

    fn unbind(&mut self, binding: &PeerBindingId) -> Result<UnbindReceipt, PeerFailure>;
}
