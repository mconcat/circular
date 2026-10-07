
use crate::record::{
    ArrivalOrigin, BoundaryFact, Class, ClassKey, ObservationBucket, ObservationFact,
    OperationCoordinate, Record, RecordOrigin, RecordPosition, StoreSchema,
};
use circular_core::{
    ArrivalIndex, BigEndian, ByteReadError, ByteReader, RecordedInstant, RevisionEpochId, Stamp,
};
use std::fmt;

pub const RECORD_VERSION: u8 = circular_core::compatibility::RECORD_FORMAT_VERSION;

const CLASS_BOUNDARY: u8 = 1;
const CLASS_STRUCTURE: u8 = 2;
const CLASS_DISPLAY: u8 = 4;
const CLASS_OBSERVATION: u8 = 5;

const ORIGIN_ACTOR: u8 = 1;
const ORIGIN_STREAM: u8 = 2;

const POSITION_STAMPED: u8 = 0;
const POSITION_OPERATION: u8 = 2;

const fn class_tag(class: Class) -> u8 {
    match class {
        Class::Boundary => CLASS_BOUNDARY,
        Class::Structure => CLASS_STRUCTURE,
        Class::Display => CLASS_DISPLAY,
        Class::Observation => CLASS_OBSERVATION,
    }
}

const fn class_of_tag(tag: u8) -> Option<Class> {
    match tag {
        CLASS_BOUNDARY => Some(Class::Boundary),
        CLASS_STRUCTURE => Some(Class::Structure),
        CLASS_DISPLAY => Some(Class::Display),
        CLASS_OBSERVATION => Some(Class::Observation),
        _ => None,
    }
}

pub(crate) const fn class_key_tag<S: StoreSchema>(key: &ClassKey<S>) -> u8 {
    match key {
        ClassKey::Boundary(inner) => match inner {
            crate::record::BoundaryKey::Arrival { .. } => 1,
            crate::record::BoundaryKey::ScheduleReservation { .. } => 3,
            crate::record::BoundaryKey::Admission { .. } => 4,
            crate::record::BoundaryKey::EmissionBody { .. } => 5,
        },
        ClassKey::Structure(inner) => match inner {
            crate::record::StructureKey::Manifest => 1,
            crate::record::StructureKey::Revision(..) => 2,
        },
        ClassKey::Display { .. } => 1,
        ClassKey::Observation(inner) => match inner {
            crate::record::ObservationKey::StreamItem(..) => 1,
            crate::record::ObservationKey::GlobalItem(..) => 2,
            crate::record::ObservationKey::CheckpointItem(..) => 3,
        },
    }
}

pub trait RecordIdentityCodec<S: StoreSchema> {
    fn producer(&self, producer: &S::Producer) -> Result<Vec<u8>>;
    fn origin_body(&self, origin: &RecordOrigin<S>) -> Result<Vec<u8>>;
    fn class_key_body(&self, key: &ClassKey<S>) -> Result<Vec<u8>>;
    fn payload(&self, record: &Record<S>) -> Result<Vec<u8>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordCodecError {
    Truncated,
    UnknownVersion(u8),
    UnknownClass(u8),
    UnknownArrivalOrigin(u8),
    UnknownObservationFact(u8),
    OriginFactMismatch,
    UnknownOrigin(u8),
    InvalidPositionTag(u8),
    InvalidRevision(u64),
    LengthOutOfRange,
    LengthMismatch,
    Identity(crate::product_identity::ProductIdentityError),
}

impl fmt::Display for RecordCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRevision(value) => {
                write!(formatter, "invalid revision coordinate {value}")
            }
            Self::Truncated => {
                formatter.write_str("record bytes ended before the field was complete")
            }
            Self::UnknownVersion(version) => {
                write!(formatter, "unknown stored batch version {version}")
            }
            Self::UnknownClass(tag) => write!(formatter, "unknown record class tag {tag}"),
            Self::UnknownOrigin(tag) => write!(formatter, "unknown attribution tag {tag}"),
            Self::UnknownArrivalOrigin(tag) => {
                write!(formatter, "unknown arrival source tag {tag}")
            }
            Self::UnknownObservationFact(tag) => {
                write!(formatter, "unknown observed fact tag {tag}")
            }
            Self::OriginFactMismatch => {
                formatter.write_str("record origin arm does not match its observed fact arm")
            }
            Self::Identity(source) => {
                write!(formatter, "cannot encode identity as bytes: {source}")
            }
            Self::InvalidPositionTag(byte) => {
                write!(formatter, "unknown record position tag: {byte:#04x}")
            }
            Self::LengthOutOfRange => formatter.write_str("declared length exceeds the boundary"),
            Self::LengthMismatch => {
                formatter.write_str("declared length differs from the consumed byte count")
            }
        }
    }
}

impl std::error::Error for RecordCodecError {}

type Result<T> = std::result::Result<T, RecordCodecError>;

type RecordReader<'bytes> = ByteReader<'bytes, BigEndian>;

impl From<ByteReadError> for RecordCodecError {
    fn from(error: ByteReadError) -> Self {
        match error {
            ByteReadError::Truncated => Self::Truncated,
            ByteReadError::LengthOverflow => Self::LengthOutOfRange,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvelopeOrigin<'bytes> {
    Stamped {
        l: u64,
        c: u64,
        producer: &'bytes [u8],
        sequence: u64,
        revision: RevisionEpochId,
    },
    OperationCoordinate {
        namespace: &'bytes [u8],
        commit: u64,
        operation: u32,
        at: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordEnvelope<'bytes> {
    pub version: u8,
    pub class: Class,
    pub origin: EnvelopeOrigin<'bytes>,
    pub attribution_tag: u8,
    pub attribution_body: &'bytes [u8],
    pub class_key_tag: u8,
    pub arrival_origin_tag: Option<u8>,
    pub arrival_index: Option<ArrivalIndex>,
    pub observation_fact_tag: Option<u8>,
    pub observation_bucket: Option<ObservationBucket>,
    pub observed_at: Option<RecordedInstant>,
    pub class_key_body: &'bytes [u8],
    pub payload: &'bytes [u8],
    pub total_len: usize,
}

pub fn encode_record<S, C>(record: &Record<S>, codec: &C) -> Result<Vec<u8>>
where
    S: StoreSchema,
    C: RecordIdentityCodec<S>,
{
    let header = record.header();
    let mut body = Vec::new();

    body.push(class_tag(header.class()));

    let fact_tag = match record {
        Record::Observation(observation) => Some(observation_fact_tag(observation.fact())),
        _ => None,
    };
    let checkpoint = fact_tag == Some(CHECKPOINT_FACT_TAG);
    match header.position() {
        RecordPosition::Stamped(at) => {
            if checkpoint {
                return Err(RecordCodecError::OriginFactMismatch);
            }
            body.push(POSITION_STAMPED);
            encode_stamp(&mut body, at, codec)?;
        }
        RecordPosition::OperationCoordinate(coordinate) => {
            if !checkpoint {
                return Err(RecordCodecError::OriginFactMismatch);
            }
            body.push(POSITION_OPERATION);
            encode_operation_coordinate(&mut body, coordinate)?;
        }
    }

    body.push(match header.origin() {
        RecordOrigin::Actor(_) => ORIGIN_ACTOR,
        RecordOrigin::Stream => ORIGIN_STREAM,
    });
    push_len_prefixed(&mut body, &codec.origin_body(header.origin())?)?;

    let key_tag = class_key_tag(header.key());
    if checkpoint != (key_tag == CHECKPOINT_KEY_TAG && header.class() == Class::Observation) {
        return Err(RecordCodecError::OriginFactMismatch);
    }
    body.push(key_tag);
    if let ClassKey::Boundary(
        crate::record::BoundaryKey::Arrival { origin, .. }
        | crate::record::BoundaryKey::Admission { origin, .. },
    ) = header.key()
    {
        body.push(arrival_key_tag(origin));
    }
    if let Some(tag) = fact_tag {
        body.push(tag);
    }
    push_len_prefixed(&mut body, &codec.class_key_body(header.key())?)?;

    if let ClassKey::Observation(key) = header.key() {
        body.extend_from_slice(&key.bucket().millis().to_be_bytes());
    }

    if let Some(BoundaryFact::Arrival { arrival_index, .. }) = arrival_fact(record) {
        body.extend_from_slice(&arrival_index.get().to_be_bytes());
    }

    if let Some(
        BoundaryFact::Arrival { observed_at, .. } | BoundaryFact::Admission { observed_at, .. },
    ) = arrival_fact(record)
    {
        body.extend_from_slice(&observed_at.millis().to_be_bytes());
    }

    push_len_prefixed(&mut body, &codec.payload(record)?)?;

    let mut output = Vec::with_capacity(body.len() + 5);
    output.push(RECORD_VERSION);
    let length = u32::try_from(body.len()).map_err(|_| RecordCodecError::LengthOutOfRange)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&body);
    Ok(output)
}

pub(crate) const fn arrival_origin_tag<S: StoreSchema>(origin: &ArrivalOrigin<S>) -> u8 {
    match origin {
        ArrivalOrigin::EdgeDelivery { .. } => 1,
        ArrivalOrigin::TimerFire { .. } => 2,
        ArrivalOrigin::EffectOutcome { .. } => 3,
        ArrivalOrigin::ExternalInject { .. } => 4,
    }
}

pub(crate) const fn arrival_key_tag<S: StoreSchema>(origin: &crate::record::ArrivalKey<S>) -> u8 {
    match origin {
        crate::record::ArrivalKey::EdgeDelivery { .. } => 1,
        crate::record::ArrivalKey::TimerFire { .. } => 2,
        crate::record::ArrivalKey::EffectOutcome { .. } => 3,
        crate::record::ArrivalKey::ExternalInject { .. } => 4,
    }
}

#[must_use]
pub const fn observation_fact_tag<S: StoreSchema>(fact: &ObservationFact<S>) -> u8 {
    match fact {
        ObservationFact::Lifecycle(_) => LIFECYCLE_FACT_TAG,
        ObservationFact::Diagnostic(_) => DIAGNOSTIC_FACT_TAG,
        ObservationFact::Accounting(_) => ACCOUNTING_FACT_TAG,
        ObservationFact::DeadLetter(_) => DEAD_LETTER_FACT_TAG,
        ObservationFact::ReplaySessionTransition(_) => REPLAY_SESSION_FACT_TAG,
        ObservationFact::Restart(_) => RESTART_FACT_TAG,
        ObservationFact::Checkpoint(_) => CHECKPOINT_FACT_TAG,
    }
}

pub const LIFECYCLE_FACT_TAG: u8 = 1;
pub const DIAGNOSTIC_FACT_TAG: u8 = 2;
pub const ACCOUNTING_FACT_TAG: u8 = 3;
pub const DEAD_LETTER_FACT_TAG: u8 = 4;
pub const REPLAY_SESSION_FACT_TAG: u8 = 6;
pub const RESTART_FACT_TAG: u8 = 7;
pub const CHECKPOINT_FACT_TAG: u8 = 8;

pub const CHECKPOINT_KEY_TAG: u8 = 3;

fn arrival_fact<S: StoreSchema>(record: &Record<S>) -> Option<&BoundaryFact<S>> {
    match record {
        Record::Boundary(boundary) => Some(boundary.fact()),
        _ => None,
    }
}

fn encode_stamp<S, C>(output: &mut Vec<u8>, stamp: &Stamp<S::Producer>, codec: &C) -> Result<()>
where
    S: StoreSchema,
    C: RecordIdentityCodec<S>,
{
    let producer = codec.producer(stamp.producer())?;
    let length = u16::try_from(producer.len()).map_err(|_| RecordCodecError::LengthOutOfRange)?;
    output.extend_from_slice(&stamp.hlc().l().get().to_be_bytes());
    output.extend_from_slice(&stamp.hlc().c().get().to_be_bytes());
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&producer);
    output.extend_from_slice(&stamp.sequence().get().to_be_bytes());
    output.extend_from_slice(&stamp.revision().get().to_be_bytes());
    Ok(())
}

fn encode_operation_coordinate(
    output: &mut Vec<u8>,
    coordinate: &OperationCoordinate,
) -> Result<()> {
    push_len_prefixed(output, coordinate.namespace().as_bytes())?;
    output.extend_from_slice(&coordinate.commit().to_be_bytes());
    output.extend_from_slice(&coordinate.operation().to_be_bytes());
    output.extend_from_slice(&coordinate.at().to_be_bytes());
    Ok(())
}

fn push_len_prefixed(output: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let length = u32::try_from(bytes.len()).map_err(|_| RecordCodecError::LengthOutOfRange)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OpaqueWitness(Box<[u8]>);

impl OpaqueWitness {
    pub fn for_record_ref<S, C>(class: Class, key: &ClassKey<S>, codec: &C) -> Result<Self>
    where
        S: StoreSchema,
        C: RecordIdentityCodec<S>,
    {
        let body = codec.class_key_body(key)?;
        let mut bytes = Vec::with_capacity(2 + body.len());
        bytes.push(class_tag(class));
        bytes.push(class_key_tag(key));
        bytes.extend_from_slice(&body);
        Ok(Self(bytes.into_boxed_slice()))
    }

    #[must_use]
    pub fn for_envelope(envelope: &RecordEnvelope<'_>) -> Self {
        let mut bytes = Vec::with_capacity(2 + envelope.class_key_body.len());
        bytes.push(class_tag(envelope.class));
        bytes.push(envelope.class_key_tag);
        bytes.extend_from_slice(envelope.class_key_body);
        Self(bytes.into_boxed_slice())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 2 {
            return Err(RecordCodecError::Truncated);
        }
        if class_of_tag(bytes[0]).is_none() {
            return Err(RecordCodecError::UnknownClass(bytes[0]));
        }
        Ok(Self(bytes.to_vec().into_boxed_slice()))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn class(&self) -> Class {
        class_of_tag(self.0[0]).expect("the construction path checks the class tag")
    }
}

pub fn decode_envelope(bytes: &[u8]) -> Result<RecordEnvelope<'_>> {
    let mut reader = RecordReader::new(bytes);
    let version = reader.byte()?;
    if version != RECORD_VERSION {
        return Err(RecordCodecError::UnknownVersion(version));
    }
    let declared = reader.u32()? as usize;
    let body = reader.take(declared)?;
    let total_len = 5 + declared;

    let mut body_reader = RecordReader::new(body);
    let class =
        class_of_tag(body_reader.byte()?).ok_or_else(|| RecordCodecError::UnknownClass(body[0]))?;

    let position_tag = body_reader.byte()?;
    let origin = match position_tag {
        POSITION_STAMPED => decode_stamped(&mut body_reader)?,
        POSITION_OPERATION => EnvelopeOrigin::OperationCoordinate {
            namespace: body_reader.len_prefixed()?,
            commit: body_reader.u64()?,
            operation: body_reader.u32()?,
            at: body_reader.u64()?,
        },
        other => return Err(RecordCodecError::InvalidPositionTag(other)),
    };

    let attribution_tag = body_reader.byte()?;
    if attribution_tag != ORIGIN_ACTOR && attribution_tag != ORIGIN_STREAM {
        return Err(RecordCodecError::UnknownOrigin(attribution_tag));
    }
    let attribution_body = body_reader.len_prefixed()?;

    let class_key_tag = body_reader.byte()?;
    let arrival_origin = if class == Class::Boundary && (class_key_tag == 1 || class_key_tag == 4) {
        let tag = body_reader.byte()?;
        if tag == 0 || tag > 4 {
            return Err(RecordCodecError::UnknownArrivalOrigin(tag));
        }
        Some(tag)
    } else {
        None
    };
    let observation_fact = if class == Class::Observation {
        let tag = body_reader.byte()?;
        if !matches!(tag, 1..=4 | 6..=8) {
            return Err(RecordCodecError::UnknownObservationFact(tag));
        }
        Some(tag)
    } else {
        None
    };
    let coordinate_origin = matches!(origin, EnvelopeOrigin::OperationCoordinate { .. });
    let checkpoint_fact = observation_fact == Some(CHECKPOINT_FACT_TAG);
    if coordinate_origin != checkpoint_fact
        || (checkpoint_fact && class_key_tag != CHECKPOINT_KEY_TAG)
        || (!checkpoint_fact && class == Class::Observation && class_key_tag == CHECKPOINT_KEY_TAG)
    {
        return Err(RecordCodecError::OriginFactMismatch);
    }
    let class_key_body = body_reader.len_prefixed()?;

    let observation_bucket = if class == Class::Observation {
        Some(ObservationBucket::from_millis(body_reader.u64()?))
    } else {
        None
    };

    let arrival_index = if arrival_origin.is_some() && class_key_tag == 1 {
        Some(ArrivalIndex::new(body_reader.u64()?))
    } else {
        None
    };

    let observed_at = if arrival_origin.is_some() {
        Some(RecordedInstant::from_millis(body_reader.u64()?))
    } else {
        None
    };

    let payload = body_reader.len_prefixed()?;

    if !body_reader.finished() {
        return Err(RecordCodecError::LengthMismatch);
    }

    Ok(RecordEnvelope {
        version,
        class,
        origin,
        attribution_tag,
        attribution_body,
        class_key_tag,
        arrival_origin_tag: arrival_origin,
        arrival_index,
        observation_fact_tag: observation_fact,
        observation_bucket,
        observed_at,
        class_key_body,
        payload,
        total_len,
    })
}

fn decode_stamped<'bytes>(reader: &mut RecordReader<'bytes>) -> Result<EnvelopeOrigin<'bytes>> {
    let l = reader.u64()?;
    let c = reader.u64()?;
    let producer_len = reader.u16()? as usize;
    let producer = reader.take(producer_len)?;
    let sequence = reader.u64()?;
    let revision = {
        let value = reader.u64()?;
        RevisionEpochId::new(value).ok_or(RecordCodecError::InvalidRevision(value))?
    };
    Ok(EnvelopeOrigin::Stamped {
        l,
        c,
        producer,
        sequence,
        revision,
    })
}

pub fn decode_many(bytes: &[u8]) -> Result<Vec<RecordEnvelope<'_>>> {
    let mut envelopes = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let envelope = decode_envelope(&bytes[at..])?;
        at += envelope.total_len;
        envelopes.push(envelope);
    }
    Ok(envelopes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ManifestSchema;
    use circular_core::{EncodedPayload, PayloadVersionTag, Sequence};

    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    struct Id(u64);

    impl circular_core::StreamIdentity for Id {}

    impl circular_core::ProducerIdentity for Id {
        type EventProducer = Self;
        fn from_event_producer(producer: Self) -> Self {
            producer
        }
    }

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct TestSchema;

    macro_rules! u64_types {
        ($($name:ident),* $(,)?) => { $(type $name = Id;)* };
    }

    impl ManifestSchema for TestSchema {
        u64_types!(
            Stream,
            Producer,
            Placement,
            Failure,
            Versions,
            RevisionId,
            AuthoringCut,
            ScopeId,
            GrantSet,
            InputValue,
            CadencePolicy,
            TickOrigin,
        );
    }

    impl StoreSchema for TestSchema {
        u64_types!(
            Incarnation,
            ActorId,
            EdgeId,
            EffectId,
            TimerId,
            DisplayKey,
            ObservationKindKey,
            ObservationIdentityKey,
            ExternalOrigin,
            EffectTerm,
            EffectOutcome,
            GraphRevision,
            DisplayPayload,
            ObservationPayload,
        );
    }

    struct BigEndianCodec;

    impl RecordIdentityCodec<TestSchema> for BigEndianCodec {
        fn producer(&self, producer: &Id) -> Result<Vec<u8>> {
            Ok(producer.0.to_be_bytes().to_vec())
        }
        fn origin_body(&self, origin: &RecordOrigin<TestSchema>) -> Result<Vec<u8>> {
            Ok(match origin {
                RecordOrigin::Actor(incarnation) => incarnation.0.to_be_bytes().to_vec(),
                RecordOrigin::Stream => Vec::new(),
            })
        }
        fn class_key_body(&self, _key: &ClassKey<TestSchema>) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn payload(&self, _record: &Record<TestSchema>) -> Result<Vec<u8>> {
            Ok(EncodedPayload::new(PayloadVersionTag::FIRST, b"body")
                .as_bytes()
                .to_vec())
        }
    }

    fn stamp(producer: u64, sequence: u64) -> circular_core::Stamp<Id> {
        circular_core::Stamp::from_event_producer(
            circular_core::Tick::new(0),
            Id(producer),
            Sequence::new(sequence).expect("fixture sequence"),
            circular_core::RevisionEpochId::new(1).expect("first revision"),
        )
    }

    fn boundary_record() -> Record<TestSchema> {
        Record::Boundary(crate::record::BoundaryRecord::arrival(
            Id(42),
            circular_core::Stamp::from_event_producer_at(
                circular_core::Hlc::new(
                    circular_core::Tick::new(13),
                    circular_core::LogicalCounter::new(17),
                ),
                Id(3),
                Sequence::new(11).unwrap(),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            RecordOrigin::Actor(Id(42)),
            ArrivalOrigin::TimerFire { timer: Id(5) },
            crate::ArrivalBody::Owned(EncodedPayload::new(PayloadVersionTag::FIRST, b"body")),
            ArrivalIndex::new(6),
            Box::new([]),
            RecordedInstant::from_millis(9),
            None,
        ))
    }

    fn display_record() -> Record<TestSchema> {
        Record::Display(crate::record::DisplayRecord::new(
            stamp(3, 11),
            RecordOrigin::Actor(Id(42)),
            Id(5),
            Id(9),
        ))
    }

    fn arrival_record(origin: ArrivalOrigin<TestSchema>) -> Record<TestSchema> {
        Record::Boundary(crate::record::BoundaryRecord::arrival(
            Id(42),
            stamp(42, 3),
            RecordOrigin::Actor(Id(42)),
            origin,
            crate::ArrivalBody::Owned(EncodedPayload::new(PayloadVersionTag::FIRST, b"arrived")),
            ArrivalIndex::new(6),
            Box::new([]),
            RecordedInstant::from_millis(1_234),
            None,
        ))
    }

    #[test]
    fn a_record_that_is_not_an_arrival_carries_neither_slot() {
        let bytes = encode_record(&display_record(), &BigEndianCodec).expect("encode");
        let envelope = decode_envelope(&bytes).expect("decode");
        assert_eq!(envelope.arrival_origin_tag, None);
        assert_eq!(envelope.observed_at, None);
    }

    #[test]
    fn a_reserved_or_unassigned_arrival_tag_is_refused() {
        let record = arrival_record(ArrivalOrigin::TimerFire { timer: Id(77) });
        let good = encode_record(&record, &BigEndianCodec).expect("encode");
        let tag_at = 5 + 1 + 1 + (8 + 8 + 2 + 8 + 8 + 8) + (1 + 4 + 8) + 1;
        for bad in [0_u8, 5, 255] {
            let mut broken = good.clone();
            broken[tag_at] = bad;
            assert!(
                matches!(
                    decode_envelope(&broken),
                    Err(RecordCodecError::UnknownArrivalOrigin(_) | RecordCodecError::Truncated)
                ),
                "tag {bad} passed"
            );
        }
    }

    #[test]
    fn the_seven_observation_facts_take_distinct_tags_in_one_shape() {
        use crate::record::{ObservationBucket, ObservationItemKey, ObservationRecord};

        let bucket = ObservationBucket::from_millis(60_000);
        let by = || RecordOrigin::Stream;
        let item = || ObservationItemKey::new(Id(1), Id(2));
        let facts = [
            ObservationRecord::lifecycle(stamp(3, 11), bucket, by(), item(), Id(9)),
            ObservationRecord::diagnostic(stamp(3, 11), bucket, by(), item(), Id(9)),
            ObservationRecord::accounting(stamp(3, 11), bucket, by(), item(), Id(9)),
            ObservationRecord::dead_letter(stamp(3, 11), bucket, by(), item(), Id(9)),
            ObservationRecord::replay_session_transition(stamp(3, 11), bucket, by(), item(), Id(9)),
            ObservationRecord::restart(stamp(3, 11), bucket, by(), item(), Id(9)),
            ObservationRecord::checkpoint(
                crate::record::OperationCoordinate::new("arrival/7".into(), 12, 1, 44),
                bucket,
                by(),
                item(),
                Id(9),
            ),
        ];
        let mut seen = Vec::new();
        for fact in facts {
            let bytes = encode_record(&Record::Observation(fact), &BigEndianCodec).expect("encode");
            let envelope = decode_envelope(&bytes).expect("decode");
            assert_eq!(envelope.class, Class::Observation);
            seen.push(
                envelope
                    .observation_fact_tag
                    .expect("an observation carries a fact tag"),
            );
        }
        assert_eq!(seen, [1, 2, 3, 4, 6, 7, 8]);
        assert_eq!(
            seen,
            [
                LIFECYCLE_FACT_TAG,
                DIAGNOSTIC_FACT_TAG,
                ACCOUNTING_FACT_TAG,
                DEAD_LETTER_FACT_TAG,
                REPLAY_SESSION_FACT_TAG,
                RESTART_FACT_TAG,
                CHECKPOINT_FACT_TAG,
            ]
        );
    }

    #[test]
    fn reserved_and_unknown_versions_are_refused() {
        let mut bytes = encode_record(&boundary_record(), &BigEndianCodec).expect("encode");
        for version in [0x00_u8, 0xff, 0x03] {
            bytes[0] = version;
            assert_eq!(
                decode_envelope(&bytes),
                Err(RecordCodecError::UnknownVersion(version))
            );
        }
    }

    #[test]
    fn a_truncated_record_yields_no_partial_header() {
        let bytes = encode_record(&boundary_record(), &BigEndianCodec).expect("encode");
        for cut in 1..bytes.len() {
            assert!(
                decode_envelope(&bytes[..cut]).is_err(),
                "{cut} bytes decoded a partial record"
            );
        }
    }

    #[test]
    fn run_inputs_adds_only_the_payload_tag() {
        use crate::manifest::{RunInputs, encode_run_inputs};
        let inputs = RunInputs::from_primary_data(vec![7_u8, 8, 9]);
        let payload = encode_run_inputs(&inputs, |value| value.clone());
        assert_eq!(payload.version_tag(), PayloadVersionTag::FIRST);
        assert_eq!(payload.body(), &[7, 8, 9]);
        assert_eq!(payload.as_bytes(), &[0, 1, 7, 8, 9]);
    }
}
