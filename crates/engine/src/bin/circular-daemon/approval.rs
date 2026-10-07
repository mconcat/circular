//! Strict daemon projection for live runtime approval custody.

use circular_core::Value;
use circular_plan::{ActorId, InstanceKey, LocalKey, ScopeSeg, SystemActor};
use circular_protocol::approval_payload::{
    ApprovalCause, ApprovalDecisionOutcome, ApprovalDecisionReceipt, ApprovalEmitterIdentity,
    ApprovalOpenState, ApprovalProducerAvailability, ApprovalQueueAnchor, ApprovalQueuePersistence,
    ApprovalQueuePersistenceUnavailable, ApprovalQueueRow, ApprovalSummaryAvailability,
    ApprovalSummaryUnavailable, approval_decision_receipt_value, approval_queue_anchor_value,
    approval_queue_row_value,
};
use circular_protocol::declaration_payload::{QueryPage, Terminal};
use circular_protocol::scope_identity::ScopeSegment as WireScopeSegment;
use circular_runtime::{ApprovalRequestOutcome, OpenApprovalState};
use engine::{
    RuntimeApprovalDecision, RuntimeApprovalQueueSnapshot, RuntimeApprovalRow,
    RuntimeApprovalSummary, RuntimeApprovalSummaryUnavailable,
};

pub(crate) fn queue_page(
    snapshot: RuntimeApprovalQueueSnapshot,
    producer: ApprovalProducerAvailability,
) -> Result<QueryPage, String> {
    if !matches!(producer, ApprovalProducerAvailability::Available) && !snapshot.rows().is_empty() {
        return Err("approval queue has rows while the product producer is unavailable".to_owned());
    }
    let anchor = approval_queue_anchor_value(&ApprovalQueueAnchor {
        producer,
        persistence: match snapshot.persistence() {
            engine::RuntimeApprovalPersistence::Durable => ApprovalQueuePersistence::Durable,
            engine::RuntimeApprovalPersistence::Unavailable(
                engine::RuntimeApprovalPersistenceUnavailable::PendingOutcomeRouteIsProcessLocal,
            ) => ApprovalQueuePersistence::Unavailable(
                ApprovalQueuePersistenceUnavailable::PendingOutcomeRouteIsProcessLocal,
            ),
        },
    })
    .map_err(|error| format!("approval anchor: {error:?}"))?;
    let items = snapshot
        .rows()
        .iter()
        .map(project_row)
        .map(|row| {
            row.and_then(|row| {
                approval_queue_row_value(&row).map_err(|error| format!("approval row: {error:?}"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor,
        items,
        terminal: Terminal::Complete,
    })
}

pub(crate) fn decision_receipt(
    item: circular_runtime::EffectId,
    decision: RuntimeApprovalDecision,
) -> Result<Value, String> {
    let outcome = match decision {
        RuntimeApprovalDecision::Applied(ApprovalRequestOutcome::Approved(ticket)) => {
            ApprovalDecisionOutcome::Approved {
                ledger_item: key_value(ticket.ledger_item())?,
                target_effect: key_value(ticket.target_effect())?,
            }
        }
        RuntimeApprovalDecision::Applied(ApprovalRequestOutcome::Denied) => {
            ApprovalDecisionOutcome::Denied
        }
        RuntimeApprovalDecision::Rejected => {
            return Err("approval item already has a terminal decision".to_owned());
        }
        RuntimeApprovalDecision::Unknown => {
            return Err("approval item is not pending in the standing run".to_owned());
        }
    };
    approval_decision_receipt_value(&ApprovalDecisionReceipt {
        item: key_value(&item)?,
        outcome,
    })
    .map_err(|error| format!("approval decision receipt: {error:?}"))
}

pub(crate) fn actor_event_approval(
    fact: engine::effect_outcome_record::RecordedApproval,
) -> Result<(&'static str, Value), String> {
    use circular_protocol::actor_events::ApprovalDecisionKind as Kind;
    use engine::effect_outcome_record::RecordedApproval;
    let object = |fields: Vec<(&str, Value)>| {
        Value::object(fields).map_err(|error| format!("approval fact: {error:?}"))
    };
    match fact {
        RecordedApproval::Decision { item, decision } => {
            let mut fields = vec![("item", key_value(&item)?)];
            match decision {
                ApprovalRequestOutcome::Approved(ticket) => {
                    fields.push(("decision", Value::string(Kind::Approved.as_str())));
                    fields.push(("target_effect", key_value(ticket.target_effect())?));
                }
                ApprovalRequestOutcome::Denied => {
                    fields.push(("decision", Value::string(Kind::Denied.as_str())));
                }
            }
            Ok(("approval", object(fields)?))
        }
        RecordedApproval::Ticket(ticket) => Ok((
            "ticket",
            object(vec![("item", key_value(ticket.ledger_item())?)])?,
        )),
    }
}

fn project_row(row: &RuntimeApprovalRow) -> Result<ApprovalQueueRow, String> {
    Ok(ApprovalQueueRow {
        item: key_value(&row.item())?,
        emitter: emitter(row.emitter())?,
        target_effect: key_value(&row.target_effect())?,
        state: match row.state() {
            OpenApprovalState::Requested => ApprovalOpenState::Requested,
            OpenApprovalState::Approved(ticket) => ApprovalOpenState::Approved {
                ledger_item: key_value(ticket.ledger_item())?,
                target_effect: key_value(ticket.target_effect())?,
            },
        },
        summary: match row.summary() {
            RuntimeApprovalSummary::Unavailable(
                RuntimeApprovalSummaryUnavailable::ProductEffectSummaryIsUnit,
            ) => ApprovalSummaryAvailability::Unavailable(
                ApprovalSummaryUnavailable::ProductEffectSummaryIsUnit,
            ),
        },
        cause: cause(row),
    })
}

fn cause(row: &RuntimeApprovalRow) -> Option<ApprovalCause> {
    let index = row.cause()?;
    let ActorId::Scoped {
        scope,
        local: LocalKey::Named(name),
    } = row.emitter()
    else {
        return None;
    };
    Some(ApprovalCause {
        actor: circular_runtime::product_identity::wire_named_actor(
            &circular_plan::NamedActorId::new(scope.clone(), name.clone()),
        ),
        index: index.get(),
    })
}

fn emitter(actor: &ActorId) -> Result<ApprovalEmitterIdentity, String> {
    Ok(match actor {
        ActorId::Scoped { scope, local } => {
            let scope = scope
                .segments()
                .iter()
                .map(scope_segment)
                .collect::<Result<Vec<_>, _>>()?;
            match local {
                LocalKey::Named(name) => ApprovalEmitterIdentity::Named {
                    scope,
                    local: name.as_str().to_owned(),
                },
                LocalKey::Ephemeral(uuid) => ApprovalEmitterIdentity::Ephemeral {
                    scope,
                    uuid: *uuid.as_bytes(),
                },
            }
        }
        ActorId::System(SystemActor::Stream) => ApprovalEmitterIdentity::SystemStream,
        ActorId::System(SystemActor::Heartbeat) => ApprovalEmitterIdentity::SystemHeartbeat,
        ActorId::System(SystemActor::Pipeline) => {
            return Err("Pipeline is a record-only producer, not an approval emitter".into());
        }
    })
}

fn scope_segment(segment: &ScopeSeg) -> Result<WireScopeSegment, String> {
    if let ScopeSeg::Instance {
        key: InstanceKey::Tuple(values),
        ..
    } = segment
        && values.is_empty()
    {
        return Err("runtime approval emitter has an empty instance tuple".to_owned());
    }
    Ok(circular_runtime::product_identity::wire_scope_segment(
        segment,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::Tick;
    use circular_plan::{Name, ScopeId};
    use circular_protocol::approval_payload::{
        ApprovalProducerUnavailable, decode_approval_queue_anchor,
    };
    use engine::RuntimeApprovalQueue;

    #[test]
    fn producer_unavailable_is_not_an_empty_available_queue() {
        let page = queue_page(
            RuntimeApprovalQueue::new().snapshot().unwrap(),
            ApprovalProducerAvailability::Unavailable(
                ApprovalProducerUnavailable::NoRegisteredApprovalEmitter,
            ),
        )
        .unwrap();
        assert!(page.items.is_empty());
        assert!(matches!(
            decode_approval_queue_anchor(page.anchor).unwrap().producer,
            ApprovalProducerAvailability::Unavailable(
                ApprovalProducerUnavailable::NoRegisteredApprovalEmitter
            )
        ));
    }
}

fn key_value(key: &circular_runtime::EffectId) -> Result<Value, String> {
    let bytes = circular_runtime::EffectId::encode(key).map_err(|e| e.to_string())?;
    circular_core::decode(
        &bytes,
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Identity),
    )
    .map_err(|e| format!("effect key: {e:?}"))
}

/// The same boundary in the receiving direction — a decision arrives carrying
/// the carrier and is answered by the runtime, which speaks `EffectId`.
pub(crate) fn key_from_value(value: &Value) -> Result<circular_runtime::EffectId, String> {
    let bytes = circular_core::encode(
        value,
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Identity),
    )
    .map_err(|e| format!("effect key: {e:?}"))?;
    circular_runtime::EffectId::decode(&bytes).map_err(|e| e.to_string())
}
