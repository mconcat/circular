
use circular_core::{
    Boundary, BuiltinObservationName, Ceilings, EncodedPayload, PayloadVersionTag, Value,
};
use circular_store::{
    ClassKey, IncarnationId, ObservationBucket, ObservationFact, ObservationItemKey,
    ObservationKey, ObservationRecord, OpaqueId, OperationCoordinate, ProductStore,
    ProductTransaction, Record, RecordOrigin, StoreTransactionOp,
};
use std::fmt;

pub const CHECKPOINT_OBSERVATION_NAME: BuiltinObservationName =
    BuiltinObservationName::CustodyClaim;

pub const REPLACE_CHECKPOINT_TAG: u64 = 13;
pub const SETTLE_CHECKPOINT_TAG: u64 = 14;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointValueRejection {
    FactIsNotCheckpoint,
    PayloadNotCanonical,
    CarrierIsNotObject,
    MissingField(&'static str),
    UnknownOperation(u64),
    ActorIsNotPublished,
}

impl fmt::Display for CheckpointValueRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FactIsNotCheckpoint => {
                formatter.write_str("checkpoint entry is not a Checkpoint observation")
            }
            Self::PayloadNotCanonical => {
                formatter.write_str("checkpoint payload is not a canonical Value")
            }
            Self::CarrierIsNotObject => formatter.write_str("checkpoint payload is not an object"),
            Self::MissingField(name) => write!(formatter, "checkpoint payload has no {name}"),
            Self::UnknownOperation(tag) => {
                write!(formatter, "unknown checkpoint operation tag {tag}")
            }
            Self::ActorIsNotPublished => {
                formatter.write_str("checkpoint actor is not a published address")
            }
        }
    }
}

impl std::error::Error for CheckpointValueRejection {}

pub fn checkpoint_record(
    namespace: &str,
    commit: u64,
    index: u32,
    operation: &StoreTransactionOp<ProductTransaction>,
) -> Result<Option<Record<ProductStore>>, String> {
    let Some(projected) = checkpoint_body(operation)? else {
        return Ok(None);
    };
    let mut fields = vec![
        (
            "operation",
            Value::UInt(u64::from(circular_store::op_tag(operation))),
        ),
        ("index", Value::UInt(u64::from(index))),
    ];
    fields.extend(projected.fields);
    let value = Value::object(fields).map_err(|e| format!("checkpoint carrier: {e:?}"))?;
    let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal))
        .map_err(|e| format!("checkpoint carrier: {e:?}"))?;
    let by = projected
        .owner
        .map_or_else(|| RecordOrigin::Stream, RecordOrigin::Actor);
    Ok(Some(Record::Observation(ObservationRecord::checkpoint(
        OperationCoordinate::new(namespace.to_owned(), commit, index, projected.at.get()),
        ObservationBucket::default(),
        by,
        ObservationItemKey::new(CHECKPOINT_OBSERVATION_NAME, projected.at),
        EncodedPayload::new(PayloadVersionTag::FIRST, &bytes),
    ))))
}

struct ProjectedCheckpoint {
    fields: Vec<(&'static str, Value)>,
    at: OpaqueId,
    owner: Option<IncarnationId>,
}

fn checkpoint_body(
    operation: &StoreTransactionOp<ProductTransaction>,
) -> Result<Option<ProjectedCheckpoint>, String> {
    Ok(match operation {
        StoreTransactionOp::ReplaceCheckpoint(row) => {
            let actor = circular_store::actor_value(row.actor())
                .map_err(|e| format!("checkpoint actor: {e:?}"))?;
            Some(ProjectedCheckpoint {
                fields: vec![
                    ("actor", actor),
                    ("at", Value::UInt(row.at().get())),
                    ("owner", Value::UInt(row.owner().get())),
                ],
                at: *row.at(),
                owner: Some(*row.owner()),
            })
        }
        StoreTransactionOp::SettleCheckpoint {
            actor,
            at,
            observation,
        } => {
            let actor_value = circular_store::actor_value(actor)
                .map_err(|e| format!("checkpoint actor: {e:?}"))?;
            Some(ProjectedCheckpoint {
                fields: vec![
                    ("actor", actor_value),
                    ("at", Value::UInt(at.get())),
                    ("settlement", Value::UInt(observation.key().get())),
                ],
                at: *at,
                owner: None,
            })
        }
        _ => None,
    })
}

pub fn checkpoint_value(
    record: &Record<ProductStore>,
) -> Result<Option<Value>, CheckpointValueRejection> {
    let Record::Observation(observation) = record else {
        return Ok(None);
    };
    let ClassKey::Observation(ObservationKey::CheckpointItem(_, _, item)) =
        observation.header().key()
    else {
        return Ok(None);
    };
    if item.kind() != &CHECKPOINT_OBSERVATION_NAME {
        return Ok(None);
    }
    let ObservationFact::Checkpoint(payload) = observation.fact() else {
        return Err(CheckpointValueRejection::FactIsNotCheckpoint);
    };
    let value = circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
        .map_err(|_| CheckpointValueRejection::PayloadNotCanonical)?;
    let Value::Object(fields) = &value else {
        return Err(CheckpointValueRejection::CarrierIsNotObject);
    };
    let Some(Value::UInt(tag)) = fields.get("operation") else {
        return Err(CheckpointValueRejection::MissingField("operation"));
    };
    if !matches!(*tag, REPLACE_CHECKPOINT_TAG | SETTLE_CHECKPOINT_TAG) {
        return Err(CheckpointValueRejection::UnknownOperation(*tag));
    }
    if !matches!(fields.get("index"), Some(Value::UInt(_))) {
        return Err(CheckpointValueRejection::MissingField("index"));
    }
    if !matches!(fields.get("at"), Some(Value::UInt(_))) {
        return Err(CheckpointValueRejection::MissingField("at"));
    }
    let actor = fields
        .get("actor")
        .ok_or(CheckpointValueRejection::MissingField("actor"))?;
    circular_store::actor_parts_from_value(actor)
        .map_err(|_| CheckpointValueRejection::ActorIsNotPublished)?;
    Ok(Some(value))
}
