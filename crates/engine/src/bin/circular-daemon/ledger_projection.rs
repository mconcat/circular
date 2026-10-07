use super::*;
use circular_store::{BoundaryRecord, OpaqueWitness, ProductRecordCodec};

use crate::daemon::replay::replay_wire_cut as recorded_wire_cut;

pub(crate) struct ProjectionRecord {
    pub(crate) witness: OpaqueWitness,
    source: ProjectionSource,
}

const fn role_name(role: Role) -> &'static str {
    match role {
        Role::Request => "request",
        Role::Progress => "progress",
        Role::Result => "result",
        Role::Error => "error",
    }
}

enum ProjectionSource {
    Arrival {
        record: BoundaryRecord<ProductStore>,
        role: Role,
    },
    Checkpoint(ServerReplayCheckpoint, u64),
    Value(Value),
}

fn timeline_point_value(point: &ServerReplayCheckpoint, stream: u64) -> Result<Value, String> {
    Value::object([
        (
            "at_ms",
            Value::Int(
                i64::try_from(point.at().millis()).map_err(|_| "timeline instant exceeds Int")?,
            ),
        ),
        (
            "cut",
            recorded_wire_cut(point.cut())?
                .to_value()
                .map_err(|e| format!("timeline cut: {e:?}"))?,
        ),
        ("revision_epoch", Value::UInt(point.revision().get())),
        (
            "stream",
            Value::Int(i64::try_from(stream).map_err(|_| "timeline stream exceeds Int")?),
        ),
    ])
    .map_err(|e| format!("timeline point: {e:?}"))
}

impl ProjectionRecord {
    pub(super) fn new(record: &BoundaryRecord<ProductStore>, role: Role) -> Result<Self, String> {
        Ok(Self {
            witness: OpaqueWitness::for_record_ref(
                record.header().class(),
                record.header().key(),
                &ProductRecordCodec,
            )
            .map_err(|e| format!("projection record reference: {e:?}"))?,
            source: ProjectionSource::Arrival {
                record: record.clone(),
                role,
            },
        })
    }

    pub(crate) fn recorded(witness: OpaqueWitness, value: Value) -> Self {
        Self {
            witness,
            source: ProjectionSource::Value(value),
        }
    }

    pub(super) fn checkpoint(
        witness: OpaqueWitness,
        point: ServerReplayCheckpoint,
        stream: u64,
    ) -> Self {
        Self {
            witness,
            source: ProjectionSource::Checkpoint(point, stream),
        }
    }

    pub(crate) fn value(&self) -> Result<Value, String> {
        let (record, role) = match &self.source {
            ProjectionSource::Value(value) => return Ok(value.clone()),
            ProjectionSource::Checkpoint(point, stream) => {
                return timeline_point_value(point, *stream);
            }
            ProjectionSource::Arrival { record, role } => (record, role),
        };
        let ClassKey::Boundary(BoundaryKey::Arrival { origin, .. }) = record.header().key() else {
            return Err("projection reference is not an arrival".into());
        };
        let BoundaryFact::Arrival {
            body: arrival_body,
            causal_parents,
            ..
        } = record.fact()
        else {
            return Err("projection reference is not an arrival".into());
        };
        let payload = match arrival_body {
            circular_store::ArrivalBody::Owned(payload) => payload,
            circular_store::ArrivalBody::Emitted { .. } => {
                return Err("projection reference body is an unresolved reference".into());
            }
        };
        let body = if matches!(
            origin.as_ref(),
            circular_store::ArrivalKey::EffectOutcome { .. }
        ) {
            engine::effect_outcome_record::effect_outcome_summary(payload)?
        } else {
            circular_core::decode(
                payload.body(),
                circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
            )
            .unwrap_or(Value::Null)
        };
        let fields = vec![
            ("role", Value::string(role_name(*role))),
            ("kind", Value::Int(i64::from(origin.kind_tag()))),
            (
                "origin",
                match origin.as_ref() {
                    circular_store::ArrivalKey::ExternalInject { origin, .. } => {
                        Value::bytes(origin.body().to_vec())
                    }
                    _ => Value::Null,
                },
            ),
            (
                "at",
                crate::daemon::restart_query::stamp_value(record.header().at())?,
            ),
            (
                "causal_parents",
                Value::array(
                    causal_parents
                        .iter()
                        .map(|parent| {
                            crate::daemon::restart_query::stamp_value(
                                circular_core::EventId::stamp(parent),
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            ),
            ("body", body),
        ];
        Value::object(fields).map_err(|e| format!("arrival scan item: {e:?}"))
    }
}

pub(crate) trait ProjectionTail {
    fn fill(&mut self, records: &mut Vec<ProjectionRecord>, limit: usize) -> Result<bool, String>;
}

impl ProjectionTail for crate::daemon::subscription::records::FrozenRecords {
    fn fill(&mut self, records: &mut Vec<ProjectionRecord>, limit: usize) -> Result<bool, String> {
        self.fill(records, limit)
    }
}

pub(crate) struct ProjectionSnapshot {
    pub(crate) cut: Option<Value>,
    pub(crate) folded_from: Option<Value>,
    pub(crate) anchor: Value,
    pub(crate) records: Vec<ProjectionRecord>,
    pub(crate) records_tail: Option<Box<dyn ProjectionTail>>,
    /// Existing checkpoint certificates paired with the complete record prefix required.
    reached: Vec<(usize, Value)>,
}

impl ProjectionSnapshot {
    pub(crate) fn recorded(
        anchor: Value,
        records: Vec<ProjectionRecord>,
        reached: Vec<(usize, Value)>,
    ) -> Self {
        Self {
            anchor,
            records,
            records_tail: None,
            cut: None,
            folded_from: None,
            reached,
        }
    }

    #[cfg(test)]
    pub(crate) fn filled(mut self) -> Self {
        if let Some(mut tail) = self.records_tail.take() {
            assert!(
                tail.fill(&mut self.records, usize::MAX)
                    .expect("tail fills")
            );
        }
        self
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.records.len()
    }

    pub(crate) fn reached(&self, emitted: usize) -> Option<Value> {
        self.reached
            .iter()
            .rev()
            .find(|(required, _)| *required <= emitted)
            .map(|(_, value)| value.clone())
    }
}

#[cfg(test)]
pub(crate) fn test_snapshot(anchor: Value, values: Vec<Value>) -> ProjectionSnapshot {
    use circular_store::{ArrivalOrigin, RecordOrigin};
    let actor = NamedActorId::new(ScopeId::root(), circular_plan::Name::from_normalized("tap"));
    let records = values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let record = BoundaryRecord::arrival(
                actor.as_actor_id(),
                Stamp::from_event_producer(
                    Tick::new(1),
                    actor.clone(),
                    Sequence::new(index as u64 + 1).unwrap(),
                    RevisionEpochId::new(1).unwrap(),
                ),
                RecordOrigin::Stream,
                ArrivalOrigin::TimerFire {
                    timer: circular_runtime::EffectId::from_components(
                        actor.clone().into(),
                        vec![circular_plan::Generation::new(0)].into_boxed_slice(),
                        circular_runtime::EffectOccasion::Delivery(
                            None,
                            Stamp::from_event_producer(
                                Tick::new(1),
                                actor.clone(),
                                Sequence::FIRST,
                                RevisionEpochId::new(1).unwrap(),
                            ),
                        ),
                        index as u64,
                    )
                    .expect("root fixture has one generation and a distinct hook index"),
                },
                circular_store::ArrivalBody::Owned(EncodedPayload::new(
                    PayloadVersionTag::FIRST,
                    &circular_core::encode(
                        &value,
                        circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
                    )
                    .unwrap(),
                )),
                circular_core::ArrivalIndex::new(index as u64),
                Box::new([]),
                RecordedInstant::from_millis(10),
                Some(PortId::try_new("in").unwrap()),
            );
            ProjectionRecord::new(&record, Role::Request).unwrap()
        })
        .collect();
    ProjectionSnapshot {
        anchor,
        records,
        records_tail: None,
        cut: None,
        folded_from: None,
        reached: Vec::new(),
    }
}
