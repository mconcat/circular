use super::ids::*;
use super::protocol::*;

/// Exact `(binding, cursor)` receipt for the peer actor's durably recorded outcome arrival.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerOutcomeArrivalReceipt {
    binding: PeerBindingId,
    cursor: PeerCursor,
}

impl PeerOutcomeArrivalReceipt {
    #[must_use]
    pub const fn new(binding: PeerBindingId, cursor: PeerCursor) -> Self {
        Self { binding, cursor }
    }

    #[must_use]
    pub const fn binding(&self) -> &PeerBindingId {
        &self.binding
    }

    #[must_use]
    pub const fn cursor(&self) -> PeerCursor {
        self.cursor
    }
}

/// Supplies the receipt for an envelope's recorded outcome arrival. The actor kernel
/// calls this after recording that arrival; it does not append a second copy.
pub trait PeerOutcomeArrivalStore {
    type Error;

    fn outcome_arrival_receipt(
        &mut self,
        event: &PeerEventEnvelope,
    ) -> Result<PeerOutcomeArrivalReceipt, Self::Error>;
}

/// Marker for a Store port whose receipt proves a crash-durable outcome arrival.
pub trait CrashDurablePeerOutcomeArrival: PeerOutcomeArrivalStore {}

#[derive(Debug)]
pub enum PeerIngressCommitError<E> {
    Store(E),
    ReceiptMismatch {
        expected_binding: PeerBindingId,
        expected_cursor: PeerCursor,
        receipt: PeerOutcomeArrivalReceipt,
    },
}

/// Proof that one exact envelope was durably recorded as the peer actor's outcome arrival.
/// Only the adapter acknowledgement path can turn this into a graph-admissible event.
#[derive(Debug)]
pub struct DurablyCommittedPeerEvent {
    pub(super) envelope: PeerEventEnvelope,
    receipt: PeerOutcomeArrivalReceipt,
}

impl DurablyCommittedPeerEvent {
    /// The exact envelope whose recorded outcome arrival produced this proof. An adapter
    /// outside this module needs it to compare the acknowledgement against the
    /// event it is still holding for the provider.
    #[must_use]
    pub const fn envelope(&self) -> &PeerEventEnvelope {
        &self.envelope
    }

    #[must_use]
    pub const fn binding(&self) -> &PeerBindingId {
        self.envelope.binding()
    }

    #[must_use]
    pub const fn cursor(&self) -> PeerCursor {
        self.envelope.cursor()
    }

    #[must_use]
    pub const fn receipt(&self) -> &PeerOutcomeArrivalReceipt {
        &self.receipt
    }
}

pub fn commit_peer_event<S>(
    store: &mut S,
    envelope: PeerEventEnvelope,
) -> Result<DurablyCommittedPeerEvent, PeerIngressCommitError<S::Error>>
where
    S: CrashDurablePeerOutcomeArrival + ?Sized,
{
    let receipt = store
        .outcome_arrival_receipt(&envelope)
        .map_err(PeerIngressCommitError::Store)?;
    if receipt.binding() != envelope.binding() || receipt.cursor() != envelope.cursor() {
        return Err(PeerIngressCommitError::ReceiptMismatch {
            expected_binding: envelope.binding().clone(),
            expected_cursor: envelope.cursor(),
            receipt,
        });
    }
    Ok(DurablyCommittedPeerEvent { envelope, receipt })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedPeerEvent(pub(super) PeerEventEnvelope);

impl AdmittedPeerEvent {
    /// Admit the exact envelope whose durable outcome arrival is already proven. This is
    /// the only constructor available to a provider adapter outside this module,
    /// so admission cannot be minted from an envelope that was never committed.
    #[must_use]
    pub fn admit(committed: DurablyCommittedPeerEvent) -> Self {
        Self(committed.envelope)
    }

    #[must_use]
    pub const fn envelope(&self) -> &PeerEventEnvelope {
        &self.0
    }

    #[must_use]
    pub fn into_envelope(self) -> PeerEventEnvelope {
        self.0
    }
}

#[derive(Debug)]
pub struct PeerAcknowledgeError {
    pub(super) committed: Box<DurablyCommittedPeerEvent>,
    pub(super) reason: PeerAcknowledgeFailure,
}

impl PeerAcknowledgeError {
    /// Return the committed evidence with the reason it was not admitted. The
    /// caller keeps the proof, so a later attempt acknowledges the same event.
    #[must_use]
    pub fn refused(committed: DurablyCommittedPeerEvent, reason: PeerAcknowledgeFailure) -> Self {
        Self {
            committed: Box::new(committed),
            reason,
        }
    }

    #[must_use]
    pub const fn reason(&self) -> &PeerAcknowledgeFailure {
        &self.reason
    }

    #[must_use]
    pub fn into_committed(self) -> DurablyCommittedPeerEvent {
        *self.committed
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerAcknowledgeFailure {
    BindingNotFound(PeerBindingId),
    BindingStale(PeerBindingId),
    NoPendingEvent,
    OutOfOrder {
        expected: PeerCursor,
        actual: PeerCursor,
    },
    EventMismatch,
}

/// Provider-free replay carrier. It cannot contact an adapter by construction.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PeerReplayTape(Box<[AdmittedPeerEvent]>);

impl PeerReplayTape {
    #[must_use]
    pub fn new(events: impl Into<Box<[AdmittedPeerEvent]>>) -> Self {
        Self(events.into())
    }

    pub fn replay(&self) -> impl ExactSizeIterator<Item = &PeerEventEnvelope> {
        self.0.iter().map(AdmittedPeerEvent::envelope)
    }
}
