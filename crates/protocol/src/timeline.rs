use circular_core::{Ceilings, ObjectValue, RevisionEpochId, Value};

use crate::replay_payload::{LogCut, ReplayTarget};
use crate::scope_identity::{PlanActorKey, decode_plan_actor_key, plan_actor_key_value};
use crate::wire_value::{
    PayloadRejection, WireError, exhausted, object_fields, optional, take, text_of,
};

pub const TIMELINE_BINS_QUERY: &str = "timeline.bins";
pub const TIMELINE_AT_QUERY: &str = "timeline.at";
pub const TIMELINE_MAX_BINS: u64 = 4096;

circular_core::closed_table! {
    pub enum TimelineMarkKind {
        Edit => "edit",
        Restart => "restart",
        Pause => "pause",
        Resume => "resume",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimelineArgsRejection {
    Payload(PayloadRejection),
    EmptyRange,
    BinsOutOfRange,
    RangeOverflow,
}

impl From<PayloadRejection> for TimelineArgsRejection {
    fn from(rejection: PayloadRejection) -> Self {
        Self::Payload(rejection)
    }
}

impl From<WireError> for TimelineArgsRejection {
    fn from(error: WireError) -> Self {
        Self::Payload(error.into())
    }
}

impl std::fmt::Display for TimelineArgsRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Payload(rejection) => write!(formatter, "malformed arguments: {rejection:?}"),
            Self::EmptyRange => formatter.write_str("from_ms must precede to_ms"),
            Self::BinsOutOfRange => {
                write!(formatter, "bins must be in 1..={TIMELINE_MAX_BINS}")
            }
            Self::RangeOverflow => formatter.write_str("the binned range exceeds the UInt width"),
        }
    }
}

fn millis_of(value: Value, key: &'static str) -> Result<u64, PayloadRejection> {
    match value {
        Value::UInt(millis) => Ok(millis),
        _ => Err(PayloadRejection::WrongCarrier { key }),
    }
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(
        ObjectValue::try_from_entries(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value)),
        )
        .expect("timeline object keys are unique"),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineBinsArgs {
    pub from_ms: u64,
    pub to_ms: u64,
    pub bins: u64,
    pub actor: Option<PlanActorKey>,
}

impl TimelineBinsArgs {
    pub fn from_value(value: Value) -> Result<Self, TimelineArgsRejection> {
        let mut fields = object_fields(value, "args")?;
        let from_ms = millis_of(take(&mut fields, "from_ms")?, "from_ms")?;
        let to_ms = millis_of(take(&mut fields, "to_ms")?, "to_ms")?;
        let bins = millis_of(take(&mut fields, "bins")?, "bins")?;
        let actor = optional(&mut fields, "actor")
            .map(decode_plan_actor_key)
            .transpose()?;
        exhausted(fields)?;
        let args = Self {
            from_ms,
            to_ms,
            bins,
            actor,
        };
        args.coverage()?;
        Ok(args)
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut fields = vec![
            ("bins", Value::UInt(self.bins)),
            ("from_ms", Value::UInt(self.from_ms)),
            ("to_ms", Value::UInt(self.to_ms)),
        ];
        if let Some(actor) = &self.actor {
            fields.push(("actor", plan_actor_key_value(actor)));
        }
        object(fields)
    }

    pub fn coverage(&self) -> Result<(u64, u64), TimelineArgsRejection> {
        if self.from_ms >= self.to_ms {
            return Err(TimelineArgsRejection::EmptyRange);
        }
        if self.bins == 0 || self.bins > TIMELINE_MAX_BINS {
            return Err(TimelineArgsRejection::BinsOutOfRange);
        }
        let bin_ms = (self.to_ms - self.from_ms).div_ceil(self.bins);
        let covered = bin_ms
            .checked_mul(self.bins)
            .and_then(|span| self.from_ms.checked_add(span))
            .ok_or(TimelineArgsRejection::RangeOverflow)?;
        Ok((bin_ms, covered))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TimelineBin {
    pub count: u64,
    pub incidents: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineMark {
    pub kind: TimelineMarkKind,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineBins {
    pub from_ms: u64,
    pub to_ms: u64,
    pub bin_ms: u64,
    pub bins: Vec<TimelineBin>,
    pub marks: Vec<TimelineMark>,
    pub clock_regressions: u64,
}

impl TimelineBins {
    #[must_use]
    pub fn to_value(&self) -> Value {
        object(vec![
            ("bin_ms", Value::UInt(self.bin_ms)),
            (
                "bins",
                Value::Array(
                    self.bins
                        .iter()
                        .map(|bin| {
                            object(vec![
                                ("count", Value::UInt(bin.count)),
                                ("incidents", Value::UInt(bin.incidents)),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("clock_regressions", Value::UInt(self.clock_regressions)),
            ("from_ms", Value::UInt(self.from_ms)),
            (
                "marks",
                Value::Array(
                    self.marks
                        .iter()
                        .map(|mark| {
                            object(vec![
                                ("at_ms", Value::UInt(mark.at_ms)),
                                ("kind", Value::String(mark.kind.as_str().to_owned())),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("to_ms", Value::UInt(self.to_ms)),
        ])
    }

    pub fn from_value(value: Value) -> Result<Self, PayloadRejection> {
        let mut fields = object_fields(value, "timeline.bins")?;
        let from_ms = millis_of(take(&mut fields, "from_ms")?, "from_ms")?;
        let to_ms = millis_of(take(&mut fields, "to_ms")?, "to_ms")?;
        let bin_ms = millis_of(take(&mut fields, "bin_ms")?, "bin_ms")?;
        let clock_regressions =
            millis_of(take(&mut fields, "clock_regressions")?, "clock_regressions")?;
        let Value::Array(bins) = take(&mut fields, "bins")? else {
            return Err(PayloadRejection::WrongCarrier { key: "bins" });
        };
        let bins = bins
            .into_iter()
            .map(|bin| {
                let mut bin = object_fields(bin, "bins")?;
                let count = millis_of(take(&mut bin, "count")?, "count")?;
                let incidents = millis_of(take(&mut bin, "incidents")?, "incidents")?;
                exhausted(bin)?;
                Ok(TimelineBin { count, incidents })
            })
            .collect::<Result<Vec<_>, PayloadRejection>>()?;
        let Value::Array(marks) = take(&mut fields, "marks")? else {
            return Err(PayloadRejection::WrongCarrier { key: "marks" });
        };
        let marks = marks
            .into_iter()
            .map(|mark| {
                let mut mark = object_fields(mark, "marks")?;
                let at_ms = millis_of(take(&mut mark, "at_ms")?, "at_ms")?;
                let kind = TimelineMarkKind::from_str(&text_of(take(&mut mark, "kind")?, "kind")?)
                    .ok_or(PayloadRejection::WrongCarrier { key: "kind" })?;
                exhausted(mark)?;
                Ok(TimelineMark { kind, at_ms })
            })
            .collect::<Result<Vec<_>, PayloadRejection>>()?;
        exhausted(fields)?;
        Ok(Self {
            from_ms,
            to_ms,
            bin_ms,
            bins,
            marks,
            clock_regressions,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineAtArgs {
    pub at_ms: u64,
}

impl TimelineAtArgs {
    pub fn from_value(value: Value) -> Result<Self, TimelineArgsRejection> {
        let mut fields = object_fields(value, "args")?;
        let at_ms = millis_of(take(&mut fields, "at_ms")?, "at_ms")?;
        exhausted(fields)?;
        Ok(Self { at_ms })
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        object(vec![("at_ms", Value::UInt(self.at_ms))])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineAt {
    pub requested_ms: u64,
    pub resolved_ms: u64,
    pub target: ReplayTarget,
}

impl TimelineAt {
    pub fn to_value(&self) -> Result<Value, PayloadRejection> {
        let Value::Object(target) = self.target.to_value()? else {
            unreachable!("a replay target is an object")
        };
        let mut fields: Vec<(String, Value)> = target.into_map().into_iter().collect();
        fields.push(("requested_ms".to_owned(), Value::UInt(self.requested_ms)));
        fields.push(("resolved_ms".to_owned(), Value::UInt(self.resolved_ms)));
        Ok(Value::Object(
            ObjectValue::try_from_entries(fields).expect("timeline.at keys are unique"),
        ))
    }

    pub fn from_value(value: Value, ceilings: Ceilings) -> Result<Self, PayloadRejection> {
        let mut fields = object_fields(value, "timeline.at")?;
        let requested_ms = millis_of(take(&mut fields, "requested_ms")?, "requested_ms")?;
        let resolved_ms = millis_of(take(&mut fields, "resolved_ms")?, "resolved_ms")?;
        let target = ReplayTarget::from_value(
            Value::Object(
                ObjectValue::try_from_entries(fields)
                    .map_err(|_| PayloadRejection::NotCanonical { key: "timeline.at" })?,
            ),
            ceilings,
        )?;
        Ok(Self {
            requested_ms,
            resolved_ms,
            target,
        })
    }

    #[must_use]
    pub const fn new(
        requested_ms: u64,
        resolved_ms: u64,
        stream: u64,
        revision_epoch: RevisionEpochId,
        cut: LogCut,
    ) -> Self {
        Self {
            requested_ms,
            resolved_ms,
            target: ReplayTarget {
                stream,
                cut,
                revision_epoch,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::Boundary;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn the_answer_bytes_are_the_ones_the_sdk_reads() {
        let bins = TimelineBins {
            from_ms: 100,
            to_ms: 120,
            bin_ms: 10,
            bins: vec![
                TimelineBin {
                    count: 3,
                    incidents: 1,
                },
                TimelineBin::default(),
            ],
            marks: vec![TimelineMark {
                kind: TimelineMarkKind::Pause,
                at_ms: 105,
            }],
            clock_regressions: 2,
        };
        let ceilings = Ceilings::for_boundary(Boundary::Wire);
        let bytes = circular_core::encode(&bins.to_value(), ceilings).unwrap();
        assert_eq!(hex(&bytes), BINS_GOLDEN);
        assert_eq!(
            TimelineBins::from_value(circular_core::decode(&bytes, ceilings).unwrap()),
            Ok(bins)
        );
        let at = TimelineAt::new(
            50,
            40,
            1,
            RevisionEpochId::new(2).unwrap(),
            LogCut {
                components: vec![crate::replay_payload::LogCutComponent {
                    actor: decode_plan_actor_key(
                        Value::object([
                            ("local", Value::string("sink")),
                            ("scope", Value::Array(Vec::new())),
                        ])
                        .unwrap(),
                    )
                    .unwrap(),
                    index: 3,
                }],
            },
        );
        let bytes = circular_core::encode(&at.to_value().unwrap(), ceilings).unwrap();
        assert_eq!(hex(&bytes), AT_GOLDEN);
        assert_eq!(
            TimelineAt::from_value(circular_core::decode(&bytes, ceilings).unwrap(), ceilings),
            Ok(at)
        );
    }

    const BINS_GOLDEN: &str = "08000000060000000662696e5f6d7309000000000000000a0000000462696e730700000002080000000200000005636f756e7409000000000000000300000009696e636964656e7473090000000000000001080000000200000005636f756e7409000000000000000000000009696e636964656e747309000000000000000000000011636c6f636b5f72656772657373696f6e730900000000000000020000000766726f6d5f6d73090000000000000064000000056d61726b73070000000108000000020000000561745f6d73090000000000000069000000046b696e640500000005706175736500000005746f5f6d73090000000000000078";
    const AT_GOLDEN: &str = "08000000050000000363757407000000010800000002000000056163746f720800000002000000056c6f63616c050000000473696e6b0000000573636f7065070000000000000005696e6465780300000000000000030000000c7265717565737465645f6d730900000000000000320000000b7265736f6c7665645f6d730900000000000000280000000e7265766973696f6e5f65706f63680900000000000000020000000673747265616d030000000000000001";

    #[test]
    fn a_range_that_does_not_divide_rounds_the_bin_up_and_reports_the_covered_end() {
        let args = TimelineBinsArgs {
            from_ms: 1_000,
            to_ms: 1_010,
            bins: 3,
            actor: None,
        };
        assert_eq!(args.coverage(), Ok((4, 1_012)));
        let exact = TimelineBinsArgs {
            from_ms: 0,
            to_ms: 4_096,
            bins: 4_096,
            actor: None,
        };
        assert_eq!(exact.coverage(), Ok((1, 4_096)));
    }

    #[test]
    fn arguments_outside_the_value_space_are_refused_with_a_reason() {
        let args = |fields: Vec<(&str, Value)>| TimelineBinsArgs::from_value(object(fields));
        assert_eq!(
            args(vec![
                ("from_ms", Value::UInt(5)),
                ("to_ms", Value::UInt(5)),
                ("bins", Value::UInt(1)),
            ]),
            Err(TimelineArgsRejection::EmptyRange)
        );
        assert_eq!(
            args(vec![
                ("from_ms", Value::UInt(0)),
                ("to_ms", Value::UInt(5)),
                ("bins", Value::UInt(0)),
            ]),
            Err(TimelineArgsRejection::BinsOutOfRange)
        );
        assert_eq!(
            args(vec![
                ("from_ms", Value::UInt(0)),
                ("to_ms", Value::UInt(5)),
                ("bins", Value::UInt(4_097)),
            ]),
            Err(TimelineArgsRejection::BinsOutOfRange)
        );
        assert_eq!(
            args(vec![
                ("from_ms", Value::UInt(u64::MAX - 1)),
                ("to_ms", Value::UInt(u64::MAX)),
                ("bins", Value::UInt(2)),
            ]),
            Err(TimelineArgsRejection::RangeOverflow)
        );
        assert_eq!(
            args(vec![
                ("from_ms", Value::Int(0)),
                ("to_ms", Value::UInt(5)),
                ("bins", Value::UInt(1)),
            ]),
            Err(TimelineArgsRejection::Payload(
                PayloadRejection::WrongCarrier { key: "from_ms" }
            ))
        );
        assert!(matches!(
            args(vec![
                ("from_ms", Value::UInt(0)),
                ("to_ms", Value::UInt(5)),
                ("bins", Value::UInt(1)),
                ("by_actor", Value::Bool(true)),
            ]),
            Err(TimelineArgsRejection::Payload(
                PayloadRejection::UnknownKey(_)
            ))
        ));
    }
}
