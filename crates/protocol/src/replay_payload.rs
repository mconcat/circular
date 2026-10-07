
use circular_core::{Boundary, Ceilings, ObjectValue, RevisionEpochId, Value, encode};

use crate::scope_identity::{PlanActorKey, decode_plan_actor_key, plan_actor_key_value};
use crate::wire_value::{
    PayloadRejection, arm, decode_arm, decode_canonical_set, decode_ratio, exhausted, object,
    object_fields, optional, take, unit_arm, unsigned_of,
};

circular_core::closed_table! {
    pub enum Tail: i64 {
        Live = 1,
        DryRun = 2,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogCutComponent {
    pub actor: PlanActorKey,
    pub index: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LogCut {
    pub components: Vec<LogCutComponent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Arrangement {
    Observational { from: ReplayTarget },
    Counterfactual { from: ReplayTarget },
    Divergent { at: ReplayTarget, tail: Tail },
    DryRun,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Pace {
    Free,
    Paused,
    Step { upto: ReplayTarget },
    Realtime { num: u64, den: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayStart {
    pub arrangement: Arrangement,
    pub pace: Pace,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayTarget {
    pub stream: u64,
    pub cut: LogCut,
    pub revision_epoch: RevisionEpochId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayRewind {
    pub to: Option<ReplayTarget>,
    pub pace: Pace,
}

impl Tail {
    fn to_value(self) -> Value {
        unit_arm(self.tag())
    }

    fn decode(value: Value) -> Result<Self, PayloadRejection> {
        let (tag, arguments) = decode_arm(value, "tail")?;
        if !arguments.is_empty() {
            return Err(PayloadRejection::WrongCarrier { key: "tail" });
        }
        Self::from_tag(tag).ok_or(PayloadRejection::UnknownArm { tag })
    }
}

impl LogCut {
    /// Opens the published `LogCut` carrier outside a replay command.
    ///
    /// Query surfaces (for example the time-machine timeline) return the same
    /// cut value that `ReplayRewind.to` consumes.  Keeping the decoder here
    /// prevents clients from reimplementing the canonical-set and unique-actor
    /// checks merely because the value arrived through the Query partition.
    pub fn from_value(value: Value, ceilings: Ceilings) -> Result<Self, PayloadRejection> {
        Self::decode(value, "cut", ceilings)
    }

    pub fn to_value(&self) -> Result<Value, PayloadRejection> {
        let items = self
            .components
            .iter()
            .map(|component| {
                let index = i64::try_from(component.index)
                    .map_err(|_| PayloadRejection::BeyondWidth { key: "index" })?;
                Ok(Value::Object(
                    ObjectValue::try_from_entries([
                        ("index".to_owned(), Value::Int(index)),
                        ("actor".to_owned(), plan_actor_key_value(&component.actor)),
                    ])
                    .expect("two keys"),
                ))
            })
            .collect::<Result<Vec<_>, PayloadRejection>>()?;
        let mut encoded_items = items
            .into_iter()
            .map(|item| {
                let encoded = encode(&item, Ceilings::for_boundary(Boundary::Wire))
                    .map_err(PayloadRejection::Codec)?;
                Ok((encoded, item))
            })
            .collect::<Result<Vec<_>, PayloadRejection>>()?;
        encoded_items.sort_by(|(left, _), (right, _)| left.cmp(right));
        let items = encoded_items.into_iter().map(|(_, item)| item).collect();
        Ok(Value::Array(items))
    }

    fn decode(
        value: Value,
        key: &'static str,
        ceilings: Ceilings,
    ) -> Result<Self, PayloadRejection> {
        let components = decode_canonical_set(value, key, ceilings, |item| {
            let mut fields = object_fields(item, "cut")?;
            let index = unsigned_of(take(&mut fields, "index")?, "index")?;
            let actor = decode_plan_actor_key(take(&mut fields, "actor")?)?;
            exhausted(fields)?;
            Ok(LogCutComponent { actor, index })
        })?;
        let mut keys = components
            .iter()
            .map(|component| {
                encode(&plan_actor_key_value(&component.actor), ceilings)
                    .map_err(PayloadRejection::Codec)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let count = keys.len();
        keys.sort();
        keys.dedup();
        if keys.len() != count {
            return Err(PayloadRejection::NotCanonical { key });
        }
        Ok(Self { components })
    }
}

fn stream_value(stream: u64) -> Result<Value, PayloadRejection> {
    i64::try_from(stream)
        .map(Value::Int)
        .map_err(|_| PayloadRejection::BeyondWidth { key: "stream" })
}

impl Arrangement {
    pub fn to_value(&self) -> Result<Value, PayloadRejection> {
        Ok(match self {
            Self::Observational { from } => arm(1, from.to_value()?),
            Self::Counterfactual { from } => arm(2, from.to_value()?),
            Self::Divergent { at, tail } => arm(
                3,
                Value::Object(
                    ObjectValue::try_from_entries([
                        ("at".to_owned(), at.to_value()?),
                        ("tail".to_owned(), tail.to_value()),
                    ])
                    .expect("two keys"),
                ),
            ),
            Self::DryRun => unit_arm(4),
        })
    }

    fn decode(value: Value, ceilings: Ceilings) -> Result<Self, PayloadRejection> {
        let (tag, mut arguments) = decode_arm(value, "arrangement")?;
        if arguments.len() > 1 {
            return Err(PayloadRejection::WrongCarrier { key: "arrangement" });
        }
        match tag {
            1 | 2 => {
                let from = ReplayTarget::decode(
                    arguments
                        .pop()
                        .ok_or(PayloadRejection::UnknownArm { tag })?,
                    ceilings,
                    "from",
                )?;
                Ok(if tag == 1 {
                    Self::Observational { from }
                } else {
                    Self::Counterfactual { from }
                })
            }
            3 => {
                let argument = arguments
                    .pop()
                    .ok_or(PayloadRejection::UnknownArm { tag })?;
                let mut fields = object_fields(argument, "arrangement")?;
                let at = ReplayTarget::decode(take(&mut fields, "at")?, ceilings, "at")?;
                let tail = Tail::decode(take(&mut fields, "tail")?)?;
                exhausted(fields)?;
                Ok(Self::Divergent { at, tail })
            }
            4 if arguments.is_empty() => Ok(Self::DryRun),
            other => Err(PayloadRejection::UnknownArm { tag: other }),
        }
    }
}

impl Pace {
    pub fn to_value(&self) -> Result<Value, PayloadRejection> {
        Ok(match self {
            Self::Free => unit_arm(1),
            Self::Paused => unit_arm(2),
            Self::Step { upto } => arm(3, upto.to_value()?),
            Self::Realtime { num, den } => {
                let den = i64::try_from(*den)
                    .map_err(|_| PayloadRejection::BeyondWidth { key: "den" })?;
                let num = i64::try_from(*num)
                    .map_err(|_| PayloadRejection::BeyondWidth { key: "num" })?;
                arm(
                    4,
                    Value::Object(
                        ObjectValue::try_from_entries([
                            ("den".to_owned(), Value::Int(den)),
                            ("num".to_owned(), Value::Int(num)),
                        ])
                        .expect("two keys"),
                    ),
                )
            }
        })
    }

    fn decode(value: Value, ceilings: Ceilings) -> Result<Self, PayloadRejection> {
        let (tag, mut arguments) = decode_arm(value, "pace")?;
        if arguments.len() > 1 {
            return Err(PayloadRejection::WrongCarrier { key: "pace" });
        }
        match tag {
            1 if arguments.is_empty() => Ok(Self::Free),
            2 if arguments.is_empty() => Ok(Self::Paused),
            3 => Ok(Self::Step {
                upto: ReplayTarget::decode(
                    arguments
                        .pop()
                        .ok_or(PayloadRejection::UnknownArm { tag })?,
                    ceilings,
                    "upto",
                )?,
            }),
            4 if !arguments.is_empty() => {
                let (num, den) =
                    decode_ratio(arguments.pop().expect("one argument"), "multiplier")?;
                Ok(Self::Realtime { num, den })
            }
            other => Err(PayloadRejection::UnknownArm { tag: other }),
        }
    }
}

impl ReplayStart {
    pub fn to_value(&self) -> Result<Value, PayloadRejection> {
        Ok(Value::Object(
            ObjectValue::try_from_entries([
                ("arrangement".to_owned(), self.arrangement.to_value()?),
                ("pace".to_owned(), self.pace.to_value()?),
            ])
            .expect("two keys"),
        ))
    }
}

impl ReplayTarget {
    pub fn to_value(&self) -> Result<Value, PayloadRejection> {
        Ok(Value::Object(
            ObjectValue::try_from_entries([
                ("cut".to_owned(), self.cut.to_value()?),
                (
                    "revision_epoch".to_owned(),
                    Value::UInt(self.revision_epoch.get()),
                ),
                ("stream".to_owned(), stream_value(self.stream)?),
            ])
            .expect("three keys"),
        ))
    }

    pub fn from_value(value: Value, ceilings: Ceilings) -> Result<Self, PayloadRejection> {
        Self::decode(value, ceilings, "target")
    }

    fn decode(
        value: Value,
        ceilings: Ceilings,
        key: &'static str,
    ) -> Result<Self, PayloadRejection> {
        let mut fields = object_fields(value, key)?;
        let cut = LogCut::decode(take(&mut fields, "cut")?, "cut", ceilings)?;
        let revision_epoch = match take(&mut fields, "revision_epoch")? {
            Value::UInt(value) => {
                RevisionEpochId::new(value).ok_or(PayloadRejection::WrongCarrier {
                    key: "revision_epoch",
                })?
            }
            _ => {
                return Err(PayloadRejection::WrongCarrier {
                    key: "revision_epoch",
                });
            }
        };
        let stream = unsigned_of(take(&mut fields, "stream")?, "stream")?;
        exhausted(fields)?;
        Ok(Self {
            stream,
            cut,
            revision_epoch,
        })
    }
}

impl ReplayRewind {
    pub fn to_value(&self) -> Result<Value, PayloadRejection> {
        let mut entries = vec![("pace".to_owned(), self.pace.to_value()?)];
        if let Some(to) = &self.to {
            entries.push(("to".to_owned(), to.to_value()?));
        }
        Ok(Value::Object(
            ObjectValue::try_from_entries(entries).expect("distinct keys"),
        ))
    }
}

pub fn decode_replay_start(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<ReplayStart, PayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let arrangement = Arrangement::decode(take(&mut fields, "arrangement")?, ceilings)?;
    let pace = Pace::decode(take(&mut fields, "pace")?, ceilings)?;
    exhausted(fields)?;
    Ok(ReplayStart { arrangement, pace })
}

pub fn decode_replay_rewind(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<ReplayRewind, PayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let pace = Pace::decode(take(&mut fields, "pace")?, ceilings)?;
    let to = optional(&mut fields, "to")
        .map(|to| ReplayTarget::decode(to, ceilings, "to"))
        .transpose()?;
    exhausted(fields)?;
    Ok(ReplayRewind { to, pace })
}

