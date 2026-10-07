
use circular_core::{BuiltinObservationName, ProducerIdentity, Stamp, Value};
use circular_protocol::dead_letter::{DeadLetterReasonRejection, decode_dead_letter_reason};
use circular_runtime::{DeadLetterLane, DeadLetterRecord};
use circular_store::{
    ClassKey, ObservationBucket, ObservationFact, ObservationItemKey, ObservationKey,
    ObservationRecord, ProductStore, Record, RecordOrigin, StoreSchema,
};
use std::fmt;

/// A record named `DeadLetterEntry` failed to yield its published value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeadLetterValueRejection {
    FactIsNotDeadLetter,
    PayloadNotCanonical,
    CarrierIsNotObject,
    MissingReason,
    InvalidReason(DeadLetterReasonRejection),
    MissingDropped,
    InvalidDropped(String),
}

impl fmt::Display for DeadLetterValueRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FactIsNotDeadLetter => {
                formatter.write_str("dead-letter entry is not a DeadLetter observation")
            }
            Self::PayloadNotCanonical => {
                formatter.write_str("dead-letter payload is not a canonical Value")
            }
            Self::CarrierIsNotObject => formatter.write_str("dead-letter payload is not an object"),
            Self::MissingReason => formatter.write_str("dead-letter payload has no reason"),
            Self::InvalidReason(reason) => {
                write!(formatter, "invalid dead-letter reason: {reason:?}")
            }
            Self::MissingDropped => formatter.write_str("dead-letter payload has no dropped stamp"),
            Self::InvalidDropped(reason) => write!(formatter, "invalid dropped stamp: {reason}"),
        }
    }
}

impl std::error::Error for DeadLetterValueRejection {}

#[derive(Clone, Debug)]
pub struct DeadLetterCoordinates<S: StoreSchema> {
    pub at: Stamp<S::Producer>,
    pub bucket: ObservationBucket,
    pub identity: S::ObservationIdentityKey,
    pub by: RecordOrigin<S>,
}

#[must_use]
pub fn project_dead_letter<S, P, D>(
    record: &DeadLetterRecord<P, D>,
    coordinates: DeadLetterCoordinates<S>,
    kind: impl FnOnce(BuiltinObservationName) -> S::ObservationKindKey,
    encode: impl FnOnce(&DeadLetterRecord<P, D>) -> S::ObservationPayload,
) -> Record<S>
where
    S: StoreSchema,
    P: ProducerIdentity,
{
    let DeadLetterCoordinates {
        at,
        bucket,
        identity,
        by,
    } = coordinates;
    Record::Observation(ObservationRecord::dead_letter(
        at,
        bucket,
        by,
        ObservationItemKey::new(kind(BuiltinObservationName::DeadLetterEntry), identity),
        encode(record),
    ))
}

#[must_use]
pub fn dead_letter_batch<S, P, D>(
    lane: &DeadLetterLane<P, D>,
    mut coordinates: impl FnMut(usize, &DeadLetterRecord<P, D>) -> DeadLetterCoordinates<S>,
    mut kind: impl FnMut(BuiltinObservationName) -> S::ObservationKindKey,
    mut encode: impl FnMut(&DeadLetterRecord<P, D>) -> S::ObservationPayload,
) -> Option<circular_store::AppendBatch<S>>
where
    S: StoreSchema,
    P: ProducerIdentity,
{
    let records = lane
        .records()
        .iter()
        .enumerate()
        .map(|(index, record)| {
            project_dead_letter::<S, P, D>(
                record,
                coordinates(index, record),
                &mut kind,
                &mut encode,
            )
        })
        .collect::<Vec<_>>();
    if records.is_empty() {
        return None;
    }
    Some(circular_store::AppendBatch::try_new(records).expect("non-emptiness was checked above"))
}

/// The validated carrier used by the daemon's `dead.letters` answer.
///
/// Other record kinds return `Ok(None)`. A record published under the dead-letter name is never
/// silently skipped: malformed payloads and unknown reason spellings are rejections.
pub fn dead_letter_value(
    record: &Record<ProductStore>,
) -> Result<Option<Value>, DeadLetterValueRejection> {
    let Record::Observation(observation) = record else {
        return Ok(None);
    };
    let ClassKey::Observation(ObservationKey::StreamItem(_, _, item)) = observation.header().key()
    else {
        return Ok(None);
    };
    if item.kind() != &BuiltinObservationName::DeadLetterEntry {
        return Ok(None);
    }
    let ObservationFact::DeadLetter(payload) = observation.fact() else {
        return Err(DeadLetterValueRejection::FactIsNotDeadLetter);
    };
    let value = circular_core::decode(
        payload.body(),
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
    )
    .map_err(|_| DeadLetterValueRejection::PayloadNotCanonical)?;
    let Value::Object(fields) = &value else {
        return Err(DeadLetterValueRejection::CarrierIsNotObject);
    };
    let reason = fields
        .get("reason")
        .ok_or(DeadLetterValueRejection::MissingReason)?;
    decode_dead_letter_reason(reason.clone()).map_err(DeadLetterValueRejection::InvalidReason)?;
    circular_store::record_stamp_from_value(
        fields
            .get("dropped")
            .ok_or(DeadLetterValueRejection::MissingDropped)?,
    )
    .map_err(DeadLetterValueRejection::InvalidDropped)?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{EncodedPayload, PayloadVersionTag, Sequence, Tick};
    use circular_plan::{ActorId, Name, ScopeId};
    use circular_runtime::{DeadLetterOrigin, DeadLetterReason};
    use circular_store::{
        Bound, IncarnationId, MemoryStore, ObservationScope, PagePolicy, ProductStore, Query,
        ScanStart, Store,
    };
    use std::num::NonZeroUsize;

    fn actor(value: &str) -> ActorId {
        ActorId::from(circular_plan::NamedActorId::new(
            ScopeId::root(),
            Name::from_normalized(value),
        ))
    }

    fn named(value: &str) -> circular_plan::NamedActorId {
        circular_plan::NamedActorId::new(ScopeId::root(), Name::from_normalized(value))
    }

    fn lane() -> DeadLetterLane<ActorId, u8> {
        let mut lane = DeadLetterLane::new(ScopeId::root());
        for (index, reason) in [
            DeadLetterReason::Poisoned,
            DeadLetterReason::DestinationGone,
            DeadLetterReason::OutcomeUnclaimed,
        ]
        .into_iter()
        .enumerate()
        {
            lane.write(DeadLetterRecord::new(
                u8::try_from(index).unwrap(),
                DeadLetterOrigin::new(actor("sink"), None),
                reason,
                circular_core::Stamp::from_event_producer(
                    Tick::ZERO,
                    named("sink"),
                    Sequence::new(u64::try_from(index).unwrap() + 1).unwrap(),
                    circular_core::RevisionEpochId::new(1).expect("first revision"),
                ),
            ));
        }
        lane
    }

    fn coordinates(
        index: usize,
        _record: &DeadLetterRecord<ActorId, u8>,
    ) -> DeadLetterCoordinates<ProductStore> {
        let ordinal = u64::try_from(index).unwrap() + 1;
        DeadLetterCoordinates {
            at: circular_core::Stamp::from_event_producer(
                Tick::new(ordinal),
                named("sink"),
                Sequence::new(ordinal).unwrap(),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            bucket: ObservationBucket::from_millis(ordinal),
            identity: circular_store::OpaqueId::new(ordinal),
            by: RecordOrigin::Actor(IncarnationId::new(ordinal)),
        }
    }

    fn encode(record: &DeadLetterRecord<ActorId, u8>) -> EncodedPayload {
        EncodedPayload::new(PayloadVersionTag::FIRST, &[*record.subject()])
    }

    #[test]
    fn an_empty_lane_writes_nothing() {
        assert!(
            dead_letter_batch::<ProductStore, ActorId, u8>(
                &DeadLetterLane::new(ScopeId::root()),
                coordinates,
                |name| name,
                encode,
            )
            .is_none()
        );
    }

    #[test]
    fn a_named_dead_letter_with_an_unknown_reason_is_rejected_not_skipped() {
        let lane = lane();
        let runtime_record = &lane.records()[0];
        let unknown_reason = Value::object([
            ("code", Value::String("future_reason".to_owned())),
            ("detail", Value::Null),
        ])
        .expect("reason fields");
        let carrier = Value::object([("reason", unknown_reason)]).expect("carrier fields");
        let bytes = circular_core::encode(
            &carrier,
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
        )
        .expect("carrier encodes");
        let record = project_dead_letter::<ProductStore, _, _>(
            runtime_record,
            coordinates(0, runtime_record),
            |name| name,
            |_| EncodedPayload::new(PayloadVersionTag::FIRST, &bytes),
        );

        assert!(matches!(
            dead_letter_value(&record),
            Err(DeadLetterValueRejection::InvalidReason(
                DeadLetterReasonRejection::UnknownReason(reason)
            )) if reason == "future_reason"
        ));
    }
}

pub fn product_dead_letter_record(
    scope: &circular_plan::ScopeId,
    record: &circular_runtime::DeadLetterRecord<
        circular_plan::ActorId,
        circular_actors::ProductPayload,
    >,
    at: Stamp<circular_plan::ActorId>,
    by: RecordOrigin<ProductStore>,
) -> Result<Record<ProductStore>, String> {
    let payload = product_dead_letter_payload(scope, record)?;
    Ok(
        project_dead_letter::<ProductStore, circular_plan::ActorId, _>(
            record,
            DeadLetterCoordinates {
                bucket: ObservationBucket::from_millis(at.physical_time().get()),
                at,
                identity: circular_store::OpaqueId::new(0),
                by,
            },
            |name| name,
            |_| payload,
        ),
    )
}

pub fn product_dead_letter_payload(
    scope: &circular_plan::ScopeId,
    record: &circular_runtime::DeadLetterRecord<
        circular_plan::ActorId,
        circular_actors::ProductPayload,
    >,
) -> Result<circular_core::EncodedPayload, String> {
    let shape = circular_protocol::port_type::PortShape::from_core(
        &circular_actors::types::to_unnamed_shape(record.subject().shape().as_shape()),
    );
    let shape = circular_protocol::port_type::encode_port_shape(&shape)
        .map_err(|error| format!("dead-letter subject shape is not publishable: {error}"))?;
    let origin = Value::object([
        (
            "actor",
            circular_store::actor_value(record.origin().actor())
                .map_err(|error| format!("dead-letter origin is not publishable: {error}"))?,
        ),
        (
            "port",
            record
                .origin()
                .port()
                .map_or(Value::Null, |port| Value::String(port.as_str().to_owned())),
        ),
    ])
    .map_err(|error| format!("dead-letter origin carrier is invalid: {error:?}"))?;
    let subject = Value::object([
        ("shape", shape),
        ("value", record.subject().value().clone()),
    ])
    .map_err(|error| format!("dead-letter subject carrier is invalid: {error:?}"))?;
    let mut fields = vec![
        (
            "dropped",
            circular_store::record_stamp_value(record.admitted())?,
        ),
        ("origin", origin),
        (
            "reason",
            circular_actors::dead_letter_reason_with_failure_point(
                record.reason(),
                record.failure_point(),
            ),
        ),
        (
            "scope",
            circular_store::scope_value(scope)
                .map_err(|error| format!("dead-letter scope is not publishable: {error}"))?,
        ),
        ("subject", subject),
    ];
    if let Some(target) = record.target() {
        let target = match target {
            circular_runtime::DeadLetterTarget::Delivery(edge) => circular_store::edge_value(edge)
                .map_err(|error| format!("dead-letter target edge: {error}"))?,
            circular_runtime::DeadLetterTarget::Outlet { actor, port } => Value::object([
                (
                    "actor",
                    circular_store::actor_value(actor)
                        .map_err(|error| format!("dead-letter target actor: {error}"))?,
                ),
                ("port", Value::string(port.as_str())),
            ])
            .map_err(|error| format!("dead-letter target outlet: {error:?}"))?,
        };
        fields.push(("target", target));
    }
    let value = Value::object(fields)
        .map_err(|error| format!("dead-letter carrier is invalid: {error:?}"))?;
    let bytes = circular_core::encode(
        &value,
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
    )
    .map_err(|error| format!("dead-letter carrier does not encode: {error}"))?;
    Ok(circular_core::EncodedPayload::new(
        circular_core::PayloadVersionTag::FIRST,
        &bytes,
    ))
}
