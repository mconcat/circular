
use circular_actors::ProductPayload;
use circular_core::{ProducerIdentity, RecordedInstant, Stamp};
use circular_plan::{ActorId, EdgeId, NamedActorId, PortId};
use circular_runtime::ArrivalOrigin;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArrivalWitness<P: ProducerIdentity, C, T, D> {
    actor: NamedActorId,
    stamp: Stamp<P>,
    index: circular_core::ArrivalIndex,
    origin: ArrivalOrigin<P, C, T>,
    causal_parents: Box<[Stamp<P>]>,
    inlet: PortId,
    route_edge: Option<EdgeId>,
    payload: D,
    observed_at: RecordedInstant,
    result: circular_runtime::EnvelopeResult,
}

impl<P: ProducerIdentity, C, T, D> ArrivalWitness<P, C, T, D> {
    #[must_use]
    pub fn new(
        actor: NamedActorId,
        stamp: Stamp<P>,
        index: circular_core::ArrivalIndex,
        origin: ArrivalOrigin<P, C, T>,
        inlet: PortId,
        payload: D,
        observed_at: RecordedInstant,
    ) -> Self {
        let route_edge = match &origin {
            ArrivalOrigin::EdgeDelivery { edge, .. } => Some(edge.clone()),
            _ => None,
        };
        Self {
            result: circular_runtime::EnvelopeResult::Ok,
            route_edge,
            actor,
            stamp,
            index,
            origin,
            causal_parents: Box::new([]),
            inlet,
            payload,
            observed_at,
        }
    }

    pub fn with_result(mut self, result: circular_runtime::EnvelopeResult) -> Self {
        self.result = result;
        self
    }
    pub fn result(&self) -> &circular_runtime::EnvelopeResult {
        &self.result
    }

    pub fn with_route_edge(mut self, edge: Option<EdgeId>) -> Self {
        self.route_edge = edge;
        self
    }
    pub fn route_edge(&self) -> Option<&EdgeId> {
        self.route_edge.as_ref()
    }

    #[must_use]
    pub const fn actor(&self) -> &NamedActorId {
        &self.actor
    }

    #[must_use]
    pub const fn stamp(&self) -> &Stamp<P> {
        &self.stamp
    }

    #[must_use]
    pub const fn index(&self) -> circular_core::ArrivalIndex {
        self.index
    }

    /// Restore the exact causal parent column carried by a durable arrival.
    #[must_use]
    pub fn with_causal_parents(mut self, causal_parents: Box<[Stamp<P>]>) -> Self {
        self.causal_parents = causal_parents;
        self
    }

    #[must_use]
    pub const fn origin(&self) -> &ArrivalOrigin<P, C, T> {
        &self.origin
    }

    #[must_use]
    pub const fn causal_parents(&self) -> &[Stamp<P>] {
        &self.causal_parents
    }

    #[must_use]
    pub const fn inlet(&self) -> &PortId {
        &self.inlet
    }

    #[must_use]
    pub const fn edge(&self) -> Option<&EdgeId> {
        match &self.origin {
            ArrivalOrigin::EdgeDelivery { edge, .. } => Some(edge),
            ArrivalOrigin::TimerFire { .. }
            | ArrivalOrigin::EffectOutcome { .. }
            | ArrivalOrigin::ExternalInject { .. } => None,
        }
    }

    #[must_use]
    pub const fn sender(&self) -> Option<&Stamp<P>> {
        match &self.origin {
            ArrivalOrigin::EdgeDelivery { stamp, .. } => Some(stamp),
            ArrivalOrigin::TimerFire { .. }
            | ArrivalOrigin::EffectOutcome { .. }
            | ArrivalOrigin::ExternalInject { .. } => None,
        }
    }

    #[must_use]
    pub const fn payload(&self) -> &D {
        &self.payload
    }

    #[must_use]
    pub const fn observed_at(&self) -> RecordedInstant {
        self.observed_at
    }

    #[must_use]
    pub fn map_payload<E>(self, map: impl FnOnce(D) -> E) -> ArrivalWitness<P, C, T, E> {
        ArrivalWitness {
            actor: self.actor,
            stamp: self.stamp,
            index: self.index,
            origin: self.origin,
            causal_parents: self.causal_parents,
            inlet: self.inlet,
            route_edge: self.route_edge,
            payload: map(self.payload),
            observed_at: self.observed_at,
            result: self.result,
        }
    }
}

#[derive(Clone, Debug, Eq)]
pub struct RecordedPayload {
    shape: circular_core::GroundShape<circular_actors::Name>,
    bytes: circular_core::EncodedPayload,
}

impl PartialEq for RecordedPayload {
    fn eq(&self, other: &Self) -> bool {
        self.shape == other.shape && self.bytes.as_bytes() == other.bytes.as_bytes()
    }
}

impl RecordedPayload {
    #[must_use]
    pub const fn new(
        shape: circular_core::GroundShape<circular_actors::Name>,
        bytes: circular_core::EncodedPayload,
    ) -> Self {
        Self { shape, bytes }
    }

    #[must_use]
    pub const fn shape(&self) -> &circular_core::GroundShape<circular_actors::Name> {
        &self.shape
    }

    #[must_use]
    pub const fn bytes(&self) -> &circular_core::EncodedPayload {
        &self.bytes
    }

    pub fn decode(&self) -> Result<ProductPayload, String> {
        let value = circular_core::decode(
            self.bytes.body(),
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
        )
        .map_err(|error| format!("recorded arrival payload does not decode: {error:?}"))?;
        Ok(ProductPayload::new(self.shape.clone(), value))
    }

    pub fn encode(payload: &ProductPayload) -> Result<Self, String> {
        let body = circular_core::encode(
            payload.value(),
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
        )
        .map_err(|error| format!("arrival payload does not encode: {error:?}"))?;
        Ok(Self {
            shape: payload.shape().clone(),
            bytes: circular_core::EncodedPayload::new(
                circular_core::PayloadVersionTag::FIRST,
                &body,
            ),
        })
    }
}

pub type RecordedArrival = ArrivalWitness<
    ActorId,
    circular_runtime::EffectId,
    circular_runtime::EffectId,
    RecordedPayload,
>;

pub type RecordedColumns = BTreeMap<NamedActorId, Vec<RecordedArrival>>;
