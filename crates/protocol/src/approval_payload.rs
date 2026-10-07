//! Stable payload carriers for the live runtime approval queue.
//!
//! The generic protocol already owns the `ApprovalDecide` verb. This module
//! supplies its product payload and the corresponding query projection without
//! adding a second action vocabulary. All integer identities use `Value::UInt`
//! so the full runtime `u64` space survives the wire.

use circular_core::{Ceilings, CodecError, Value, decode, encode};

use crate::scope_identity::{
    PlanActorKey, ScopeSegment, decode_plan_actor_key, decode_scope_identity, plan_actor_key_value,
    scope_identity_value,
};
use crate::wire_value::{WireError, exhausted, object_from_value, take};

pub const RUNTIME_APPROVALS_QUERY: &str = "runtime.approvals";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalProducerAvailability {
    Available,
    Unavailable(ApprovalProducerUnavailable),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalProducerUnavailable {
    NoRegisteredApprovalEmitter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalQueuePersistence {
    Durable,
    Unavailable(ApprovalQueuePersistenceUnavailable),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalQueuePersistenceUnavailable {
    PendingOutcomeRouteIsProcessLocal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalSummaryAvailability {
    Unavailable(ApprovalSummaryUnavailable),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalSummaryUnavailable {
    ProductEffectSummaryIsUnit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalEmitterIdentity {
    Named {
        scope: Vec<ScopeSegment>,
        local: String,
    },
    Ephemeral {
        scope: Vec<ScopeSegment>,
        uuid: [u8; 16],
    },
    SystemStream,
    SystemHeartbeat,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ApprovalOpenState {
    Requested,
    Approved {
        ledger_item: Value,
        target_effect: Value,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalCause {
    pub actor: PlanActorKey,
    pub index: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalQueueRow {
    pub item: Value,
    pub emitter: ApprovalEmitterIdentity,
    pub target_effect: Value,
    pub state: ApprovalOpenState,
    pub summary: ApprovalSummaryAvailability,
    pub cause: Option<ApprovalCause>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalQueueAnchor {
    pub producer: ApprovalProducerAvailability,
    pub persistence: ApprovalQueuePersistence,
}

circular_core::closed_table! {
    pub enum ApprovalDecisionValue: i64 {
        Approve = 1,
        Deny = 2,
    }
}

/// The pending item whose decision is requested.
#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalDecisionItem {
    pub item: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ApprovalDecisionOutcome {
    Approved {
        ledger_item: Value,
        target_effect: Value,
    },
    Denied,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalDecisionReceipt {
    pub item: Value,
    pub outcome: ApprovalDecisionOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalPayloadRejection {
    Codec(CodecError),
    /// Object structure — shared with every object decoder.
    Wire(WireError),
    WrongCarrier(&'static str),
    UnknownTag {
        field: &'static str,
        tag: i64,
    },
    WrongUuidLength(usize),
    Scope,
}

pub fn approval_queue_anchor_value(
    anchor: &ApprovalQueueAnchor,
) -> Result<Value, ApprovalPayloadRejection> {
    object([
        ("producer", producer_value(anchor.producer)),
        ("persistence", persistence_value(anchor.persistence)),
    ])
}

pub fn decode_approval_queue_anchor(
    value: Value,
) -> Result<ApprovalQueueAnchor, ApprovalPayloadRejection> {
    let mut fields = object_from_value(value)?;
    let producer = decode_producer(take(&mut fields, "producer")?)?;
    let persistence = decode_persistence(take(&mut fields, "persistence")?)?;
    exhausted(fields)?;
    Ok(ApprovalQueueAnchor {
        producer,
        persistence,
    })
}

pub fn approval_queue_row_value(row: &ApprovalQueueRow) -> Result<Value, ApprovalPayloadRejection> {
    object([
        ("item", row.item.clone()),
        ("emitter", emitter_value(&row.emitter)?),
        ("target_effect", row.target_effect.clone()),
        ("state", state_value(&row.state)),
        ("summary", summary_value(row.summary)),
        ("cause", cause_value(row.cause.as_ref())?),
    ])
}

pub fn decode_approval_queue_row(
    value: Value,
) -> Result<ApprovalQueueRow, ApprovalPayloadRejection> {
    let mut fields = object_from_value(value)?;
    let item = effect_key(take(&mut fields, "item")?, "item")?;
    let emitter = decode_emitter(take(&mut fields, "emitter")?)?;
    let target_effect = effect_key(take(&mut fields, "target_effect")?, "target_effect")?;
    let state = decode_state(take(&mut fields, "state")?)?;
    let summary = decode_summary(take(&mut fields, "summary")?)?;
    let cause = decode_cause(take(&mut fields, "cause")?)?;
    exhausted(fields)?;
    Ok(ApprovalQueueRow {
        item,
        emitter,
        target_effect,
        state,
        summary,
        cause,
    })
}

pub fn encode_approval_decision(
    item: ApprovalDecisionItem,
    decision: ApprovalDecisionValue,
    ceilings: Ceilings,
) -> Result<Vec<u8>, ApprovalPayloadRejection> {
    let value = object([
        ("item", item.item.clone()),
        ("decision", Value::Int(decision.tag())),
    ])?;
    encode(&value, ceilings).map_err(ApprovalPayloadRejection::Codec)
}

pub fn decode_approval_decision(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<(ApprovalDecisionItem, ApprovalDecisionValue), ApprovalPayloadRejection> {
    let mut fields =
        object_from_value(decode(bytes, ceilings).map_err(ApprovalPayloadRejection::Codec)?)?;
    let item = effect_key(take(&mut fields, "item")?, "item")?;
    let tag = integer(take(&mut fields, "decision")?, "decision")?;
    let decision =
        ApprovalDecisionValue::from_tag(tag).ok_or(ApprovalPayloadRejection::UnknownTag {
            field: "decision",
            tag,
        })?;
    exhausted(fields)?;
    Ok((ApprovalDecisionItem { item }, decision))
}

pub fn approval_decision_receipt_value(
    receipt: &ApprovalDecisionReceipt,
) -> Result<Value, ApprovalPayloadRejection> {
    let outcome = match &receipt.outcome {
        ApprovalDecisionOutcome::Approved {
            ledger_item,
            target_effect,
        } => Value::Array(vec![
            Value::Int(1),
            ledger_item.clone(),
            target_effect.clone(),
        ]),
        ApprovalDecisionOutcome::Denied => Value::Int(2),
    };
    object([("item", receipt.item.clone()), ("outcome", outcome)])
}

fn producer_value(value: ApprovalProducerAvailability) -> Value {
    match value {
        ApprovalProducerAvailability::Available => Value::Int(1),
        ApprovalProducerAvailability::Unavailable(
            ApprovalProducerUnavailable::NoRegisteredApprovalEmitter,
        ) => Value::Array(vec![Value::Int(2), Value::Int(1)]),
    }
}

fn decode_producer(value: Value) -> Result<ApprovalProducerAvailability, ApprovalPayloadRejection> {
    match value {
        Value::Int(1) => Ok(ApprovalProducerAvailability::Available),
        Value::Array(values) if values == [Value::Int(2), Value::Int(1)] => {
            Ok(ApprovalProducerAvailability::Unavailable(
                ApprovalProducerUnavailable::NoRegisteredApprovalEmitter,
            ))
        }
        Value::Int(tag) => Err(ApprovalPayloadRejection::UnknownTag {
            field: "producer",
            tag,
        }),
        _ => Err(ApprovalPayloadRejection::WrongCarrier("producer")),
    }
}

fn persistence_value(value: ApprovalQueuePersistence) -> Value {
    match value {
        ApprovalQueuePersistence::Durable => Value::Array(vec![Value::Int(3)]),
        ApprovalQueuePersistence::Unavailable(
            ApprovalQueuePersistenceUnavailable::PendingOutcomeRouteIsProcessLocal,
        ) => Value::Array(vec![Value::Int(2), Value::Int(1)]),
    }
}

fn decode_persistence(value: Value) -> Result<ApprovalQueuePersistence, ApprovalPayloadRejection> {
    match value {
        Value::Array(values) if values == [Value::Int(3)] => Ok(ApprovalQueuePersistence::Durable),
        Value::Array(values) if values == [Value::Int(2), Value::Int(1)] => {
            Ok(ApprovalQueuePersistence::Unavailable(
                ApprovalQueuePersistenceUnavailable::PendingOutcomeRouteIsProcessLocal,
            ))
        }
        _ => Err(ApprovalPayloadRejection::WrongCarrier("persistence")),
    }
}

fn summary_value(value: ApprovalSummaryAvailability) -> Value {
    match value {
        ApprovalSummaryAvailability::Unavailable(
            ApprovalSummaryUnavailable::ProductEffectSummaryIsUnit,
        ) => Value::Array(vec![Value::Int(2), Value::Int(1)]),
    }
}

fn decode_summary(value: Value) -> Result<ApprovalSummaryAvailability, ApprovalPayloadRejection> {
    match value {
        Value::Array(values) if values == [Value::Int(2), Value::Int(1)] => {
            Ok(ApprovalSummaryAvailability::Unavailable(
                ApprovalSummaryUnavailable::ProductEffectSummaryIsUnit,
            ))
        }
        _ => Err(ApprovalPayloadRejection::WrongCarrier("summary")),
    }
}

fn cause_value(cause: Option<&ApprovalCause>) -> Result<Value, ApprovalPayloadRejection> {
    let Some(cause) = cause else {
        return Ok(Value::Null);
    };
    let index =
        i64::try_from(cause.index).map_err(|_| ApprovalPayloadRejection::WrongCarrier("cause"))?;
    object([
        ("actor", plan_actor_key_value(&cause.actor)),
        ("index", Value::Int(index)),
    ])
}

fn decode_cause(value: Value) -> Result<Option<ApprovalCause>, ApprovalPayloadRejection> {
    if value == Value::Null {
        return Ok(None);
    }
    let mut fields = object_from_value(value)?;
    let actor = decode_plan_actor_key(take(&mut fields, "actor")?)
        .map_err(|_| ApprovalPayloadRejection::WrongCarrier("cause.actor"))?;
    let index = match take(&mut fields, "index")? {
        Value::Int(index) => u64::try_from(index)
            .map_err(|_| ApprovalPayloadRejection::WrongCarrier("cause.index"))?,
        _ => return Err(ApprovalPayloadRejection::WrongCarrier("cause.index")),
    };
    exhausted(fields)?;
    Ok(Some(ApprovalCause { actor, index }))
}

fn emitter_value(value: &ApprovalEmitterIdentity) -> Result<Value, ApprovalPayloadRejection> {
    Ok(match value {
        ApprovalEmitterIdentity::Named { scope, local } => Value::Array(vec![
            Value::Int(1),
            scope_identity_value(scope),
            Value::String(local.clone()),
        ]),
        ApprovalEmitterIdentity::Ephemeral { scope, uuid } => Value::Array(vec![
            Value::Int(2),
            scope_identity_value(scope),
            Value::Bytes(uuid.to_vec()),
        ]),
        ApprovalEmitterIdentity::SystemStream => Value::Int(3),
        ApprovalEmitterIdentity::SystemHeartbeat => Value::Int(4),
    })
}

fn decode_emitter(value: Value) -> Result<ApprovalEmitterIdentity, ApprovalPayloadRejection> {
    match value {
        Value::Array(mut fields) if fields.len() == 3 => {
            let identity = fields.remove(2);
            let scope = decode_scope_identity(fields.remove(1))
                .map_err(|_| ApprovalPayloadRejection::Scope)?;
            match fields.remove(0) {
                Value::Int(1) => match identity {
                    Value::String(local) => Ok(ApprovalEmitterIdentity::Named { scope, local }),
                    _ => Err(ApprovalPayloadRejection::WrongCarrier("emitter.local")),
                },
                Value::Int(2) => match identity {
                    Value::Bytes(bytes) if bytes.len() == 16 => {
                        let mut uuid = [0_u8; 16];
                        uuid.copy_from_slice(&bytes);
                        Ok(ApprovalEmitterIdentity::Ephemeral { scope, uuid })
                    }
                    Value::Bytes(bytes) => {
                        Err(ApprovalPayloadRejection::WrongUuidLength(bytes.len()))
                    }
                    _ => Err(ApprovalPayloadRejection::WrongCarrier("emitter.uuid")),
                },
                Value::Int(tag) => Err(ApprovalPayloadRejection::UnknownTag {
                    field: "emitter",
                    tag,
                }),
                _ => Err(ApprovalPayloadRejection::WrongCarrier("emitter")),
            }
        }
        Value::Int(3) => Ok(ApprovalEmitterIdentity::SystemStream),
        Value::Int(4) => Ok(ApprovalEmitterIdentity::SystemHeartbeat),
        Value::Int(tag) => Err(ApprovalPayloadRejection::UnknownTag {
            field: "emitter",
            tag,
        }),
        _ => Err(ApprovalPayloadRejection::WrongCarrier("emitter")),
    }
}

fn state_value(value: &ApprovalOpenState) -> Value {
    match value {
        ApprovalOpenState::Requested => Value::Int(1),
        ApprovalOpenState::Approved {
            ledger_item,
            target_effect,
        } => Value::Array(vec![
            Value::Int(2),
            ledger_item.clone(),
            target_effect.clone(),
        ]),
    }
}

fn decode_state(value: Value) -> Result<ApprovalOpenState, ApprovalPayloadRejection> {
    match value {
        Value::Int(1) => Ok(ApprovalOpenState::Requested),
        Value::Array(values) if values.len() == 3 => match values.as_slice() {
            [Value::Int(2), ledger_item, target_effect]
                if is_effect_key(ledger_item) && is_effect_key(target_effect) =>
            {
                Ok(ApprovalOpenState::Approved {
                    ledger_item: ledger_item.clone(),
                    target_effect: target_effect.clone(),
                })
            }
            _ => Err(ApprovalPayloadRejection::WrongCarrier("state")),
        },
        Value::Int(tag) => Err(ApprovalPayloadRejection::UnknownTag {
            field: "state",
            tag,
        }),
        _ => Err(ApprovalPayloadRejection::WrongCarrier("state")),
    }
}

fn object<const N: usize>(
    entries: [(&'static str, Value); N],
) -> Result<Value, ApprovalPayloadRejection> {
    Value::object(entries).map_err(|_| ApprovalPayloadRejection::Wire(WireError::not_object(None)))
}

impl From<WireError> for ApprovalPayloadRejection {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

fn is_effect_key(value: &Value) -> bool {
    matches!(value, Value::Array(fields) if fields.len() == 4)
}
fn effect_key(value: Value, field: &'static str) -> Result<Value, ApprovalPayloadRejection> {
    if is_effect_key(&value) {
        Ok(value)
    } else {
        Err(ApprovalPayloadRejection::WrongCarrier(field))
    }
}
fn integer(value: Value, field: &'static str) -> Result<i64, ApprovalPayloadRejection> {
    match value {
        Value::Int(value) => Ok(value),
        _ => Err(ApprovalPayloadRejection::WrongCarrier(field)),
    }
}

#[cfg(test)]
mod durable_persistence_tests {
    use super::*;
    #[test]
    fn durable_persistence_uses_literal_tag_three() {
        let literal = Value::Array(vec![Value::Int(3)]);
        assert_eq!(
            persistence_value(ApprovalQueuePersistence::Durable),
            literal
        );
        assert_eq!(
            decode_persistence(literal),
            Ok(ApprovalQueuePersistence::Durable)
        );
        assert!(decode_persistence(Value::Array(vec![Value::Int(3), Value::Int(1)])).is_err());
    }
}

#[cfg(test)]
mod cause_tests {
    use super::*;
    use crate::scope_identity::ActorLocal;

    fn row(cause: Option<ApprovalCause>) -> ApprovalQueueRow {
        let key = Value::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4),
        ]);
        ApprovalQueueRow {
            item: key.clone(),
            emitter: ApprovalEmitterIdentity::Named {
                scope: Vec::new(),
                local: "rollback".to_owned(),
            },
            target_effect: key,
            state: ApprovalOpenState::Requested,
            summary: ApprovalSummaryAvailability::Unavailable(
                ApprovalSummaryUnavailable::ProductEffectSummaryIsUnit,
            ),
            cause,
        }
    }

    #[test]
    fn approval_decision_wire_names_only_the_item_and_decision() {
        use circular_core::Boundary;
        let ceilings = Ceilings::for_boundary(Boundary::Wire);
        let literal = [
            8, 0, 0, 0, 2, 0, 0, 0, 8, b'd', b'e', b'c', b'i', b's', b'i', b'o', b'n', 3, 0, 0, 0,
            0, 0, 0, 0, 2, 0, 0, 0, 4, b'i', b't', b'e', b'm', 7, 0, 0, 0, 4, 1, 1, 1, 1,
        ];
        let item = Value::array([Value::Null, Value::Null, Value::Null, Value::Null]);
        let request = ApprovalDecisionItem { item: item.clone() };
        assert_eq!(
            encode_approval_decision(request.clone(), ApprovalDecisionValue::Deny, ceilings)
                .unwrap(),
            literal
        );
        assert_eq!(
            decode_approval_decision(&literal, ceilings).unwrap(),
            (request, ApprovalDecisionValue::Deny)
        );
        let old = encode(
            &Value::object([
                ("decision", Value::Int(2)),
                ("item", item),
                ("expected_run", Value::UInt(1)),
            ])
            .unwrap(),
            ceilings,
        )
        .unwrap();
        assert!(matches!(
            decode_approval_decision(&old, ceilings),
            Err(ApprovalPayloadRejection::Wire(_))
        ));
    }

    fn field(value: &Value, key: &str) -> Value {
        match value {
            Value::Object(object) => object.clone().into_map().remove(key).expect("field"),
            _ => panic!("row is an object"),
        }
    }
}
