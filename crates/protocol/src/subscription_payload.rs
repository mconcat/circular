
use circular_core::{Ceilings, ObjectValue, Value, encode};

use crate::declaration_payload::environment::{AuthoringEnvironment, decode_environment};
use crate::wire_value::{
    PayloadRejection, arm, bytes_of, decode_arm, exhausted, object, object_fields, optional, take,
    text_of, unit_arm, unsigned_of,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Subscribe {
    pub target: String,
    pub args: Value,
    pub lens: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Credit {
    pub frames: u32,
}

circular_core::closed_table! {
    pub enum FrameOrigin: i64 {
        Retained = 1,
        Live = 2,
    }
}

circular_core::closed_table! {
    pub enum SubscriptionFrameArm: i64 {
        Lossless = 1,
        Conflated = 2,
        Credit = 3,
        RetentionComplete = 4,
    }
}

circular_core::closed_table! {
    pub enum SubscriptionEndReasonArm: i64 {
        ByClient = 1,
        ConsumerBehind = 2,
        TargetGone = 3,
        Withdrawn = 4,
        SessionClosed = 5,
        ResetRequired = 6,
        ScopeGone = 7,
        IncompatibleClient = 8,
        Complete = 9,
    }
}

pub type SubscriptionFrame = crate::message::SubscriptionFrame<Value, Value>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionEndReason {
    ByClient,
    ConsumerBehind,
    TargetGone,
    Withdrawn,
    SessionClosed,
    ResetRequired { floor_or_cursor: Vec<u8> },
    ScopeGone { cursor: Vec<u8> },
    IncompatibleClient {
        required_environment: AuthoringEnvironment,
    },
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionEnded {
    pub reason: SubscriptionEndReason,
    pub code: u32,
    pub anchor: Vec<u8>,
}

fn bounded_u32(value: Value, key: &'static str) -> Result<u32, PayloadRejection> {
    u32::try_from(unsigned_of(value, key)?).map_err(|_| PayloadRejection::WrongCarrier { key })
}

pub fn decode_subscribe(bytes: &[u8], ceilings: Ceilings) -> Result<Subscribe, PayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let args = take(&mut fields, "args")?;
    let target = text_of(take(&mut fields, "target")?, "target")?;
    let lens = optional(&mut fields, "lens")
        .map(|lens| bounded_u32(lens, "lens"))
        .transpose()?;
    exhausted(fields)?;
    Ok(Subscribe { target, args, lens })
}

pub fn decode_credit(bytes: &[u8], ceilings: Ceilings) -> Result<Credit, PayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let frames = bounded_u32(take(&mut fields, "frames")?, "frames")?;
    exhausted(fields)?;
    if frames == 0 {
        return Err(PayloadRejection::WrongCarrier { key: "frames" });
    }
    Ok(Credit { frames })
}

impl FrameOrigin {
    fn decode(value: Value) -> Result<Self, PayloadRejection> {
        let (tag, arguments) = decode_arm(value, "origin")?;
        if !arguments.is_empty() {
            return Err(PayloadRejection::WrongCarrier { key: "origin" });
        }
        Self::from_tag(tag).ok_or(PayloadRejection::UnknownArm { tag })
    }
}

impl<S, P> crate::message::SubscriptionFrame<S, P> {
    #[must_use]
    pub const fn arm(&self) -> SubscriptionFrameArm {
        match self {
            Self::Lossless { .. } => SubscriptionFrameArm::Lossless,
            Self::Conflated { .. } => SubscriptionFrameArm::Conflated,
            Self::Credit { .. } => SubscriptionFrameArm::Credit,
            Self::RetentionComplete { .. } => SubscriptionFrameArm::RetentionComplete,
        }
    }
}

impl SubscriptionFrame {
    #[must_use]
    pub fn to_value(&self) -> Value {
        let body = |entries: Vec<(&str, Value)>| {
            Value::Object(
                ObjectValue::try_from_entries(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value)),
                )
                .expect("frame keys differ"),
            )
        };
        match self {
            Self::RetentionComplete { anchor, delivered } => arm(
                self.arm().tag(),
                body(vec![
                    ("anchor", anchor.clone()),
                    ("delivered", Value::UInt(*delivered)),
                ]),
            ),
            Self::Lossless { origin, payload } => arm(
                self.arm().tag(),
                body(vec![
                    ("origin", unit_arm(origin.tag())),
                    ("payload", payload.clone()),
                ]),
            ),
            Self::Conflated {
                origin,
                slot,
                folded,
                payload,
            } => {
                let mut entries = vec![
                    ("folded", Value::Int(i64::from(*folded))),
                    ("origin", unit_arm(origin.tag())),
                    ("payload", payload.clone()),
                ];
                if let Some(slot) = slot {
                    entries.push(("slot", slot.clone()));
                }
                arm(self.arm().tag(), body(entries))
            }
            Self::Credit {
                origin,
                payload,
                pending_after,
            } => arm(
                self.arm().tag(),
                body(vec![
                    ("origin", unit_arm(origin.tag())),
                    ("payload", payload.clone()),
                    ("pending_after", Value::UInt(*pending_after)),
                ]),
            ),
        }
    }

    pub fn decode(bytes: &[u8], ceilings: Ceilings) -> Result<Self, PayloadRejection> {
        let value = circular_core::decode(bytes, ceilings).map_err(PayloadRejection::Codec)?;
        let (tag, mut arguments) = decode_arm(value, "frame")?;
        if arguments.len() != 1 {
            return Err(PayloadRejection::WrongCarrier { key: "frame" });
        }
        let mut fields = object_fields(arguments.pop().expect("one argument"), "frame")?;
        let Some(frame_arm) = SubscriptionFrameArm::from_tag(tag) else {
            return Err(PayloadRejection::UnknownArm { tag });
        };
        match frame_arm {
            SubscriptionFrameArm::RetentionComplete => {
                let anchor = take(&mut fields, "anchor")?;
                let Value::UInt(delivered) = take(&mut fields, "delivered")? else {
                    return Err(PayloadRejection::WrongCarrier { key: "delivered" });
                };
                exhausted(fields)?;
                Ok(Self::RetentionComplete { anchor, delivered })
            }
            SubscriptionFrameArm::Lossless => {
                let origin = FrameOrigin::decode(take(&mut fields, "origin")?)?;
                let payload = take(&mut fields, "payload")?;
                exhausted(fields)?;
                Ok(Self::Lossless { origin, payload })
            }
            SubscriptionFrameArm::Credit => {
                let origin = FrameOrigin::decode(take(&mut fields, "origin")?)?;
                let payload = take(&mut fields, "payload")?;
                let Value::UInt(pending_after) = take(&mut fields, "pending_after")? else {
                    return Err(PayloadRejection::WrongCarrier {
                        key: "pending_after",
                    });
                };
                exhausted(fields)?;
                Ok(Self::Credit {
                    origin,
                    payload,
                    pending_after,
                })
            }
            SubscriptionFrameArm::Conflated => {
                let folded = bounded_u32(take(&mut fields, "folded")?, "folded")?;
                let origin = FrameOrigin::decode(take(&mut fields, "origin")?)?;
                let payload = take(&mut fields, "payload")?;
                let slot = optional(&mut fields, "slot");
                exhausted(fields)?;
                Ok(Self::Conflated {
                    origin,
                    slot,
                    folded,
                    payload,
                })
            }
        }
    }

    pub fn encode(&self, ceilings: Ceilings) -> Result<Vec<u8>, circular_core::CodecError> {
        encode(&self.to_value(), ceilings)
    }
}

impl SubscriptionEndReason {
    #[must_use]
    pub const fn arm(&self) -> SubscriptionEndReasonArm {
        match self {
            Self::ByClient => SubscriptionEndReasonArm::ByClient,
            Self::ConsumerBehind => SubscriptionEndReasonArm::ConsumerBehind,
            Self::TargetGone => SubscriptionEndReasonArm::TargetGone,
            Self::Withdrawn => SubscriptionEndReasonArm::Withdrawn,
            Self::SessionClosed => SubscriptionEndReasonArm::SessionClosed,
            Self::ResetRequired { .. } => SubscriptionEndReasonArm::ResetRequired,
            Self::ScopeGone { .. } => SubscriptionEndReasonArm::ScopeGone,
            Self::IncompatibleClient { .. } => SubscriptionEndReasonArm::IncompatibleClient,
            Self::Complete => SubscriptionEndReasonArm::Complete,
        }
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        let tag = self.arm().tag();
        match self {
            Self::ByClient
            | Self::ConsumerBehind
            | Self::TargetGone
            | Self::Withdrawn
            | Self::SessionClosed
            | Self::Complete => unit_arm(tag),
            Self::ResetRequired { floor_or_cursor } => {
                arm(tag, Value::bytes(floor_or_cursor.clone()))
            }
            Self::ScopeGone { cursor } => arm(tag, Value::bytes(cursor.clone())),
            Self::IncompatibleClient {
                required_environment,
            } => arm(
                tag,
                Value::Object(
                    ObjectValue::try_from_entries([
                        (
                            "declaration_schema".to_owned(),
                            Value::bytes(required_environment.declaration_schema.clone()),
                        ),
                        (
                            "spec_set".to_owned(),
                            Value::bytes(required_environment.spec_set.clone()),
                        ),
                    ])
                    .expect("two axes"),
                ),
            ),
        }
    }

    fn decode(value: Value) -> Result<Self, PayloadRejection> {
        let (tag, mut arguments) = decode_arm(value, "reason")?;
        if arguments.len() > 1 {
            return Err(PayloadRejection::WrongCarrier { key: "reason" });
        }
        use SubscriptionEndReasonArm as A;
        let reason_arm = A::from_tag(tag).ok_or(PayloadRejection::UnknownArm { tag })?;
        let argument = arguments.pop();
        let unit = |argument: Option<Value>, reason: Self| match argument {
            None => Ok(reason),
            Some(_) => Err(PayloadRejection::UnknownArm { tag }),
        };
        let carried =
            |argument: Option<Value>| argument.ok_or(PayloadRejection::UnknownArm { tag });
        match reason_arm {
            A::ByClient => unit(argument, Self::ByClient),
            A::ConsumerBehind => unit(argument, Self::ConsumerBehind),
            A::TargetGone => unit(argument, Self::TargetGone),
            A::Withdrawn => unit(argument, Self::Withdrawn),
            A::SessionClosed => unit(argument, Self::SessionClosed),
            A::ResetRequired => Ok(Self::ResetRequired {
                floor_or_cursor: bytes_of(carried(argument)?, "floor_or_cursor")?,
            }),
            A::ScopeGone => Ok(Self::ScopeGone {
                cursor: bytes_of(carried(argument)?, "cursor")?,
            }),
            A::IncompatibleClient => Ok(Self::IncompatibleClient {
                required_environment: decode_environment(carried(argument)?)?,
            }),
            A::Complete => unit(argument, Self::Complete),
        }
    }
}

impl SubscriptionEnded {
    #[must_use]
    pub fn to_value(&self) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("anchor".to_owned(), Value::bytes(self.anchor.clone())),
                ("code".to_owned(), Value::Int(i64::from(self.code))),
                ("reason".to_owned(), self.reason.to_value()),
            ])
            .expect("three keys"),
        )
    }

    pub fn decode(bytes: &[u8], ceilings: Ceilings) -> Result<Self, PayloadRejection> {
        let mut fields = object(bytes, ceilings)?;
        let anchor = bytes_of(take(&mut fields, "anchor")?, "anchor")?;
        let code = bounded_u32(take(&mut fields, "code")?, "code")?;
        let reason = SubscriptionEndReason::decode(take(&mut fields, "reason")?)?;
        exhausted(fields)?;
        Ok(Self {
            reason,
            code,
            anchor,
        })
    }

    pub fn encode(&self, ceilings: Ceilings) -> Result<Vec<u8>, circular_core::CodecError> {
        encode(&self.to_value(), ceilings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::Boundary;

    const CEILINGS: Ceilings = Ceilings::for_boundary(Boundary::Wire);

    fn body(entries: Vec<(&str, Value)>) -> Vec<u8> {
        encode(
            &Value::Object(
                ObjectValue::try_from_entries(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value)),
                )
                .expect("keys do not overlap"),
            ),
            CEILINGS,
        )
        .expect("encodes")
    }

    #[test]
    fn a_subscribe_carries_a_flat_name_and_opaque_args() {
        let decoded = decode_subscribe(
            &body(vec![
                (
                    "args",
                    Value::Object(
                        ObjectValue::try_from_entries([(
                            "after".to_owned(),
                            Value::bytes(vec![7]),
                        )])
                        .expect("one key"),
                    ),
                ),
                ("target", Value::String("authoring-commits".to_owned())),
            ]),
            CEILINGS,
        )
        .expect("decodes");
        assert_eq!(decoded.target, "authoring-commits");
        assert!(matches!(decoded.args, Value::Object(_)));
    }

    #[test]
    fn absent_args_are_a_value_and_not_a_missing_key() {
        assert!(
            decode_subscribe(
                &body(vec![
                    ("args", Value::Null),
                    ("target", Value::String("failures".to_owned())),
                ]),
                CEILINGS
            )
            .is_ok()
        );
        assert_eq!(
            decode_subscribe(
                &body(vec![("target", Value::String("failures".to_owned()))]),
                CEILINGS
            ),
            Err(PayloadRejection::MissingKey("args"))
        );
    }

    #[test]
    fn pending_after_uses_independent_credit_literals() {
        for pending_after in [0, 2, u64::MAX] {
            let literal = Value::Array(vec![
                Value::Int(3),
                Value::object([
                    ("origin", Value::Int(2)),
                    ("payload", Value::Null),
                    ("pending_after", Value::UInt(pending_after)),
                ])
                .unwrap(),
            ]);
            let bytes = encode(&literal, CEILINGS).unwrap();
            let expected = SubscriptionFrame::Credit {
                origin: FrameOrigin::Live,
                payload: Value::Null,
                pending_after,
            };
            assert_eq!(
                SubscriptionFrame::decode(&bytes, CEILINGS),
                Ok(expected.clone())
            );
            assert_eq!(expected.to_value(), literal);
        }
    }

    #[test]
    fn an_end_without_all_three_places_is_rejected() {
        for (missing, present) in [
            (
                "anchor",
                vec![("code", Value::Int(1)), ("reason", Value::Int(1))],
            ),
            (
                "code",
                vec![("anchor", Value::bytes(vec![1])), ("reason", Value::Int(1))],
            ),
            (
                "reason",
                vec![("anchor", Value::bytes(vec![1])), ("code", Value::Int(1))],
            ),
        ] {
            assert_eq!(
                SubscriptionEnded::decode(&body(present), CEILINGS),
                Err(PayloadRejection::MissingKey(missing))
            );
        }
    }

    #[test]
    fn an_unassigned_end_reason_is_rejected() {
        assert_eq!(
            SubscriptionEnded::decode(
                &body(vec![
                    ("anchor", Value::bytes(vec![1])),
                    ("code", Value::Int(1)),
                    ("reason", Value::Int(10)),
                ]),
                CEILINGS
            ),
            Err(PayloadRejection::UnknownArm { tag: 10 })
        );
    }
}
