
use std::error::Error;
use std::fmt;

use circular_core::Value;

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum MoveToScopeRejection {
        EmptyMoveSet => "empty_move_set",
        SourceUnresolved => "source_unresolved",
        TargetScopeUnresolved => "target_scope_unresolved",
        TargetScopeIsTemplate => "target_scope_is_template",
        LocalCollisionInTarget => "local_collision_in_target",
        BoundarySynthesisRefused => "boundary_synthesis_refused",
        BoundaryActorImmovable => "boundary_actor_immovable",
        WouldNestIntoSelf => "would_nest_into_self",
        ReservedLocalSpelling => "reserved_local_spelling",
    }
}

pub fn decode_move_to_scope_rejection(
    value: &Value,
) -> Result<MoveToScopeRejection, MoveToScopeRejectionCodecError> {
    let Value::String(spelling) = value else {
        return Err(MoveToScopeRejectionCodecError::WrongCarrier);
    };
    MoveToScopeRejection::from_str(spelling).ok_or(MoveToScopeRejectionCodecError::UnknownSpelling)
}

#[must_use]
pub fn encode_move_to_scope_rejection(reason: MoveToScopeRejection) -> Value {
    Value::String(reason.as_str().to_owned())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveToScopeRejectionCodecError {
    WrongCarrier,
    UnknownSpelling,
}

impl fmt::Display for MoveToScopeRejectionCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongCarrier => formatter.write_str("move-to-scope rejection is not a String"),
            Self::UnknownSpelling => {
                formatter.write_str("unknown move-to-scope rejection spelling")
            }
        }
    }
}

impl Error for MoveToScopeRejectionCodecError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_move_to_scope_rejection_has_a_unique_spelling() {
        let mut spellings = BTreeSet::new();
        for reason in MoveToScopeRejection::ALL {
            assert!(spellings.insert(reason.as_str()));
        }
    }

    #[test]
    fn every_move_to_scope_rejection_round_trips() {
        for reason in MoveToScopeRejection::ALL {
            let encoded = encode_move_to_scope_rejection(reason);
            assert_eq!(decode_move_to_scope_rejection(&encoded), Ok(reason));
        }
    }

    #[test]
    fn unknown_spellings_and_wrong_carriers_fail_closed() {
        assert_eq!(
            decode_move_to_scope_rejection(&Value::String("other".to_owned())),
            Err(MoveToScopeRejectionCodecError::UnknownSpelling)
        );
        assert_eq!(
            decode_move_to_scope_rejection(&Value::Int(1)),
            Err(MoveToScopeRejectionCodecError::WrongCarrier)
        );
    }
}
