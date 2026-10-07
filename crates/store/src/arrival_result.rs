use crate::RecordCodecError;
use circular_core::{Boundary, Ceilings, Value};
use circular_runtime::EnvelopeResult;

pub(crate) fn encode(result: &EnvelopeResult) -> Result<Vec<u8>, RecordCodecError> {
    let value = match result {
        EnvelopeResult::Ok => Value::array([Value::UInt(1)]),
        EnvelopeResult::Err {
            reason,
            failure_point,
        } => {
            if failure_point.is_some()
                && !matches!(reason, circular_runtime::DeadLetterReason::Processing(_))
            {
                return Err(RecordCodecError::LengthOutOfRange);
            }
            Value::array([
                Value::UInt(2),
                circular_runtime::failure_value::dead_letter_reason_with_failure_point(
                    reason,
                    failure_point.as_ref(),
                ),
            ])
        }
    };
    circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal))
        .map_err(|_| RecordCodecError::LengthOutOfRange)
}

pub(crate) fn decode(bytes: &[u8]) -> Option<EnvelopeResult> {
    let Value::Array(fields) =
        circular_core::decode(bytes, Ceilings::for_boundary(Boundary::Journal)).ok()?
    else {
        return None;
    };
    let result = match fields.as_slice() {
        [Value::UInt(1)] => EnvelopeResult::Ok,
        [Value::UInt(2), value] => {
            let (reason, failure_point) =
                circular_runtime::failure_value::decode_runtime_reason(value.clone())?;
            EnvelopeResult::Err {
                reason,
                failure_point,
            }
        }
        _ => return None,
    };
    (encode(&result).ok()?.as_slice() == bytes).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_runtime::{DeadLetterReason, ProcessingCause};
    #[test]
    fn arrival_result_preserves_existing_reason_domain_and_failure_point() {
        use circular_runtime::{EdgeId, Name, NamedActorId, ScopeId};
        let actor = NamedActorId::new(ScopeId::root(), Name::from_normalized("receiver"));
        let point = circular_runtime::PreprocessFailurePoint {
            edge: EdgeId::Outcome { target: actor },
            index: 2,
            kind: circular_runtime::PreprocessKind::Map,
            code: None,
        };
        let declared =
            circular_runtime::ReasonDecl::<circular_runtime::DeadLettering>::try_from_names([
                "missing",
            ])
            .unwrap()
            .resolve("missing")
            .unwrap();
        for reason in [
            DeadLetterReason::OutcomeUnclaimed,
            DeadLetterReason::DestinationGone,
            DeadLetterReason::Capacity,
            DeadLetterReason::Poisoned,
            DeadLetterReason::ActorDeclared(declared),
        ] {
            let result = EnvelopeResult::Err {
                reason: reason.clone(),
                failure_point: None,
            };
            assert_eq!(decode(&encode(&result).unwrap()), Some(result));
            assert!(
                encode(&EnvelopeResult::Err {
                    reason,
                    failure_point: Some(point.clone())
                })
                .is_err()
            );
        }
        for cause in [
            ProcessingCause::EffectFailed(circular_runtime::EffectFailure::EndpointGone),
            ProcessingCause::OutcomeUnclaimed,
            ProcessingCause::InputOutOfDomain,
            ProcessingCause::DomainRejected,
            ProcessingCause::Poisoned,
            ProcessingCause::ApprovalRequired,
        ] {
            let result = EnvelopeResult::Err {
                reason: DeadLetterReason::Processing(cause),
                failure_point: Some(point.clone()),
            };
            assert_eq!(decode(&encode(&result).unwrap()), Some(result));
        }
        let value = Value::array([
            Value::UInt(2),
            Value::object([
                ("code", Value::string("processing")),
                ("detail", Value::array([Value::UInt(3)])),
            ])
            .unwrap(),
        ]);
        assert_eq!(
            decode(
                &circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal)).unwrap()
            ),
            Some(EnvelopeResult::Err {
                reason: DeadLetterReason::Processing(ProcessingCause::InputOutOfDomain),
                failure_point: None
            })
        );
    }

    #[test]
    fn arrival_result_round_trip_rejects_unknown_truncated_and_missing_tags() {
        for result in [
            EnvelopeResult::Ok,
            EnvelopeResult::Err {
                reason: DeadLetterReason::Processing(ProcessingCause::InputOutOfDomain),
                failure_point: None,
            },
        ] {
            let bytes = encode(&result).unwrap();
            assert_eq!(decode(&bytes), Some(result));
            for end in 0..bytes.len() {
                assert_eq!(decode(&bytes[..end]), None);
            }
        }
        for value in [
            Value::array([]),
            Value::array([Value::UInt(0)]),
            Value::array([Value::UInt(3)]),
            Value::array([Value::UInt(2)]),
            Value::array([Value::UInt(1), Value::Null]),
        ] {
            assert_eq!(
                decode(
                    &circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal))
                        .unwrap()
                ),
                None
            );
        }
    }
}
