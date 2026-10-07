
use crate::manifest::{ManifestSchema, RunManifest};
use circular_core::{ArrivalIndex, EncodedPayload, EventId, PortId, RecordedInstant, Stamp};
use std::fmt::Debug;

pub trait RecordRowCodec<S: StoreSchema>: Send + Sync {
    fn encode(&self, record: &Record<S>) -> Option<EncodedPayload>;
    fn decode(&self, bytes: &EncodedPayload) -> Option<Record<S>>;
}

#[derive(Clone, Debug)]
pub struct EncodedRow<S: StoreSchema> {
    record: Record<S>,
    bytes: Option<EncodedPayload>,
}

impl<S: StoreSchema> EncodedRow<S> {
    #[must_use]
    pub const fn new(record: Record<S>) -> Self {
        Self {
            record,
            bytes: None,
        }
    }

    #[must_use]
    pub fn decoded(codec: &dyn RecordRowCodec<S>, bytes: EncodedPayload) -> Option<Self> {
        let record = codec.decode(&bytes)?;
        Some(Self {
            record,
            bytes: Some(bytes),
        })
    }

    #[must_use]
    pub const fn record(&self) -> &Record<S> {
        &self.record
    }

    pub fn record_mut(&mut self) -> &mut Record<S> {
        self.bytes = None;
        &mut self.record
    }

    #[must_use]
    pub fn into_record(self) -> Record<S> {
        self.record
    }

    #[must_use]
    pub(crate) fn into_parts(self) -> (Record<S>, Option<EncodedPayload>) {
        (self.record, self.bytes)
    }
}

impl<S: StoreSchema> PartialEq for EncodedRow<S> {
    fn eq(&self, other: &Self) -> bool {
        self.record == other.record
    }
}

impl<S: StoreSchema> Eq for EncodedRow<S> {}

pub trait StoreSchema: ManifestSchema + 'static {
    fn row_codec() -> Option<&'static dyn RecordRowCodec<Self>>
    where
        Self: Sized,
    {
        None
    }

    type Incarnation: Clone + Debug + Eq;
    type ActorId: Clone + Debug + Eq + std::hash::Hash;
    type EdgeId: Clone + Debug + Eq + std::hash::Hash;
    type EffectId: Clone + Debug + Eq + std::hash::Hash;
    type TimerId: Clone + Debug + Eq + std::hash::Hash;
    type DisplayKey: Clone + Debug + Eq + std::hash::Hash;
    type ObservationKindKey: Clone + Debug + Eq + std::hash::Hash;
    /// Registration-owned identity projection for one Observation item.
    ///
    /// This is deliberately distinct from [`Self::ObservationKindKey`]: kind
    /// arguments select a query/subscription surface, while this value
    /// distinguishes items that share one stamp and kind. The Store compares
    /// both opaque values but interprets neither.
    type ObservationIdentityKey: Clone + Debug + Eq + std::hash::Hash;
    type ExternalOrigin: Clone + Debug + Eq + std::hash::Hash;
    type EffectTerm: Clone + Debug + Eq;
    type EffectOutcome: Clone + Debug + Eq;
    type GraphRevision: Clone + Debug + Eq;
    type DisplayPayload: Clone + Debug + Eq;
    type ObservationPayload: Clone + Debug + Eq;
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Class {
    Boundary,
    Structure,
    Display,
    Observation,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RecordOrigin<S: StoreSchema> {
    Actor(S::Incarnation),
    Stream,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationCoordinate {
    namespace: String,
    commit: u64,
    operation: u32,
    at: u64,
}

impl OperationCoordinate {
    #[must_use]
    pub const fn new(namespace: String, commit: u64, operation: u32, at: u64) -> Self {
        Self {
            namespace,
            commit,
            operation,
            at,
        }
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub const fn commit(&self) -> u64 {
        self.commit
    }

    #[must_use]
    pub const fn operation(&self) -> u32 {
        self.operation
    }

    #[must_use]
    pub const fn at(&self) -> u64 {
        self.at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArrivalBody<P: circular_core::ProducerIdentity> {
    Owned(EncodedPayload),
    Emitted {
        producer: P,
        sequence: circular_core::Sequence,
    },
}

impl<P: circular_core::ProducerIdentity> ArrivalBody<P> {
    #[must_use]
    pub fn emitted(sender: &Stamp<P>) -> Self {
        Self::Emitted {
            producer: sender.producer().clone(),
            sequence: sender.sequence(),
        }
    }

    #[must_use]
    pub const fn payload(&self) -> Option<&EncodedPayload> {
        match self {
            Self::Owned(payload) => Some(payload),
            Self::Emitted { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum RecordPosition<S: StoreSchema> {
    Stamped(Stamp<S::Producer>),
    OperationCoordinate(OperationCoordinate),
}

impl<S: StoreSchema> RecordPosition<S> {
    #[must_use]
    pub const fn stamp(&self) -> Option<&Stamp<S::Producer>> {
        match self {
            Self::Stamped(stamp) => Some(stamp),
            Self::OperationCoordinate(_) => None,
        }
    }

    #[must_use]
    pub const fn coordinate(&self) -> Option<&OperationCoordinate> {
        match self {
            Self::Stamped(_) => None,
            Self::OperationCoordinate(coordinate) => Some(coordinate),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArrivalOrigin<S: StoreSchema> {
    EdgeDelivery {
        edge: S::EdgeId,
        sender: Stamp<S::Producer>,
    },
    TimerFire { timer: S::TimerId },
    EffectOutcome {
        effect: S::EffectId,
        term: S::EffectTerm,
        outcome: S::EffectOutcome,
    },
    ExternalInject { origin: S::ExternalOrigin },
}

impl<S: StoreSchema> ArrivalOrigin<S> {
    #[must_use]
    pub const fn kind_tag(&self) -> u8 {
        crate::record_codec::arrival_origin_tag(self)
    }

    #[must_use]
    pub fn key(&self) -> ArrivalKey<S> {
        match self {
            Self::EdgeDelivery { edge, sender } => ArrivalKey::EdgeDelivery {
                edge: edge.clone(),
                sender: sender.clone(),
            },
            Self::TimerFire { timer } => ArrivalKey::TimerFire {
                timer: timer.clone(),
            },
            Self::EffectOutcome { effect, .. } => ArrivalKey::EffectOutcome {
                effect: effect.clone(),
            },
            Self::ExternalInject { origin } => ArrivalKey::ExternalInject {
                origin: origin.clone(),
                route_edge: None,
            },
        }
    }

    #[must_use]
    pub const fn is_external_nondeterministic(&self) -> bool {
        matches!(self, Self::ExternalInject { .. })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ArrivalKey<S: StoreSchema> {
    EdgeDelivery {
        edge: S::EdgeId,
        sender: Stamp<S::Producer>,
    },
    TimerFire {
        timer: S::TimerId,
    },
    EffectOutcome {
        effect: S::EffectId,
    },
    ExternalInject {
        origin: S::ExternalOrigin,
        route_edge: Option<S::EdgeId>,
    },
}

impl<S: StoreSchema> ArrivalKey<S> {
    #[must_use]
    pub const fn kind_tag(&self) -> u8 {
        crate::record_codec::arrival_key_tag(self)
    }

    #[must_use]
    pub const fn is_external_nondeterministic(&self) -> bool {
        matches!(self, Self::ExternalInject { .. })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BoundaryKey<S: StoreSchema> {
    EmissionBody {
        producer: S::Producer,
        sequence: circular_core::Sequence,
    },
    ScheduleReservation {
        effect: S::EffectId,
    },
    Arrival {
        actor: S::ActorId,
        origin: Box<ArrivalKey<S>>,
    },
    Admission {
        actor: S::ActorId,
        origin: Box<ArrivalKey<S>>,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum StructureKey<S: StoreSchema> {
    Manifest,
    Revision(S::ScopeId, Stamp<S::Producer>),
}

/// The part of an Observation key owned by its registration.
///
/// `kind` is the registered name plus its canonical query arguments.
/// `identity` is the separate canonical projection that distinguishes two
/// items at the same stamp and kind. Keeping the two fields distinct prevents
/// an item identity from silently becoming a query argument.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ObservationItemKey<S: StoreSchema> {
    kind: S::ObservationKindKey,
    identity: S::ObservationIdentityKey,
}

impl<S: StoreSchema> ObservationItemKey<S> {
    #[must_use]
    pub const fn new(kind: S::ObservationKindKey, identity: S::ObservationIdentityKey) -> Self {
        Self { kind, identity }
    }

    #[must_use]
    pub const fn kind(&self) -> &S::ObservationKindKey {
        &self.kind
    }

    #[must_use]
    pub const fn identity(&self) -> &S::ObservationIdentityKey {
        &self.identity
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObservationBucket(u64);

impl ObservationBucket {
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    #[must_use]
    pub const fn millis(self) -> u64 {
        self.0
    }
}

/// Complete class key for one immutable Observation item.
///
/// The class-key arm distinguishes stream observations from process observations.
/// The manifest owns stream identity; each key carries only its own coordinates.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ObservationKey<S: StoreSchema> {
    StreamItem(Stamp<S::Producer>, ObservationBucket, ObservationItemKey<S>),
    GlobalItem(Stamp<S::Producer>, ObservationBucket, ObservationItemKey<S>),
    CheckpointItem(
        OperationCoordinate,
        ObservationBucket,
        ObservationItemKey<S>,
    ),
}

impl<S: StoreSchema> ObservationKey<S> {
    #[must_use]
    pub const fn item(&self) -> &ObservationItemKey<S> {
        match self {
            Self::StreamItem(_, _, item)
            | Self::GlobalItem(_, _, item)
            | Self::CheckpointItem(_, _, item) => item,
        }
    }

    #[must_use]
    pub const fn bucket(&self) -> ObservationBucket {
        match self {
            Self::StreamItem(_, bucket, _)
            | Self::GlobalItem(_, bucket, _)
            | Self::CheckpointItem(_, bucket, _) => *bucket,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ClassKey<S: StoreSchema> {
    Boundary(BoundaryKey<S>),
    Structure(StructureKey<S>),
    Display {
        key: S::DisplayKey,
        at: Stamp<S::Producer>,
    },
    Observation(ObservationKey<S>),
}

impl<S: StoreSchema> ClassKey<S> {
    #[must_use]
    pub const fn class(&self) -> Class {
        match self {
            Self::Boundary(_) => Class::Boundary,
            Self::Structure(_) => Class::Structure,
            Self::Display { .. } => Class::Display,
            Self::Observation(_) => Class::Observation,
        }
    }

    pub const ARM_COUNT: usize = 9;

    #[must_use]
    pub const fn arm(&self) -> (Class, u8) {
        (self.class(), crate::record_codec::class_key_tag(self))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordHeader<S: StoreSchema> {
    class: Class,
    at: RecordPosition<S>,
    by: RecordOrigin<S>,
    key: ClassKey<S>,
}

impl<S: StoreSchema> RecordHeader<S> {
    fn new(at: Stamp<S::Producer>, by: RecordOrigin<S>, key: ClassKey<S>) -> Self {
        Self::positioned(RecordPosition::Stamped(at), by, key)
    }

    fn positioned(at: RecordPosition<S>, by: RecordOrigin<S>, key: ClassKey<S>) -> Self {
        Self {
            class: key.class(),
            at,
            by,
            key,
        }
    }

    #[must_use]
    pub const fn class(&self) -> Class {
        self.class
    }

    #[must_use]
    pub const fn position(&self) -> &RecordPosition<S> {
        &self.at
    }

    #[must_use]
    pub fn at(&self) -> &Stamp<S::Producer> {
        self.at
            .stamp()
            .expect("the append boundary refuses to record an operation coordinate")
    }

    #[must_use]
    pub const fn origin(&self) -> &RecordOrigin<S> {
        &self.by
    }

    #[must_use]
    pub const fn key(&self) -> &ClassKey<S> {
        &self.key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryFact<S: StoreSchema> {
    EmissionBody {
        port: PortId,
        cause: ArrivalIndex,
        observed_at: RecordedInstant,
        payload: EncodedPayload,
    },
    ScheduleReservation {
        payload: EncodedPayload,
    },
    Arrival {
        origin: Box<ArrivalOrigin<S>>,
        /// Actual receiving port; outcomes enter the reserved outcome edge instead.
        inlet: Option<PortId>,
        /// Actual route, independent of the idempotency key. Direct input has none.
        route_edge: Option<S::EdgeId>,
        result: circular_runtime::EnvelopeResult,
        body: ArrivalBody<S::Producer>,
        arrival_index: ArrivalIndex,
        causal_parents: Box<[EventId<S::Stream, S::Producer>]>,
        observed_at: RecordedInstant,
    },
    Admission {
        origin: Box<ArrivalOrigin<S>>,
        inlet: Option<PortId>,
        route_edge: Option<S::EdgeId>,
        result: circular_runtime::EnvelopeResult,
        body: ArrivalBody<S::Producer>,
        causal_parents: Box<[EventId<S::Stream, S::Producer>]>,
        observed_at: RecordedInstant,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmissionFact {
    pub port: PortId,
    pub cause: ArrivalIndex,
    pub observed_at: RecordedInstant,
    pub payload: EncodedPayload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryRecord<S: StoreSchema> {
    header: RecordHeader<S>,
    fact: BoundaryFact<S>,
}

impl<S: StoreSchema> BoundaryRecord<S> {
    #[must_use]
    pub fn emission_body(
        at: Stamp<S::Producer>,
        by: RecordOrigin<S>,
        emission: EmissionFact,
    ) -> Self {
        let key = BoundaryKey::EmissionBody {
            producer: at.producer().clone(),
            sequence: at.sequence(),
        };
        let EmissionFact {
            port,
            cause,
            observed_at,
            payload,
        } = emission;
        Self {
            header: RecordHeader::new(at, by, ClassKey::Boundary(key)),
            fact: BoundaryFact::EmissionBody {
                port,
                cause,
                observed_at,
                payload,
            },
        }
    }

    pub fn schedule_reservation(
        effect: S::EffectId,
        at: Stamp<S::Producer>,
        by: RecordOrigin<S>,
        payload: EncodedPayload,
    ) -> Self {
        Self {
            header: RecordHeader::new(
                at,
                by,
                ClassKey::Boundary(BoundaryKey::ScheduleReservation { effect }),
            ),
            fact: BoundaryFact::ScheduleReservation { payload },
        }
    }

    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn arrival(
        actor: S::ActorId,
        at: Stamp<S::Producer>,
        by: RecordOrigin<S>,
        origin: ArrivalOrigin<S>,
        body: ArrivalBody<S::Producer>,
        arrival_index: ArrivalIndex,
        causal_parents: Box<[EventId<S::Stream, S::Producer>]>,
        observed_at: RecordedInstant,
        inlet: Option<PortId>,
    ) -> Self {
        let route_edge = match &origin {
            ArrivalOrigin::EdgeDelivery { edge, .. } => Some(edge.clone()),
            _ => None,
        };
        let key_origin = Box::new(origin.key());
        let key = BoundaryKey::Arrival {
            actor,
            origin: key_origin,
        };
        Self {
            header: RecordHeader::new(at, by, ClassKey::Boundary(key)),
            fact: BoundaryFact::Arrival {
                result: circular_runtime::EnvelopeResult::Ok,
                origin: Box::new(origin),
                inlet,
                route_edge,
                body,
                arrival_index,
                causal_parents,
                observed_at,
            },
        }
    }

    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn admission(
        actor: S::ActorId,
        at: Stamp<S::Producer>,
        by: RecordOrigin<S>,
        origin: ArrivalOrigin<S>,
        body: ArrivalBody<S::Producer>,
        causal_parents: Box<[EventId<S::Stream, S::Producer>]>,
        observed_at: RecordedInstant,
        inlet: Option<PortId>,
    ) -> Self {
        let route_edge = match &origin {
            ArrivalOrigin::EdgeDelivery { edge, .. } => Some(edge.clone()),
            _ => None,
        };
        let key = BoundaryKey::Admission {
            actor,
            origin: Box::new(origin.key()),
        };
        Self {
            header: RecordHeader::new(at, by, ClassKey::Boundary(key)),
            fact: BoundaryFact::Admission {
                result: circular_runtime::EnvelopeResult::Ok,
                origin: Box::new(origin),
                inlet,
                route_edge,
                body,
                causal_parents,
                observed_at,
            },
        }
    }

    pub fn with_result(mut self, result: circular_runtime::EnvelopeResult) -> Self {
        let (BoundaryFact::Arrival {
            result: recorded, ..
        }
        | BoundaryFact::Admission {
            result: recorded, ..
        }) = &mut self.fact
        else {
            panic!("result belongs to an arrival envelope");
        };
        *recorded = result;
        self
    }

    pub(crate) fn share_arrival_payload(&mut self, shared: EncodedPayload) {
        let (BoundaryFact::Arrival { body, .. } | BoundaryFact::Admission { body, .. }) =
            &mut self.fact
        else {
            unreachable!("arrival payload sharing")
        };
        let ArrivalBody::Owned(payload) = body else {
            unreachable!("only an owned arrival body shares its allocation")
        };
        assert_eq!(payload, &shared, "sharing cannot change a recorded byte");
        *payload = shared;
    }

    #[must_use]
    pub const fn arrival_body(&self) -> Option<&ArrivalBody<S::Producer>> {
        match &self.fact {
            BoundaryFact::Arrival { body, .. } | BoundaryFact::Admission { body, .. } => Some(body),
            _ => None,
        }
    }

    pub fn resolve_arrival_body(&mut self, payload: EncodedPayload) -> bool {
        let (BoundaryFact::Arrival { body, .. } | BoundaryFact::Admission { body, .. }) =
            &mut self.fact
        else {
            panic!("arrival bodies belong to receiving arrivals and admissions");
        };
        if matches!(body, ArrivalBody::Owned(_)) {
            return false;
        }
        *body = ArrivalBody::Owned(payload);
        true
    }

    #[must_use]
    pub fn with_route_edge(mut self, edge: Option<S::EdgeId>) -> Self {
        let (BoundaryFact::Arrival { route_edge, .. } | BoundaryFact::Admission { route_edge, .. }) =
            &mut self.fact
        else {
            panic!("route edges belong to receiving arrivals");
        };
        *route_edge = edge.clone();
        let ClassKey::Boundary(
            BoundaryKey::Arrival { origin, .. } | BoundaryKey::Admission { origin, .. },
        ) = &mut self.header.key
        else {
            unreachable!("boundary arrival header");
        };
        if let ArrivalKey::ExternalInject { route_edge, .. } = origin.as_mut() {
            *route_edge = edge;
        }
        self
    }

    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn effect_outcome_arrival(
        actor: S::ActorId,
        at: Stamp<S::Producer>,
        by: RecordOrigin<S>,
        arrival_index: ArrivalIndex,
        effect: S::EffectId,
        term: S::EffectTerm,
        outcome: S::EffectOutcome,
        causal_parents: Box<[EventId<S::Stream, S::Producer>]>,
        observed_at: RecordedInstant,
    ) -> Self {
        Self::arrival(
            actor,
            at,
            by,
            ArrivalOrigin::EffectOutcome {
                effect,
                term,
                outcome,
            },
            ArrivalBody::Owned(EncodedPayload::new(
                circular_core::PayloadVersionTag::FIRST,
                &[],
            )),
            arrival_index,
            causal_parents,
            observed_at,
            None,
        )
    }

    #[must_use]
    pub const fn header(&self) -> &RecordHeader<S> {
        &self.header
    }

    #[must_use]
    pub const fn fact(&self) -> &BoundaryFact<S> {
        &self.fact
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StructureFact<S: StoreSchema> {
    RunManifest(RunManifest<S>),
    GraphRevision(S::GraphRevision),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StructureRecord<S: StoreSchema> {
    header: RecordHeader<S>,
    fact: StructureFact<S>,
}

impl<S: StoreSchema> StructureRecord<S> {
    #[must_use]
    pub fn manifest(at: Stamp<S::Producer>, manifest: RunManifest<S>) -> Self {
        Self {
            header: RecordHeader::new(
                at,
                RecordOrigin::Stream,
                ClassKey::Structure(StructureKey::Manifest),
            ),
            fact: StructureFact::RunManifest(manifest),
        }
    }

    #[must_use]
    pub fn graph_revision(
        scope: S::ScopeId,
        at: Stamp<S::Producer>,
        revision: S::GraphRevision,
    ) -> Self {
        Self {
            header: RecordHeader::new(
                at.clone(),
                RecordOrigin::Stream,
                ClassKey::Structure(StructureKey::Revision(scope, at)),
            ),
            fact: StructureFact::GraphRevision(revision),
        }
    }

    #[must_use]
    pub const fn header(&self) -> &RecordHeader<S> {
        &self.header
    }

    #[must_use]
    pub const fn fact(&self) -> &StructureFact<S> {
        &self.fact
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayRecord<S: StoreSchema> {
    header: RecordHeader<S>,
    payload: S::DisplayPayload,
}

impl<S: StoreSchema> DisplayRecord<S> {
    #[must_use]
    pub fn new(
        at: Stamp<S::Producer>,
        by: RecordOrigin<S>,
        key: S::DisplayKey,
        payload: S::DisplayPayload,
    ) -> Self {
        Self {
            header: RecordHeader::new(at.clone(), by, ClassKey::Display { key, at }),
            payload,
        }
    }

    #[must_use]
    pub const fn header(&self) -> &RecordHeader<S> {
        &self.header
    }

    #[must_use]
    pub const fn payload(&self) -> &S::DisplayPayload {
        &self.payload
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ObservationFact<S: StoreSchema> {
    Lifecycle(S::ObservationPayload),
    Diagnostic(S::ObservationPayload),
    Accounting(S::ObservationPayload),
    DeadLetter(S::ObservationPayload),
    ReplaySessionTransition(S::ObservationPayload),
    Restart(S::ObservationPayload),
    Checkpoint(S::ObservationPayload),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationRecord<S: StoreSchema> {
    header: RecordHeader<S>,
    fact: ObservationFact<S>,
}

impl<S: StoreSchema> ObservationRecord<S> {
    fn for_stream(
        at: Stamp<S::Producer>,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        fact: ObservationFact<S>,
    ) -> Self {
        let key = ObservationKey::StreamItem(at.clone(), bucket, item);
        Self {
            header: RecordHeader::new(at, by, ClassKey::Observation(key)),
            fact,
        }
    }

    fn global(
        at: Stamp<S::Producer>,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        fact: ObservationFact<S>,
    ) -> Self {
        Self {
            header: RecordHeader::new(
                at.clone(),
                by,
                ClassKey::Observation(ObservationKey::GlobalItem(at, bucket, item)),
            ),
            fact,
        }
    }

    #[must_use]
    pub fn checkpoint(
        at: OperationCoordinate,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        payload: S::ObservationPayload,
    ) -> Self {
        let key = ObservationKey::CheckpointItem(at.clone(), bucket, item);
        Self {
            header: RecordHeader::positioned(
                RecordPosition::OperationCoordinate(at),
                by,
                ClassKey::Observation(key),
            ),
            fact: ObservationFact::Checkpoint(payload),
        }
    }

    /// An ordinary stream observation joining the pre-boot and post-boot sequence.
    /// It does not delimit a replay interval.
    #[must_use]
    pub fn restart(
        at: Stamp<S::Producer>,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        payload: S::ObservationPayload,
    ) -> Self {
        Self::for_stream(at, bucket, by, item, ObservationFact::Restart(payload))
    }

    #[must_use]
    pub fn lifecycle(
        at: Stamp<S::Producer>,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        payload: S::ObservationPayload,
    ) -> Self {
        Self::for_stream(at, bucket, by, item, ObservationFact::Lifecycle(payload))
    }

    #[must_use]
    pub fn diagnostic(
        at: Stamp<S::Producer>,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        payload: S::ObservationPayload,
    ) -> Self {
        Self::for_stream(at, bucket, by, item, ObservationFact::Diagnostic(payload))
    }

    #[must_use]
    pub fn accounting(
        at: Stamp<S::Producer>,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        payload: S::ObservationPayload,
    ) -> Self {
        Self::for_stream(at, bucket, by, item, ObservationFact::Accounting(payload))
    }

    #[must_use]
    pub fn dead_letter(
        at: Stamp<S::Producer>,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        payload: S::ObservationPayload,
    ) -> Self {
        Self::for_stream(at, bucket, by, item, ObservationFact::DeadLetter(payload))
    }

    #[must_use]
    pub fn global_accounting(
        at: Stamp<S::Producer>,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        payload: S::ObservationPayload,
    ) -> Self {
        Self::global(at, bucket, by, item, ObservationFact::Accounting(payload))
    }

    #[must_use]
    pub fn replay_session_transition(
        at: Stamp<S::Producer>,
        bucket: ObservationBucket,
        by: RecordOrigin<S>,
        item: ObservationItemKey<S>,
        payload: S::ObservationPayload,
    ) -> Self {
        Self::global(
            at,
            bucket,
            by,
            item,
            ObservationFact::ReplaySessionTransition(payload),
        )
    }

    #[must_use]
    pub const fn header(&self) -> &RecordHeader<S> {
        &self.header
    }

    #[must_use]
    pub const fn fact(&self) -> &ObservationFact<S> {
        &self.fact
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Record<S: StoreSchema> {
    Boundary(BoundaryRecord<S>),
    Structure(StructureRecord<S>),
    Display(DisplayRecord<S>),
    Observation(ObservationRecord<S>),
}

impl<S: StoreSchema> Record<S> {
    #[must_use]
    pub const fn header(&self) -> &RecordHeader<S> {
        match self {
            Self::Boundary(record) => record.header(),
            Self::Structure(record) => record.header(),
            Self::Display(record) => record.header(),
            Self::Observation(record) => record.header(),
        }
    }
}
