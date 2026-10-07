//! Canonical payloads for the stable lifecycle partition.
//!
//! The request verb is carried by the envelope. Payloads therefore contain
//! only verb-owned fields, while results carry a closed accepted arm so a
//! correlation cannot be reconciled to the wrong lifecycle transition.

use circular_core::{Ceilings, CodecError, Value, decode, encode};

use crate::declaration_payload::Rejected;
use crate::wire_value::{arm, decode_arm, unit_arm};
use crate::{Lifecycle, LifecycleAccepted, LifecycleDomain, PauseMode};

/// Concrete wire-domain identities for lifecycle requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireLifecycleDomain;

impl LifecycleDomain for WireLifecycleDomain {
    type AuthoringRevision = Vec<u8>;
}

/// A decoded stable lifecycle request.
pub type LifecycleRequest = Lifecycle<WireLifecycleDomain>;

/// A complete lifecycle response.
#[derive(Clone, Debug, PartialEq)]
pub enum LifecycleResult {
    Accepted(LifecycleAccepted),
    Rejected(Rejected),
}

/// Structural decoding failures. Semantic rejection remains a normal result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecyclePayloadRejection {
    MalformedValue,
    NotObject,
    MissingField(&'static str),
    UnknownField(String),
    InvalidAuthoringRevision,
    InvalidResult,
}

impl LifecycleRequest {
    /// Encodes only the fields owned by the request's envelope verb.
    pub fn encode(&self, ceilings: Ceilings) -> Result<Vec<u8>, CodecError> {
        let value = match self {
            Self::Resume {
                expected_authoring_revision,
            } => Value::object([(
                "expected_authoring_revision",
                Value::Bytes(expected_authoring_revision.clone()),
            )])
            .expect("one canonical field"),
            Self::Pause { mode } => {
                let mut fields = vec![];
                if let Some(mode) = mode {
                    fields.push((
                        "mode",
                        unit_arm(match mode {
                            PauseMode::Pause => 1,
                            PauseMode::ForcePause => 2,
                        }),
                    ));
                }
                Value::object(fields).expect("distinct canonical fields")
            }
        };
        encode(&value, ceilings)
    }
}

/// Decodes a `Resume` payload. The expected revision is exactly 32 bytes.
pub fn decode_resume(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<LifecycleRequest, LifecyclePayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let revision = match fields.remove("expected_authoring_revision") {
        Some(Value::Bytes(revision)) if revision.len() == 32 => revision,
        Some(_) => return Err(LifecyclePayloadRejection::InvalidAuthoringRevision),
        None => {
            return Err(LifecyclePayloadRejection::MissingField(
                "expected_authoring_revision",
            ));
        }
    };
    exhausted(fields)?;
    Ok(Lifecycle::Resume {
        expected_authoring_revision: revision,
    })
}

/// Decodes a `Pause` payload for the pipeline served by this state.
pub fn decode_pause(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<LifecycleRequest, LifecyclePayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let mode = fields
        .remove("mode")
        .map(|value| {
            let (tag, arguments) =
                decode_arm(value, "mode").map_err(|_| LifecyclePayloadRejection::MalformedValue)?;
            if !arguments.is_empty() {
                return Err(LifecyclePayloadRejection::MalformedValue);
            }
            match tag {
                1 => Ok(PauseMode::Pause),
                2 => Ok(PauseMode::ForcePause),
                _ => Err(LifecyclePayloadRejection::MalformedValue),
            }
        })
        .transpose()?;
    exhausted(fields)?;
    Ok(Lifecycle::Pause { mode })
}

impl LifecycleResult {
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Accepted(LifecycleAccepted::Resumed) => arm(1, unit_arm(1)),
            Self::Accepted(LifecycleAccepted::Paused) => arm(1, unit_arm(2)),
            Self::Rejected(rejection) => {
                crate::declaration_payload::CommandResult::Rejected(rejection.clone()).to_value()
            }
        }
    }

    pub fn encode(&self, ceilings: Ceilings) -> Result<Vec<u8>, CodecError> {
        encode(&self.to_value(), ceilings)
    }

    pub fn decode(bytes: &[u8], ceilings: Ceilings) -> Result<Self, LifecyclePayloadRejection> {
        let value =
            decode(bytes, ceilings).map_err(|_| LifecyclePayloadRejection::MalformedValue)?;
        let (tag, mut arguments) =
            decode_arm(value, "result").map_err(|_| LifecyclePayloadRejection::InvalidResult)?;
        if arguments.len() != 1 {
            return Err(LifecyclePayloadRejection::InvalidResult);
        }
        let payload = arguments.pop().expect("one argument");
        match tag {
            1 => decode_accepted(payload),
            2 => decode_rejected(payload).map(Self::Rejected),
            _ => Err(LifecyclePayloadRejection::InvalidResult),
        }
    }
}

fn decode_accepted(value: Value) -> Result<LifecycleResult, LifecyclePayloadRejection> {
    let (tag, arguments) =
        decode_arm(value, "accepted").map_err(|_| LifecyclePayloadRejection::InvalidResult)?;
    if !arguments.is_empty() {
        return Err(LifecyclePayloadRejection::InvalidResult);
    }
    match tag {
        1 => Ok(LifecycleResult::Accepted(LifecycleAccepted::Resumed)),
        2 => Ok(LifecycleResult::Accepted(LifecycleAccepted::Paused)),
        _ => Err(LifecyclePayloadRejection::InvalidResult),
    }
}

fn decode_rejected(value: Value) -> Result<Rejected, LifecyclePayloadRejection> {
    let Value::Object(object) = value else {
        return Err(LifecyclePayloadRejection::InvalidResult);
    };
    let mut fields = object.into_map();
    let code = match fields.remove("code") {
        Some(Value::Int(code)) if code >= 0 => {
            u32::try_from(code).map_err(|_| LifecyclePayloadRejection::InvalidResult)?
        }
        _ => return Err(LifecyclePayloadRejection::InvalidResult),
    };
    let message = match fields.remove("message") {
        Some(Value::String(message)) => message,
        _ => return Err(LifecyclePayloadRejection::InvalidResult),
    };
    let hint = match fields.remove("hint") {
        Some(Value::String(hint)) => Some(hint),
        Some(_) => return Err(LifecyclePayloadRejection::InvalidResult),
        None => None,
    };
    let at = fields.remove("at");
    exhausted(fields)?;
    Ok(Rejected {
        code,
        message,
        hint,
        at,
    })
}

fn object(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<std::collections::BTreeMap<String, Value>, LifecyclePayloadRejection> {
    let decoded = decode(bytes, ceilings).map_err(|_| LifecyclePayloadRejection::MalformedValue)?;
    let Value::Object(object) = decoded else {
        return Err(LifecyclePayloadRejection::NotObject);
    };
    Ok(object.into_map())
}

fn exhausted(
    fields: std::collections::BTreeMap<String, Value>,
) -> Result<(), LifecyclePayloadRejection> {
    if let Some((field, _)) = fields.into_iter().next() {
        Err(LifecyclePayloadRejection::UnknownField(field))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use circular_core::Boundary;

    const CEILINGS: Ceilings = Ceilings::for_boundary(Boundary::Wire);

    #[test]
    fn pause_wire_has_only_the_optional_mode_and_rejects_old_run_fields() {
        let cases = [
            (None, vec![8, 0, 0, 0, 0]),
            (
                Some(PauseMode::Pause),
                vec![
                    8, 0, 0, 0, 1, 0, 0, 0, 4, b'm', b'o', b'd', b'e', 3, 0, 0, 0, 0, 0, 0, 0, 1,
                ],
            ),
            (
                Some(PauseMode::ForcePause),
                vec![
                    8, 0, 0, 0, 1, 0, 0, 0, 4, b'm', b'o', b'd', b'e', 3, 0, 0, 0, 0, 0, 0, 0, 2,
                ],
            ),
        ];
        for (mode, bytes) in cases {
            assert_eq!(
                LifecycleRequest::Pause { mode }.encode(CEILINGS).unwrap(),
                bytes
            );
            assert_eq!(
                decode_pause(&bytes, CEILINGS).unwrap(),
                LifecycleRequest::Pause { mode }
            );
        }
        let old = [
            8, 0, 0, 0, 1, 0, 0, 0, 3, b'r', b'u', b'n', 3, 0, 0, 0, 0, 0, 0, 0, 41,
        ];
        assert_eq!(
            decode_pause(&old, CEILINGS),
            Err(LifecyclePayloadRejection::UnknownField("run".into()))
        );
    }

    #[test]
    fn lifecycle_result_uses_unit_arms_and_rejects_old_run_results() {
        for (tag, accepted) in [
            (1, LifecycleAccepted::Resumed),
            (2, LifecycleAccepted::Paused),
        ] {
            let bytes = [
                7, 0, 0, 0, 2, 3, 0, 0, 0, 0, 0, 0, 0, 1, 3, 0, 0, 0, 0, 0, 0, 0, tag,
            ];
            let result = LifecycleResult::Accepted(accepted);
            assert_eq!(result.encode(CEILINGS).unwrap(), bytes);
            assert_eq!(LifecycleResult::decode(&bytes, CEILINGS).unwrap(), result);
            let old = encode(
                &Value::array([
                    Value::Int(1),
                    Value::array([Value::Int(i64::from(tag)), Value::Int(41)]),
                ]),
                CEILINGS,
            )
            .unwrap();
            assert_eq!(
                LifecycleResult::decode(&old, CEILINGS),
                Err(LifecyclePayloadRejection::InvalidResult)
            );
        }
    }

    #[test]
    fn request_variant_owns_the_matching_stable_verb() {
        assert_eq!(
            LifecycleRequest::Resume {
                expected_authoring_revision: vec![1; 32],
            }
            .verb(),
            crate::LifecycleVerb::Resume
        );
        assert_eq!(
            LifecycleRequest::Pause { mode: None }.verb(),
            crate::LifecycleVerb::Pause
        );
    }
}
