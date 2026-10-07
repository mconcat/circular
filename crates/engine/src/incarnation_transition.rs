
use circular_core::{
    Boundary, BuiltinObservationName, Ceilings, EncodedPayload, PayloadVersionTag, Stamp, Tick,
    Value,
};
use circular_plan::{LocalKey, Name, NamedActorId, ScopedActorId};

use circular_store::{
    IncarnationId, ObservationBucket, ObservationFact, ObservationItemKey, ObservationRecord,
    OpaqueId, ProductStore, Record, RecordOrigin,
};

const LIFECYCLE_CARRIER: &str = "daemon-actor-lifecycle-v2";
const LIFECYCLE_BODY_VERSION: u16 = 2;

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub(crate) enum LifecyclePhase: u64 {
        Draining = 1,
        AdmissionClosed = 2,
        Terminated = 3,
        Prepared = 4,
        Activated = 5,
        Restarted = 6,
        ConfigRestarted = 7,
        Abandoned = 8,
        ResumeDenied = 9,
        ConfigApplied = 10,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct TransitionDetail {
    retired: u64,
    prepared: u64,
    remaining: u64,
    next_recovery: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RecordedTransition {
    phase: LifecyclePhase,
    incarnation: u64,
    detail: TransitionDetail,
}

impl RecordedTransition {
    pub(crate) fn of(transition: circular_runtime::IncarnationTransition) -> Self {
        use circular_runtime::IncarnationTransition as T;
        let (phase, incarnation, detail) = match transition {
            T::Activated { incarnation } => (
                LifecyclePhase::Activated,
                incarnation,
                TransitionDetail::default(),
            ),
            T::ConfigApplied { incarnation } => (
                LifecyclePhase::ConfigApplied,
                incarnation,
                TransitionDetail::default(),
            ),
            T::Draining { incarnation } => (
                LifecyclePhase::Draining,
                incarnation,
                TransitionDetail::default(),
            ),
            T::Terminated { incarnation } => (
                LifecyclePhase::Terminated,
                incarnation,
                TransitionDetail::default(),
            ),
            T::Restarted {
                retired,
                prepared,
                remaining,
            } => (
                LifecyclePhase::Restarted,
                prepared,
                TransitionDetail {
                    retired,
                    prepared,
                    remaining: remaining as u64,
                    next_recovery: 0,
                },
            ),
            T::ConfigRestarted { retired, prepared } => (
                LifecyclePhase::ConfigRestarted,
                prepared,
                TransitionDetail {
                    retired,
                    prepared,
                    remaining: 0,
                    next_recovery: 0,
                },
            ),
            T::Abandoned {
                incarnation,
                next_recovery,
            } => (
                LifecyclePhase::Abandoned,
                incarnation,
                TransitionDetail {
                    retired: 0,
                    prepared: 0,
                    remaining: 0,
                    next_recovery: next_recovery.get(),
                },
            ),
            T::ResumeDenied {
                incarnation,
                next_recovery,
            } => (
                LifecyclePhase::ResumeDenied,
                incarnation,
                TransitionDetail {
                    retired: 0,
                    prepared: 0,
                    remaining: 0,
                    next_recovery: next_recovery.get(),
                },
            ),
        };
        Self {
            phase,
            incarnation,
            detail,
        }
    }

    #[cfg(test)]
    pub(crate) fn transition(self) -> Option<circular_runtime::IncarnationTransition> {
        use circular_runtime::IncarnationTransition as T;
        Some(match self.phase {
            LifecyclePhase::Activated => T::Activated {
                incarnation: self.incarnation,
            },
            LifecyclePhase::ConfigApplied => T::ConfigApplied {
                incarnation: self.incarnation,
            },
            LifecyclePhase::Draining => T::Draining {
                incarnation: self.incarnation,
            },
            LifecyclePhase::Terminated => T::Terminated {
                incarnation: self.incarnation,
            },
            LifecyclePhase::Restarted => T::Restarted {
                retired: self.detail.retired,
                prepared: self.detail.prepared,
                remaining: usize::try_from(self.detail.remaining).ok()?,
            },
            LifecyclePhase::ConfigRestarted => T::ConfigRestarted {
                retired: self.detail.retired,
                prepared: self.detail.prepared,
            },
            LifecyclePhase::Abandoned => T::Abandoned {
                incarnation: self.incarnation,
                next_recovery: Tick::new(self.detail.next_recovery),
            },
            LifecyclePhase::ResumeDenied => T::ResumeDenied {
                incarnation: self.incarnation,
                next_recovery: Tick::new(self.detail.next_recovery),
            },
            LifecyclePhase::Prepared | LifecyclePhase::AdmissionClosed => return None,
        })
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LifecycleIdentity {
    actor: ScopedActorId,
    generation: u64,
    declaration_revision: u64,
    config_revision: u64,
    phase: LifecyclePhase,
    detail: TransitionDetail,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LifecycleProjection {
    key: OpaqueId,
    at: Tick,
    stamp: Stamp<circular_plan::ActorId>,
}

fn lifecycle_record(
    identity: &LifecycleIdentity,
    projection: &LifecycleProjection,
) -> Result<Record<ProductStore>, String> {
    let value = lifecycle_value(identity, projection)?;
    let payload = circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal))
        .map_err(|error| format!("daemon lifecycle carrier encoding failed: {error}"))?;
    Ok(Record::Observation(ObservationRecord::lifecycle(
        projection.stamp.clone(),
        ObservationBucket::from_millis(projection.at.get()),
        RecordOrigin::Actor(IncarnationId::new(identity.generation)),
        ObservationItemKey::new(
            BuiltinObservationName::IncarnationTransition,
            projection.key,
        ),
        EncodedPayload::new(
            PayloadVersionTag::new(LIFECYCLE_BODY_VERSION)
                .map_err(|error| format!("daemon lifecycle body version: {error}"))?,
            &payload,
        ),
    )))
}

pub(crate) fn incarnation_transition_record(
    actor: &NamedActorId,
    incarnation: u64,
    revision: circular_core::RevisionEpochId,
    transition: circular_runtime::IncarnationTransition,
    at: Stamp<circular_plan::ActorId>,
    key: OpaqueId,
) -> Result<Record<ProductStore>, String> {
    let body = RecordedTransition::of(transition);
    let identity = LifecycleIdentity {
        actor: actor.as_scoped().clone(),
        generation: incarnation,
        declaration_revision: revision.get(),
        config_revision: revision.get(),
        phase: body.phase,
        detail: body.detail,
    };
    let projection = LifecycleProjection {
        key,
        at: at.physical_time(),
        stamp: at,
    };
    lifecycle_record(&identity, &projection)
}

pub fn incarnation_transition_value(
    record: &Record<ProductStore>,
) -> Result<Option<Value>, String> {
    let Record::Observation(observation) = record else {
        return Ok(None);
    };
    let circular_store::ClassKey::Observation(circular_store::ObservationKey::StreamItem(
        _,
        _,
        item,
    )) = observation.header().key()
    else {
        return Ok(None);
    };
    if item.kind() != &BuiltinObservationName::IncarnationTransition {
        return Ok(None);
    }
    let Some((identity, projection)) = decode_lifecycle_record(*item.identity(), record)? else {
        return Ok(None);
    };
    Ok(Some(lifecycle_value(&identity, &projection)?))
}

pub(crate) fn standing_incarnation(
    record: &Record<ProductStore>,
) -> Result<Option<(NamedActorId, u64)>, String> {
    let circular_store::ClassKey::Observation(circular_store::ObservationKey::StreamItem(
        _,
        _,
        item,
    )) = record.header().key()
    else {
        return Ok(None);
    };
    if item.kind() != &BuiltinObservationName::IncarnationTransition {
        return Ok(None);
    }
    let Some((identity, _)) = decode_lifecycle_record(*item.identity(), record)? else {
        return Ok(None);
    };
    if !matches!(
        identity.phase,
        LifecyclePhase::Activated
            | LifecyclePhase::ConfigApplied
            | LifecyclePhase::Restarted
            | LifecyclePhase::ConfigRestarted
    ) {
        return Ok(None);
    }
    let circular_plan::ActorId::Scoped {
        scope,
        local: LocalKey::Named(name),
    } = identity.actor.as_actor_id()
    else {
        return Ok(None);
    };
    Ok(Some((NamedActorId::new(scope, name), identity.generation)))
}

fn lifecycle_value(
    identity: &LifecycleIdentity,
    projection: &LifecycleProjection,
) -> Result<Value, String> {
    Ok(Value::array([
        Value::string(LIFECYCLE_CARRIER),
        Value::uint(identity.phase.tag()),
        circular_store::actor_value(&identity.actor.as_actor_id())
            .map_err(|error| format!("daemon lifecycle actor is not publishable: {error}"))?,
        Value::uint(identity.generation),
        Value::uint(identity.declaration_revision),
        Value::uint(identity.config_revision),
        Value::uint(projection.at.get()),
        Value::uint(identity.detail.retired),
        Value::uint(identity.detail.prepared),
        Value::uint(identity.detail.remaining),
        Value::uint(identity.detail.next_recovery),
    ]))
}

fn decode_lifecycle_record(
    key: OpaqueId,
    record: &Record<ProductStore>,
) -> Result<Option<(LifecycleIdentity, LifecycleProjection)>, String> {
    let Record::Observation(observation) = record else {
        return Ok(None);
    };
    let ObservationFact::Lifecycle(payload) = observation.fact() else {
        return Ok(None);
    };
    let value = circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
        .map_err(|error| format!("daemon lifecycle carrier decode failed: {error}"))?;
    let Some(fields) = value.as_array() else {
        return Ok(None);
    };
    if fields.first().and_then(Value::as_str) != Some(LIFECYCLE_CARRIER) {
        return Ok(None);
    }
    if payload.version_tag().get() != LIFECYCLE_BODY_VERSION {
        return Err(format!(
            "daemon lifecycle body version {} is not readable",
            payload.version_tag().get()
        ));
    }
    if fields.len() != 11 {
        return Err("daemon lifecycle carrier has the wrong arity".to_owned());
    }
    let tag = uint(&fields[1], "phase")?;
    let phase = LifecyclePhase::from_tag(tag)
        .ok_or_else(|| format!("unknown daemon lifecycle phase {tag}"))?;
    let actor = scoped_actor(&fields[2])?;
    let generation = uint(&fields[3], "generation")?;
    let declaration_revision = uint(&fields[4], "declaration revision")?;
    let config_revision = uint(&fields[5], "config revision")?;
    let at = Tick::new(uint(&fields[6], "tick")?);
    let detail = TransitionDetail {
        retired: uint(&fields[7], "retired incarnation")?,
        prepared: uint(&fields[8], "prepared incarnation")?,
        remaining: uint(&fields[9], "restart budget remaining")?,
        next_recovery: uint(&fields[10], "next recovery tick")?,
    };
    Ok(Some((
        LifecycleIdentity {
            actor,
            generation,
            declaration_revision,
            config_revision,
            phase,
            detail,
        },
        LifecycleProjection {
            key,
            at,
            stamp: record.header().at().clone(),
        },
    )))
}

fn scoped_actor(value: &Value) -> Result<ScopedActorId, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "daemon deployment actor is not an object".to_owned())?;
    let local = object
        .get("local")
        .and_then(Value::as_str)
        .ok_or_else(|| "daemon deployment actor local is malformed".to_owned())?;
    let scope = object
        .get("scope")
        .ok_or_else(|| "daemon deployment actor scope is absent".to_owned())
        .and_then(|scope| {
            circular_store::scope_from_value(scope)
                .map_err(|error| format!("daemon deployment actor scope is malformed: {error}"))
        })?;
    Ok(ScopedActorId::new(
        scope,
        LocalKey::Named(Name::from_normalized(local)),
    ))
}

fn uint(value: &Value, field: &str) -> Result<u64, String> {
    let Value::UInt(value) = value else {
        return Err(format!("incarnation transition {field} is not uint"));
    };
    Ok(*value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_plan::ScopeId;

    use circular_core::Sequence;

    fn actor(name: &str) -> NamedActorId {
        NamedActorId::new(ScopeId::root(), Name::from_normalized(name))
    }

    #[test]
    fn old_lifecycle_bodies_with_run_are_refused() {
        for (version, extra, expected) in [
            (1, Vec::new(), "version 1 is not readable"),
            (2, vec![Value::uint(0); 4], "wrong arity"),
        ] {
            let mut fields = vec![
                Value::string("daemon-actor-lifecycle-v2"),
                Value::uint(5),
                Value::uint(289),
                circular_store::actor_value(&actor("peer").as_scoped().as_actor_id()).unwrap(),
                Value::uint(1),
                Value::uint(2),
                Value::uint(2),
                Value::uint(3),
            ];
            fields.extend(extra);
            let payload = circular_core::encode(
                &Value::array(fields),
                Ceilings::for_boundary(Boundary::Journal),
            )
            .unwrap();
            let record = Record::Observation(ObservationRecord::lifecycle(
                Stamp::from_system_record_producer_at(
                    circular_core::Hlc::from_physical(Tick::new(3)),
                    circular_plan::ActorId::System(circular_plan::SystemActor::Pipeline),
                    Sequence::new(1).unwrap(),
                    circular_core::RevisionEpochId::new(2).unwrap(),
                )
                .unwrap(),
                ObservationBucket::from_millis(3),
                RecordOrigin::Actor(IncarnationId::new(1)),
                ObservationItemKey::new(
                    BuiltinObservationName::IncarnationTransition,
                    OpaqueId::new(0),
                ),
                EncodedPayload::new(PayloadVersionTag::new(version).unwrap(), &payload),
            ));
            let error = decode_lifecycle_record(OpaqueId::new(0), &record).unwrap_err();
            assert!(error.contains(expected), "an old body is refused: {error}");
        }
    }
}
