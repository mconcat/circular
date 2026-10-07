#![forbid(unsafe_code)]

pub mod actor;
pub mod byte_reader;
mod closed_table;
pub mod codec;
 pub mod compatibility;
pub mod event;
pub mod fields;
pub mod identity;
pub mod observation;
pub mod ordering;
pub mod spelling;
pub mod time;
 pub mod value;

pub use actor::{ActorType, AuthoredPortId, PortId, PortIdError};
pub use byte_reader::{BigEndian, ByteReadError, ByteReader, Endian, LittleEndian};
pub use codec::{
    Boundary, CANONICAL_VALUE_TAG, Ceiling, Ceilings, CodecError, MAX_REASSEMBLED_BODY_BYTES,
    MAX_SEGMENT_BODY_BYTES, MAX_SEGMENTS_PER_VALUE, MAX_TEXT_BODY_BYTES, MAX_VALUES_PER_BODY,
    VALUE_FORMAT_VERSION, decode, decode_frame, encode, encode_frame,
};
pub use event::{
    AdmissionError, BaseShape, CausalParents, CausalParentsError, Causality, Emission,
    EmissionDraft, EncodedPayload, Event, EventId, EventPayload, FieldMap, FieldMapError,
    GroundShape, GroundShapeError, Payload, PayloadVersionTag, PayloadVersionTagError, Shape,
    admit, emit,
};
pub use fields::{
    FieldPath, FieldRejection, FieldRejectionKind, Fields, FromValue, NotObject, UnknownField,
};
pub use identity::{
    OperationIdentity, ProducerIdentity, ProducerLocalOperation, RecordProducerIdentity,
    StreamIdentity,
};
pub use observation::BuiltinObservationName;
pub use ordering::{
    ArrivalIndex, Hlc, LogicalCounter, ProducerSequencer, RevisionEpochId, Sequence, SequenceError,
    Stamp, StampIssueError, StampIssuer, inherited_revision,
};
pub use time::{
    LogDriven, LogDrivenTimeError, LogDrivenTimeSource, ManualTimeError, ManualTimeSource, Millis,
    NonZeroMillis, NonZeroTicks, NonZeroTicksError, RecordedInstant, Tick, TickSource, Ticks,
    TicksPerSecond, TicksPerSecondError, TimeSourceKind, TimeSourcePlan, WallAnchored,
    WallClockTimeSource, ZeroMillisError,
};
pub use value::{
    BudgetExhausted, DuplicateKeyError, EvaluationBudget, FloatValue, ObjectValue, Value, ValueKind,
};
