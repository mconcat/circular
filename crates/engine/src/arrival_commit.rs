//! Delivery admission boundary between durable recording and an actor mailbox.
//!
//! The receiving actor supplies its own stamp and `ArrivalIndex`; only a successful commit
//! authorizes mailbox delivery. Exact retries return the first index and
//! are marked [`ArrivalCommitDisposition::Folded`]. The mailbox mutates only for `Recorded` or a
//! crash-window `Recovered` retry.

use circular_core::{ArrivalIndex, PortId, RecordedInstant, Stamp};
use circular_plan::{ActorId, NamedActorId};
use circular_runtime::{ArrivalOrigin, EffectFailure, EffectTerm, OutcomePayload};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EventArrivalCommit {
    actor: NamedActorId,
    inlet: PortId,
    route_edge: Option<circular_plan::EdgeId>,
    expected_horizon: ArrivalIndex,
    at: Stamp<ActorId>,
    origin: ArrivalOrigin<ActorId, circular_runtime::EffectId, circular_runtime::EffectId>,
    causal_parents: Box<[Stamp<ActorId>]>,
    payload: circular_actors::ProductPayload,
    body: Result<circular_store::ArrivalBody<ActorId>, ArrivalCommitError>,
    encoded: Result<circular_core::EncodedPayload, ArrivalCommitError>,
    observed_at: RecordedInstant,
    result: circular_runtime::EnvelopeResult,
}

impl EventArrivalCommit {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        actor: NamedActorId,
        inlet: PortId,
        expected_horizon: ArrivalIndex,
        at: Stamp<ActorId>,
        origin: ArrivalOrigin<ActorId, circular_runtime::EffectId, circular_runtime::EffectId>,
        causal_parents: Box<[Stamp<ActorId>]>,
        payload: circular_actors::ProductPayload,
        observed_at: RecordedInstant,
    ) -> Self {
        let encoded = crate::product_arrival_journal::encode_event_payload(&payload);
        Self::new_encoded(
            actor,
            inlet,
            expected_horizon,
            at,
            origin,
            causal_parents,
            payload,
            observed_at,
            encoded,
        )
    }

    /// Wire delivery carries the producer's encoding through to live publication.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_encoded(
        actor: NamedActorId,
        inlet: PortId,
        expected_horizon: ArrivalIndex,
        at: Stamp<ActorId>,
        origin: ArrivalOrigin<ActorId, circular_runtime::EffectId, circular_runtime::EffectId>,
        causal_parents: Box<[Stamp<ActorId>]>,
        payload: circular_actors::ProductPayload,
        observed_at: RecordedInstant,
        encoded: Result<circular_core::EncodedPayload, ArrivalCommitError>,
    ) -> Self {
        let route_edge = match &origin {
            ArrivalOrigin::EdgeDelivery { edge, .. } => Some(edge.clone()),
            _ => None,
        };
        let body = encoded.clone().map(circular_store::ArrivalBody::Owned);
        Self {
            body,
            encoded,
            result: circular_runtime::EnvelopeResult::Ok,
            actor,
            inlet,
            route_edge,
            expected_horizon,
            at,
            origin,
            causal_parents,
            payload,
            observed_at,
        }
    }

    /// The durable reference and the live payload have separate ownership.
    pub(crate) fn with_body(mut self, body: circular_store::ArrivalBody<ActorId>) -> Self {
        self.body = Ok(body);
        self
    }

    pub(crate) fn body(&self) -> Result<&circular_store::ArrivalBody<ActorId>, ArrivalCommitError> {
        self.body.as_ref().map_err(Clone::clone)
    }

    pub(crate) fn encoded(&self) -> Result<&circular_core::EncodedPayload, ArrivalCommitError> {
        self.encoded.as_ref().map_err(Clone::clone)
    }

    pub(crate) fn with_result(mut self, result: circular_runtime::EnvelopeResult) -> Self {
        self.result = result;
        self
    }
    pub(crate) fn result(&self) -> &circular_runtime::EnvelopeResult {
        &self.result
    }

    pub(crate) fn with_route_edge(mut self, edge: Option<circular_plan::EdgeId>) -> Self {
        self.route_edge = edge;
        self
    }
    pub(crate) fn route_edge(&self) -> Option<&circular_plan::EdgeId> {
        self.route_edge.as_ref()
    }

    pub(crate) const fn actor(&self) -> &NamedActorId {
        &self.actor
    }

    pub(crate) const fn inlet(&self) -> &PortId {
        &self.inlet
    }

    pub(crate) const fn expected_horizon(&self) -> ArrivalIndex {
        self.expected_horizon
    }

    pub(crate) const fn at(&self) -> &Stamp<ActorId> {
        &self.at
    }

    pub(crate) const fn origin(
        &self,
    ) -> &ArrivalOrigin<ActorId, circular_runtime::EffectId, circular_runtime::EffectId> {
        &self.origin
    }

    pub(crate) const fn causal_parents(&self) -> &[Stamp<ActorId>] {
        &self.causal_parents
    }

    pub(crate) const fn payload(&self) -> &circular_actors::ProductPayload {
        &self.payload
    }

    pub(crate) const fn observed_at(&self) -> RecordedInstant {
        self.observed_at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EffectArrivalCommit {
    actor: NamedActorId,
    expected_horizon: ArrivalIndex,
    at: Stamp<ActorId>,
    effect: circular_runtime::EffectId,
    cause: Stamp<ActorId>,
    term: Option<EffectTerm>,
    result: Result<OutcomePayload, EffectFailure>,
    failure_progress: Vec<circular_runtime::AgentProgressRecord>,
    observed_at: RecordedInstant,
}

impl EffectArrivalCommit {
    #[allow(clippy::too_many_arguments)]
    pub(crate) const fn new(
        actor: NamedActorId,
        expected_horizon: ArrivalIndex,
        at: Stamp<ActorId>,
        effect: circular_runtime::EffectId,
        cause: Stamp<ActorId>,
        term: Option<EffectTerm>,
        result: Result<OutcomePayload, EffectFailure>,
        observed_at: RecordedInstant,
    ) -> Self {
        Self {
            actor,
            expected_horizon,
            at,
            effect,
            cause,
            term,
            result,
            failure_progress: Vec::new(),
            observed_at,
        }
    }

    #[must_use]
    pub(crate) fn with_failure_progress(
        mut self,
        progress: &[circular_runtime::AgentProgressRecord],
    ) -> Self {
        self.failure_progress = progress.to_vec();
        self
    }

    pub(crate) fn failure_progress(&self) -> &[circular_runtime::AgentProgressRecord] {
        &self.failure_progress
    }

    pub(crate) const fn actor(&self) -> &NamedActorId {
        &self.actor
    }

    pub(crate) const fn expected_horizon(&self) -> ArrivalIndex {
        self.expected_horizon
    }

    pub(crate) const fn at(&self) -> &Stamp<ActorId> {
        &self.at
    }

    pub(crate) fn effect(&self) -> circular_runtime::EffectId {
        self.effect.clone()
    }

    pub(crate) const fn cause(&self) -> &Stamp<ActorId> {
        &self.cause
    }

    pub(crate) const fn term(&self) -> Option<&EffectTerm> {
        self.term.as_ref()
    }

    pub(crate) const fn result(&self) -> &Result<OutcomePayload, EffectFailure> {
        &self.result
    }

    pub(crate) const fn observed_at(&self) -> RecordedInstant {
        self.observed_at
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArrivalCommitDisposition {
    Recorded,
    Recovered,
    Folded,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ArrivalCommitReceipt {
    index: ArrivalIndex,
    disposition: ArrivalCommitDisposition,
    at: Stamp<ActorId>,
    observed_at: RecordedInstant,
}

impl ArrivalCommitReceipt {
    pub(crate) fn recorded_event(
        index: ArrivalIndex,
        at: Stamp<ActorId>,
        observed_at: RecordedInstant,
    ) -> Self {
        Self::folded_event(index, ArrivalCommitDisposition::Recorded, at, observed_at)
    }

    pub(crate) fn folded_event(
        index: ArrivalIndex,
        disposition: ArrivalCommitDisposition,
        at: Stamp<ActorId>,
        observed_at: RecordedInstant,
    ) -> Self {
        Self {
            index,
            disposition,
            at,
            observed_at,
        }
    }

    pub(crate) const fn needs_mailbox(&self) -> bool {
        matches!(
            self.disposition,
            ArrivalCommitDisposition::Recorded | ArrivalCommitDisposition::Recovered
        )
    }

    pub(crate) const fn index(&self) -> ArrivalIndex {
        self.index
    }

    #[cfg(test)]
    pub(crate) const fn disposition(&self) -> ArrivalCommitDisposition {
        self.disposition
    }

    pub(crate) const fn observed_at(&self) -> RecordedInstant {
        self.observed_at
    }

    pub(crate) const fn at(&self) -> &Stamp<ActorId> {
        &self.at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArrivalCommitError {
    FoldedHorizonMismatch {
        actor: NamedActorId,
        recorded: ArrivalIndex,
        mailbox: ArrivalIndex,
    },
    UnexpectedReceipt,
    Rejected { operation: usize },
    Codec,
    RehydratedProjectionFailed,
    RecordEncoding,
    RecordDecoding,
    ProjectionRejected { operation: usize },
    MissingBoundaryArrival,
    WrongRecordConstructor,
    CoordinatorGone,
    BackendAmbiguous,
    RecordKeyExhausted,
    IdentityCarriedDifferentFacts { actor: NamedActorId },
}

impl std::fmt::Display for ArrivalCommitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FoldedHorizonMismatch {
                actor,
                recorded,
                mailbox,
            } => write!(
                formatter,
                "folded arrival horizon mismatch for {actor:?}: recorded={}, mailbox={}",
                recorded.get(),
                mailbox.get()
            ),
            Self::UnexpectedReceipt => {
                formatter.write_str("arrival commit returned a receipt of the wrong kind")
            }
            Self::Rejected { operation } => {
                write!(
                    formatter,
                    "arrival transaction rejected at operation {operation}"
                )
            }
            Self::Codec => formatter.write_str("arrival transaction encoding rejected"),
            Self::RehydratedProjectionFailed => formatter
                .write_str("durable record was omitted from the rehydration query projection"),
            Self::RecordEncoding => {
                formatter.write_str("arrival record encoding did not produce canonical form")
            }
            Self::RecordDecoding => {
                formatter.write_str("failed to decode stored arrival transaction")
            }
            Self::ProjectionRejected { operation } => {
                write!(
                    formatter,
                    "arrival projection rejected operation {operation}"
                )
            }
            Self::MissingBoundaryArrival => {
                formatter.write_str("stored record has no Boundary arrival")
            }
            Self::WrongRecordConstructor => {
                formatter.write_str("effect outcomes require the dedicated record constructor")
            }
            Self::CoordinatorGone => formatter
                .write_str("arrival commit coordinator disappeared; commit status is unknown"),
            Self::BackendAmbiguous => {
                formatter.write_str("arrival group commit has an ambiguous backend outcome")
            }
            Self::RecordKeyExhausted => formatter.write_str("journal record key space exhausted"),
            Self::IdentityCarriedDifferentFacts { actor } => write!(
                formatter,
                "the same arrival identity carried different facts at {actor:?}"
            ),
        }
    }
}

impl std::error::Error for ArrivalCommitError {}
