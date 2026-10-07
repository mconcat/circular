//! Typed protocol vocabulary for a recorded dead-letter reason.
//!
//! Runtime owns the six semantic arms. This module owns their published spellings and the
//! value boundary that preserves the two arm arguments. A consumer must decode through the
//! closed match below: an unknown spelling is an error, not a record to skip.

use circular_core::Value;

use crate::wire_value::{WireError, exhausted, object_from_value, take};

circular_core::closed_table! {
    /// The six published dead-letter reason spellings — **the one declaration**.
    ///
    /// Both the runtime's typed reason and this protocol carrier name their arm through this
    /// table, so encode (`as_str`) and decode (`from_str`) come from the same line. Declaration
    /// order is the runtime enum's arm order.
    pub enum DeadLetterReasonKind {
        Processing => "processing",
        OutcomeUnclaimed => "outcome_unclaimed",
        DestinationGone => "destination_gone",
        Poisoned => "poisoned",
        ActorDeclared => "actor_declared",
        Capacity => "capacity",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeadLetterReason {
    Processing(Value),
    OutcomeUnclaimed,
    DestinationGone,
    Poisoned,
    ActorDeclared(String),
    Capacity,
}

impl DeadLetterReason {
    /// This reason's arm in the one spelling table.
    #[must_use]
    pub const fn kind(&self) -> DeadLetterReasonKind {
        match self {
            Self::Processing(_) => DeadLetterReasonKind::Processing,
            Self::OutcomeUnclaimed => DeadLetterReasonKind::OutcomeUnclaimed,
            Self::DestinationGone => DeadLetterReasonKind::DestinationGone,
            Self::Poisoned => DeadLetterReasonKind::Poisoned,
            Self::ActorDeclared(_) => DeadLetterReasonKind::ActorDeclared,
            Self::Capacity => DeadLetterReasonKind::Capacity,
        }
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        self.kind().as_str()
    }

    /// Encode one reason as `{ code, detail }`, matching the protocol-owned reason shape used by
    /// `actor.events`. Unit arms carry `Null`; argument-bearing arms retain their argument in
    /// `detail`.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let detail = match self {
            Self::Processing(cause) => cause.clone(),
            Self::OutcomeUnclaimed | Self::DestinationGone | Self::Poisoned | Self::Capacity => {
                Value::Null
            }
            Self::ActorDeclared(name) => Value::String(name.clone()),
        };
        Value::object([
            ("code", Value::String(self.as_str().to_owned())),
            ("detail", detail),
        ])
        .expect("dead-letter reason fields are distinct")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeadLetterReasonRejection {
    /// The reason object's structure — shared with every object decoder.
    Wire(WireError),
    CodeIsNotText,
    UnknownReason(String),
    UnexpectedDetail(&'static str),
    ActorDeclaredNameIsNotText,
}

/// Decode exactly one of the six published reasons.
///
/// Unknown spellings fail closed. In particular, a future arm cannot disappear from an
/// older client's failure list by being treated as an ignorable value.
pub fn decode_dead_letter_reason(
    value: Value,
) -> Result<DeadLetterReason, DeadLetterReasonRejection> {
    let mut fields = object_from_value(value)?;
    let code = match take(&mut fields, "code")? {
        Value::String(code) => code,
        _ => return Err(DeadLetterReasonRejection::CodeIsNotText),
    };
    let kind = DeadLetterReasonKind::from_str(&code)
        .ok_or(DeadLetterReasonRejection::UnknownReason(code))?;
    let detail = take(&mut fields, "detail")?;
    exhausted(fields)?;
    match kind {
        DeadLetterReasonKind::Processing => Ok(DeadLetterReason::Processing(detail)),
        DeadLetterReasonKind::OutcomeUnclaimed => {
            unit_reason(detail, DeadLetterReason::OutcomeUnclaimed)
        }
        DeadLetterReasonKind::DestinationGone => {
            unit_reason(detail, DeadLetterReason::DestinationGone)
        }
        DeadLetterReasonKind::Poisoned => unit_reason(detail, DeadLetterReason::Poisoned),
        DeadLetterReasonKind::Capacity => unit_reason(detail, DeadLetterReason::Capacity),
        DeadLetterReasonKind::ActorDeclared => match detail {
            Value::String(name) => Ok(DeadLetterReason::ActorDeclared(name)),
            _ => Err(DeadLetterReasonRejection::ActorDeclaredNameIsNotText),
        },
    }
}

impl From<WireError> for DeadLetterReasonRejection {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

fn unit_reason(
    detail: Value,
    reason: DeadLetterReason,
) -> Result<DeadLetterReason, DeadLetterReasonRejection> {
    if detail == Value::Null {
        Ok(reason)
    } else {
        Err(DeadLetterReasonRejection::UnexpectedDetail(reason.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_six_reasons_round_trip_and_actor_declared_keeps_its_name() {
        let reasons = [
            DeadLetterReason::Processing(Value::array([Value::uint(4)])),
            DeadLetterReason::OutcomeUnclaimed,
            DeadLetterReason::DestinationGone,
            DeadLetterReason::Poisoned,
            DeadLetterReason::ActorDeclared("transform_failed".to_owned()),
            DeadLetterReason::Capacity,
        ];
        let spellings = [
            "processing",
            "outcome_unclaimed",
            "destination_gone",
            "poisoned",
            "actor_declared",
            "capacity",
        ];
        assert_eq!(reasons.each_ref().map(DeadLetterReason::as_str), spellings);
        assert_eq!(
            DeadLetterReasonKind::ALL.map(DeadLetterReasonKind::as_str),
            spellings
        );
        for reason in reasons {
            let value = reason.to_value();
            assert_eq!(decode_dead_letter_reason(value), Ok(reason));
        }

        let first = DeadLetterReason::ActorDeclared("transform_failed".to_owned()).to_value();
        let second =
            DeadLetterReason::ActorDeclared("output_shape_unresolved".to_owned()).to_value();
        assert_ne!(first, second, "ActorDeclared's argument was discarded");
    }

    #[test]
    fn structural_violations_are_the_shared_wire_error() {
        let extra = Value::object([
            ("code", Value::String("poisoned".to_owned())),
            ("detail", Value::Null),
            ("extra", Value::Null),
        ])
        .expect("reason fields");
        assert_eq!(
            decode_dead_letter_reason(extra),
            Err(DeadLetterReasonRejection::Wire(WireError::unknown(
                "extra".to_owned()
            )))
        );
        let missing =
            Value::object([("code", Value::String("poisoned".to_owned()))]).expect("reason fields");
        assert_eq!(
            decode_dead_letter_reason(missing),
            Err(DeadLetterReasonRejection::Wire(WireError::missing(
                "detail"
            )))
        );
        assert_eq!(
            decode_dead_letter_reason(Value::Null),
            Err(DeadLetterReasonRejection::Wire(WireError::not_object(None)))
        );
    }

    #[test]
    fn unknown_reason_spelling_fails_closed() {
        let value = Value::object([
            ("code", Value::String("future_reason".to_owned())),
            ("detail", Value::Null),
        ])
        .expect("reason fields");
        assert_eq!(
            decode_dead_letter_reason(value),
            Err(DeadLetterReasonRejection::UnknownReason(
                "future_reason".to_owned()
            ))
        );
    }
}
