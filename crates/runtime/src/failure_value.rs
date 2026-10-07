//! Existing dead-letter Value projection, shared by recording and match output.
use circular_core::Value;
use circular_protocol::dead_letter::DeadLetterReason as PublishedDeadLetterReason;

pub fn dead_letter_reason_value(reason: &crate::DeadLetterReason) -> Value {
    dead_letter_reason_with_failure_point(reason, None)
}

pub fn dead_letter_reason_with_failure_point(
    reason: &crate::DeadLetterReason,
    point: Option<&crate::PreprocessFailurePoint>,
) -> Value {
    use crate::DeadLetterReason;
    let published = match reason {
        DeadLetterReason::Processing(cause) => {
            let mut detail = processing_cause_value(cause);
            if let Some(point) = point {
                let Value::Array(ref mut items) = detail else {
                    unreachable!("processing cause tuple")
                };
                let mut meta = vec![("failure_point", failure_point_value(point))];
                if let Some(code) = &point.code {
                    meta.push(("code", Value::string(code.as_str())));
                }
                items.push(Value::object(meta).expect("distinct optional detail fields"));
            }
            PublishedDeadLetterReason::Processing(detail)
        }
        DeadLetterReason::OutcomeUnclaimed => PublishedDeadLetterReason::OutcomeUnclaimed,
        DeadLetterReason::DestinationGone => PublishedDeadLetterReason::DestinationGone,
        DeadLetterReason::Capacity => PublishedDeadLetterReason::Capacity,
        DeadLetterReason::Poisoned => PublishedDeadLetterReason::Poisoned,
        DeadLetterReason::ActorDeclared(reason) => {
            PublishedDeadLetterReason::ActorDeclared(reason.name().to_owned())
        }
    };
    published.to_value()
}

fn failure_point_value(point: &crate::PreprocessFailurePoint) -> Value {
    use crate::product_identity::edge_value;
    let kind = point.kind.as_str();
    Value::object([
        (
            "edge",
            edge_value(&point.edge).expect("admitted edge identity"),
        ),
        (
            "step",
            Value::object([
                (
                    "index",
                    Value::uint(u64::try_from(point.index).expect("chain index fits u64")),
                ),
                ("kind", Value::string(kind)),
            ])
            .expect("step fields are distinct"),
        ),
    ])
    .expect("failure point fields are distinct")
}

fn processing_cause_value(cause: &crate::ProcessingCause) -> Value {
    use crate::ProcessingCause;
    match cause {
        ProcessingCause::EffectFailed(failure) => {
            Value::array([Value::uint(1), effect_failure_value(failure)])
        }
        ProcessingCause::OutcomeUnclaimed => Value::array([Value::uint(2)]),
        ProcessingCause::InputOutOfDomain => Value::array([Value::uint(3)]),
        ProcessingCause::DomainRejected => Value::array([Value::uint(4)]),
        ProcessingCause::Poisoned => Value::array([Value::uint(5)]),
        ProcessingCause::ApprovalRequired => Value::array([Value::uint(6)]),
    }
}

fn effect_failure_value(failure: &crate::EffectFailure) -> Value {
    Value::array([
        Value::string(failure.kind_tag()),
        effect_failure_detail(failure),
    ])
}

pub fn effect_failure_detail(failure: &crate::EffectFailure) -> Value {
    use crate::EffectFailure;
    match failure {
        EffectFailure::ParameterDenied { capability } => {
            let tag = crate::Capability::ALL
                .iter()
                .position(|candidate| candidate == capability)
                .and_then(|index| u64::try_from(index).ok())
                .and_then(|index| index.checked_add(1))
                .expect("the closed capability table fits u64");
            Value::uint(tag)
        }
        EffectFailure::Diverged(divergence) => Value::uint(u64::from(divergence.tag())),
        EffectFailure::Peer(kind) => Value::uint(u64::from(kind.tag())),
        EffectFailure::InterpreterFault(fault) => Value::uint(u64::from(fault.tag())),
        EffectFailure::RetryExhausted { attempts } => Value::uint(u64::from(*attempts)),
        EffectFailure::TransportTerminal
        | EffectFailure::TransportUnreached
        | EffectFailure::RemoteDeferred
        | EffectFailure::ApprovalRequired
        | EffectFailure::EndpointGone => Value::Null,
    }
}

/// Inverse of the existing reason projection. Exact re-encoding rejects extra fields.
pub fn decode_runtime_reason(
    value: Value,
) -> Option<(
    crate::DeadLetterReason,
    Option<crate::PreprocessFailurePoint>,
)> {
    use crate::{DeadLetterReason as R, ProcessingCause as C};
    let published =
        circular_protocol::dead_letter::decode_dead_letter_reason(value.clone()).ok()?;
    let mut point = None;
    let reason = match published {
        PublishedDeadLetterReason::Processing(Value::Array(mut fields)) => {
            if let Some(Value::Object(meta)) = fields.last()
                && let Some(v) = meta.get("failure_point")
            {
                let obj = v.as_object()?;
                let step = obj.get("step")?.as_object()?;
                let Value::UInt(index) = step.get("index")? else {
                    return None;
                };
                let kind = crate::PreprocessKind::from_wire(step.get("kind")?.as_str()?)?;
                let code = match meta.get("code") {
                    None => None,
                    Some(Value::String(code)) => Some(code.clone()),
                    Some(_) => return None,
                };
                point = Some(crate::PreprocessFailurePoint {
                    edge: crate::product_identity::edge_from_value(obj.get("edge")?).ok()?,
                    index: usize::try_from(*index).ok()?,
                    kind,
                    code,
                });
                fields.pop();
            }
            let cause = match fields.as_slice() {
                [Value::UInt(1), failure] => C::EffectFailed(decode_effect_failure_value(failure)?),
                [Value::UInt(2)] => C::OutcomeUnclaimed,
                [Value::UInt(3)] => C::InputOutOfDomain,
                [Value::UInt(4)] => C::DomainRejected,
                [Value::UInt(5)] => C::Poisoned,
                [Value::UInt(6)] => C::ApprovalRequired,
                _ => return None,
            };
            R::Processing(cause)
        }
        PublishedDeadLetterReason::Processing(_) => return None,
        PublishedDeadLetterReason::OutcomeUnclaimed => R::OutcomeUnclaimed,
        PublishedDeadLetterReason::DestinationGone => R::DestinationGone,
        PublishedDeadLetterReason::Poisoned => R::Poisoned,
        PublishedDeadLetterReason::Capacity => R::Capacity,
        PublishedDeadLetterReason::ActorDeclared(name) => {
            let decl =
                crate::ReasonDecl::<crate::DeadLettering>::try_from_names([name.as_str()]).ok()?;
            R::ActorDeclared(decl.resolve(&name)?)
        }
    };
    (dead_letter_reason_with_failure_point(&reason, point.as_ref()) == value)
        .then_some((reason, point))
}

fn decode_effect_failure_value(value: &Value) -> Option<crate::EffectFailure> {
    use crate::outcome::EffectFailureKind as K;
    use crate::{Divergence, EffectFailure as F, InterpreterFault, PeerFailureKind};
    let Value::Array(parts) = value else {
        return None;
    };
    let [Value::String(tag), detail] = parts.as_slice() else {
        return None;
    };
    let small_tag = || match detail {
        Value::UInt(tag) => u8::try_from(*tag).ok(),
        _ => None,
    };
    let Some(kind) = K::from_str(tag) else {
        let kind = PeerFailureKind::from_str(tag)?;
        return (small_tag()? == kind.tag()).then_some(F::Peer(kind));
    };
    match kind {
        K::ParameterDenied => {
            let Value::UInt(position) = detail else {
                return None;
            };
            let index = usize::try_from(position.checked_sub(1)?).ok()?;
            crate::Capability::ALL
                .get(index)
                .map(|capability| F::ParameterDenied {
                    capability: *capability,
                })
        }
        K::TransportTerminal => (*detail == Value::Null).then_some(F::TransportTerminal),
        K::TransportUnreached => (*detail == Value::Null).then_some(F::TransportUnreached),
        K::RemoteDeferred => (*detail == Value::Null).then_some(F::RemoteDeferred),
        K::ApprovalRequired => (*detail == Value::Null).then_some(F::ApprovalRequired),
        K::EndpointGone => (*detail == Value::Null).then_some(F::EndpointGone),
        K::Diverged => Divergence::from_tag(small_tag()?).map(F::Diverged),
        K::InterpreterFault => InterpreterFault::from_tag(small_tag()?).map(F::InterpreterFault),
        K::RetryExhausted => match detail {
            Value::UInt(attempts) => u32::try_from(*attempts)
                .ok()
                .map(|attempts| F::RetryExhausted { attempts }),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Capability, DeadLetterReason, Divergence, EffectFailure, InterpreterFault, PeerFailureKind,
        ProcessingCause,
    };

    fn unwired(failure: EffectFailure) -> DeadLetterReason {
        DeadLetterReason::Processing(ProcessingCause::EffectFailed(failure))
    }

    #[test]
    fn every_effect_failure_arm_reads_back_through_the_direct_decoder() {
        let mut failures = vec![
            EffectFailure::TransportTerminal,
            EffectFailure::TransportUnreached,
            EffectFailure::RemoteDeferred,
            EffectFailure::ApprovalRequired,
            EffectFailure::EndpointGone,
        ];
        failures.extend(Divergence::ALL.map(EffectFailure::Diverged));
        failures.extend(
            Capability::ALL
                .iter()
                .map(|capability| EffectFailure::ParameterDenied {
                    capability: *capability,
                }),
        );
        failures.extend(InterpreterFault::ALL.map(EffectFailure::InterpreterFault));
        failures.extend(PeerFailureKind::ALL.map(EffectFailure::Peer));
        failures.push(EffectFailure::RetryExhausted { attempts: 10 });
        for failure in failures {
            let reason = unwired(failure);
            let value = dead_letter_reason_value(&reason);
            assert_eq!(decode_runtime_reason(value), Some((reason, None)));
        }
    }

    #[test]
    fn a_detail_that_disagrees_with_its_spelling_is_refused() {
        let reason = |detail: Value| {
            Value::object([
                ("code", Value::string("processing")),
                ("detail", Value::array([Value::uint(1), detail])),
            ])
            .unwrap()
        };
        for detail in [
            Value::array([Value::string("peer_name_conflict"), Value::uint(13)]),
            Value::array([Value::string("endpoint_gone"), Value::uint(1)]),
            Value::array([Value::string("parameter_denied"), Value::uint(0)]),
            Value::array([Value::string("diverged"), Value::uint(99)]),
            Value::array([Value::string("retry_exhausted"), Value::Null]),
            Value::array([Value::string("future_failure"), Value::Null]),
        ] {
            assert_eq!(decode_runtime_reason(reason(detail)), None);
        }
    }
}
