
use crate::manifest::ManifestSchema;
use crate::record::StoreSchema;
use crate::transaction::TransactionSchema;
use circular_core::{EncodedPayload, ProducerIdentity, StreamIdentity};
use circular_core::{BuiltinObservationName, NonZeroMillis};
use circular_runtime::{ActorId, EdgeId, NamedActorId, ScopeId};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StreamId(u64);

impl StreamId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for StreamId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "stream:{}", self.0)
    }
}

impl StreamIdentity for StreamId {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IncarnationId(u64);

impl IncarnationId {
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
pub struct OpaqueId(u64);

impl OpaqueId {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OperationId {
    producer: ActorId,
    sequence: circular_core::Sequence,
}

impl OperationId {
    #[must_use]
    pub const fn new(producer: ActorId, sequence: circular_core::Sequence) -> Self {
        Self { producer, sequence }
    }

    #[must_use]
    pub const fn producer(&self) -> &ActorId {
        &self.producer
    }

    #[must_use]
    pub const fn sequence(&self) -> circular_core::Sequence {
        self.sequence
    }
}

impl circular_core::OperationIdentity for OperationId {}

impl circular_core::ProducerLocalOperation<ActorId> for OperationId {
    fn issue(producer: &ActorId, sequence: circular_core::Sequence) -> Self {
        Self::new(producer.clone(), sequence)
    }
}

pub type ProductAuthoringCut = crate::manifest::AuthoringCut<
    [u8; circular_protocol::SESSION_TOKEN_BYTES],
    u64,
    circular_protocol::declaration_payload::AuthoringEnvironment,
    circular_protocol::RevisionDigest<circular_protocol::AuthoringRevision>,
    circular_protocol::RevisionDigest<circular_protocol::TopologyRevision>,
>;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProductStore;

impl ManifestSchema for ProductStore {
    type Stream = StreamId;
    type Producer = ActorId;
    type Placement = EncodedPayload;
    type Failure = EncodedPayload;
    type Versions = EncodedPayload;
    type RevisionId = OpaqueId;
    type AuthoringCut = ProductAuthoringCut;
    type ScopeId = ScopeId;
    type GrantSet = EncodedPayload;
    type InputValue = EncodedPayload;

    type CadencePolicy = EncodedPayload;
    type TickOrigin = OpaqueId;
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DisplayKey {
    name: BuiltinObservationName,
    args: NamedActorId,
    bucket: Option<NonZeroMillis>,
}

impl DisplayKey {
    #[must_use]
    pub const fn new(
        name: BuiltinObservationName,
        args: NamedActorId,
        bucket: Option<NonZeroMillis>,
    ) -> Self {
        Self { name, args, bucket }
    }

    #[must_use]
    pub const fn name(&self) -> BuiltinObservationName {
        self.name
    }

    #[must_use]
    pub const fn args(&self) -> &NamedActorId {
        &self.args
    }

    #[must_use]
    pub const fn bucket(&self) -> Option<NonZeroMillis> {
        self.bucket
    }
}

static PRODUCT_ROW_CODEC: crate::product_journal::ProductRowCodec =
    crate::product_journal::ProductRowCodec;

impl StoreSchema for ProductStore {
    fn row_codec() -> Option<&'static dyn crate::record::RecordRowCodec<Self>> {
        Some(&PRODUCT_ROW_CODEC)
    }

    type Incarnation = IncarnationId;
    type ActorId = ActorId;
    type EdgeId = EdgeId;
    type EffectId = circular_runtime::EffectId;
    type TimerId = circular_runtime::EffectId;
    type DisplayKey = DisplayKey;
    type ObservationKindKey = BuiltinObservationName;
    type ObservationIdentityKey = OpaqueId;
    type ExternalOrigin = EncodedPayload;
    type EffectTerm = EncodedPayload;
    type EffectOutcome = EncodedPayload;
    type GraphRevision = EncodedPayload;
    type DisplayPayload = EncodedPayload;
    type ObservationPayload = EncodedPayload;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductTransaction;

impl TransactionSchema for ProductTransaction {
    type WriteContext = crate::ColumnWriteContext;
    fn record_digest(record: &Self::Record) -> crate::transaction::RecordDigest {
        crate::transaction::RecordDigest::of_bytes(record.as_bytes())
    }

    fn observation_digest(observation: &Self::Observation) -> crate::transaction::RecordDigest {
        crate::transaction::RecordDigest::of_bytes(observation.as_bytes())
    }

    type RecordKey = OpaqueId;
    type Record = EncodedPayload;
    type EffectId = circular_runtime::EffectId;
    type Outbox = EncodedPayload;
    type ApprovalKey = circular_runtime::EffectId;
    type Approval = EncodedPayload;
    type ApprovalTicket = circular_runtime::EffectId;
    type ActorId = ActorId;
    type Incarnation = IncarnationId;
    type CheckpointStamp = OpaqueId;
    type CheckpointState = EncodedPayload;
    type Outcome = EncodedPayload;
    type ObservationKey = OpaqueId;
    type Observation = EncodedPayload;
}

const fn _producer_is_an_actor()
where
    ActorId: ProducerIdentity,
{
}
