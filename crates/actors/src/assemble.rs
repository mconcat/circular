use crate::actor_support::{StampedEvent, config_unsigned, error_payload};
use crate::config::{PayloadPath, PositiveCount, Slot, payload_path_from_value};
use crate::{ActorType, FieldMap, GroundShape, ProductPayload, ProductValue, Shape};
use circular_core::{NonZeroMillis, PortId};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorTypes, EditableActor, Effect, EffectOutcome,
    EmittingActor, EmittingActorFactory, FoldedConfig, OutcomePayload, ScheduleCorrelation,
    ScheduleSpec,
};
use std::{collections::BTreeMap, marker::PhantomData};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssembleConfigError(pub String);
impl std::fmt::Display for AssembleConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for AssembleConfigError {}

#[derive(Clone, Debug)]
pub struct AssembleConfig {
    at: PayloadPath,
    inactivity: u64,
    maximum: u64,
    capacity: u64,
}
pub(crate) const CAPACITY: Slot<PositiveCount> = Slot::new("capacity", PositiveCount);

impl AssembleConfig {
    pub fn from_value(value: &ProductValue) -> Result<Self, AssembleConfigError> {
        let fail = || {
            AssembleConfigError(
                "assemble requires at, positive capacity and positive inactivity_timeout <= max_window".into(),
            )
        };
        let object = value.as_object().ok_or_else(fail)?;
        if object.len() != 4 {
            return Err(fail());
        }
        let at = payload_path_from_value(object.get("at").ok_or_else(fail)?).map_err(|_| fail())?;
        let duration = |key| {
            object
                .get(key)
                .and_then(config_unsigned)
                .filter(|n| *n > 0)
                .ok_or_else(fail)
        };
        let inactivity = duration("inactivity_timeout")?;
        let maximum = duration("max_window")?;
        if inactivity > maximum {
            return Err(fail());
        }
        let capacity = circular_core::Fields::open(value)
            .ok()
            .and_then(|mut fields| {
                crate::get(ActorType::Assemble)
                    .config()
                    .read(&mut fields, &CAPACITY)
                    .ok()
            })
            .ok_or_else(fail)?
            .get();
        Ok(Self {
            at,
            inactivity,
            maximum,
            capacity,
        })
    }
}

static DEAD_LETTER_REASONS: std::sync::LazyLock<
    circular_runtime::ReasonDecl<circular_runtime::DeadLettering>,
> = std::sync::LazyLock::new(|| {
    circular_runtime::ReasonDecl::try_from_names(["capacity"])
        .expect("capacity is a declared dead-letter reason")
});

struct Window {
    events: Vec<ProductValue>,
    started: u64,
    last: u64,
    correlation: u64,
}
pub struct AssembleActor<V, I> {
    config: AssembleConfig,
    windows: BTreeMap<String, Window>,
    correlation: u64,
    marker: PhantomData<fn() -> (V, I)>,
}
fn port(name: &str) -> PortId {
    PortId::try_new(name).unwrap()
}
fn payload(value: ProductValue) -> ProductPayload {
    ProductPayload::new(
        GroundShape::try_new(Shape::Object {
            fields: FieldMap::try_new(Vec::new()).unwrap(),
            open: true,
        })
        .unwrap(),
        value,
    )
}
impl<V, I> AssembleActor<V, I> {
    fn deadline(&self, window: &Window) -> (u64, bool) {
        let quiet = window.last.saturating_add(self.config.inactivity);
        let maximum = window.started.saturating_add(self.config.maximum);
        if quiet <= maximum {
            (quiet, false)
        } else {
            (maximum.saturating_add(1), true)
        }
    }
    fn close(&mut self, key: &str, stuck: bool) -> ActorEffects<ProductPayload> {
        let window = self.windows.remove(key).expect("owned window");
        ActorEffects::emit(
            port("event"),
            payload(
                ProductValue::object([
                    ("key", ProductValue::string(key)),
                    ("events", ProductValue::array(window.events)),
                    ("stuck", ProductValue::bool(stuck)),
                ])
                .unwrap(),
            ),
        )
    }
    fn schedule(&self, key: &str, now: u64) -> ActorEffects<ProductPayload> {
        let window = &self.windows[key];
        ActorEffects::external(Effect::schedule(ScheduleSpec::new(
            NonZeroMillis::new(self.deadline(window).0.saturating_sub(now).max(1)).unwrap(),
            ScheduleCorrelation::new(window.correlation),
        )))
    }
}
impl<V: Clone, I: Clone + Ord> EditableActor for AssembleActor<V, I> {
    type StateVersion = V;
    type EffectId = I;
}
impl<T> EmittingActor<T, ProductPayload> for AssembleActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Event: StampedEvent,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let Some(at) = input.event().recorded_instant() else {
            return ActorEffects::emit(
                port("_error"),
                error_payload("assemble requires recorded arrival time"),
            );
        };
        let now = at.millis();
        let value = input.payload::<T>().value();
        if input.inlet().as_str() == "_timer" {
            let ProductValue::UInt(correlation) = value else {
                return ActorEffects::emit(
                    port("_error"),
                    error_payload("assemble timer correlation must be UInt"),
                );
            };
            let key = self.windows.iter().find_map(|(key, window)| {
                (window.correlation == *correlation).then(|| key.clone())
            });
            let Some(key) = key else {
                return ActorEffects::empty();
            };
            let (deadline, stuck) = self.deadline(&self.windows[&key]);
            if now < deadline {
                return self.schedule(&key, now);
            }
            return self.close(&key, stuck);
        }
        let key = value
            .as_object()
            .and_then(|_| crate::route_config::select(value, &self.config.at))
            .and_then(ProductValue::as_str)
            .filter(|key| !key.is_empty());
        let Some(key) = key.filter(|_| input.inlet().as_str() == "event") else {
            return ActorEffects::emit(
                port("_error"),
                error_payload("assemble requires an object with a nonempty string key"),
            );
        };
        let mut effects = ActorEffects::empty();
        if let Some(window) = self.windows.get(key) {
            let (deadline, stuck) = self.deadline(window);
            if now >= deadline {
                effects = self.close(key, stuck);
            }
        }
        if !self.windows.contains_key(key) && self.windows.len() as u64 >= self.config.capacity {
            let reasons = &*DEAD_LETTER_REASONS;
            return effects.concat(ActorEffects::singleton(
                circular_runtime::ActorEffect::dead_letter(
                    input.payload::<T>().clone(),
                    reasons.resolve("capacity").unwrap(),
                ),
            ));
        }
        self.correlation = self
            .correlation
            .checked_add(1)
            .expect("assemble correlation exhausted");
        let window = self
            .windows
            .entry(key.to_owned())
            .or_insert_with(|| Window {
                events: Vec::new(),
                started: now,
                last: now,
                correlation: 0,
            });
        window.events.push(value.clone());
        window.last = now;
        window.correlation = self.correlation;
        effects.concat(self.schedule(key, now))
    }
    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        _: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        match outcome.result() {
            Ok(OutcomePayload::ScheduleArmed(_)) => ActorEffects::empty(),
            _ => ActorEffects::emit(port("_error"), error_payload("assemble scheduling failed")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{StreamIdentity, Tick};
    use circular_plan::{
        Config, Generation, GenerationVector, Incarnation, Name, NamedActorId, ScopeId,
    };
    use circular_runtime::ActorEffect;
    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct Run;
    impl StreamIdentity for Run {}
    struct Types;
    impl ActorTypes for Types {
        type Stream = Run;
        type Event = circular_core::Event<
            Run,
            circular_plan::ActorId,
            ProductPayload,
            std::convert::Infallible,
        >;
        type Payload = ProductPayload;
        type EffectId = circular_runtime::EffectId;
        type StateVersion = u16;
        type Observation = ();
        type Grants = ();
        fn payload(e: &Self::Event) -> &Self::Payload {
            e.payload()
        }
    }
    fn config(quiet: u64, max: u64, capacity: i64) -> ProductValue {
        ProductValue::object([
            ("at", ProductValue::array([ProductValue::string("key")])),
            ("inactivity_timeout", ProductValue::uint(quiet)),
            ("max_window", ProductValue::uint(max)),
            ("capacity", ProductValue::int(capacity)),
        ])
        .unwrap()
    }
    fn actor(
        quiet: u64,
        max: u64,
        capacity: usize,
    ) -> AssembleActor<u16, circular_runtime::EffectId> {
        AssembleFactory::<Types>::create(
            &FoldedConfig::minted(ActorType::Assemble, config(quiet, max, capacity as i64)),
            &(),
        )
        .unwrap()
    }
    fn event(key: &str, n: i64) -> ProductValue {
        ProductValue::object([
            ("key", ProductValue::string(key)),
            ("n", ProductValue::int(n)),
        ])
        .unwrap()
    }
    fn deliver(
        actor: &mut AssembleActor<u16, circular_runtime::EffectId>,
        at: u64,
        inlet: &str,
        value: ProductValue,
    ) -> ActorEffects<ProductPayload> {
        deliver_recorded(actor, Some(at), inlet, value)
    }
    fn deliver_recorded(
        actor: &mut AssembleActor<u16, circular_runtime::EffectId>,
        at: Option<u64>,
        inlet: &str,
        value: ProductValue,
    ) -> ActorEffects<ProductPayload> {
        let named = NamedActorId::new(ScopeId::root(), Name::from_normalized("assemble"));
        let incarnation = Incarnation::new(
            Run,
            named.as_scoped().clone(),
            GenerationVector::for_actor(
                named.as_scoped(),
                vec![Generation::new(0)].into_boxed_slice(),
            )
            .unwrap(),
        );
        let actor_id = named.as_actor_id();
        let config = Config::default();
        let context = ActorContext::new(&actor_id, &incarnation, &config, &());
        let stamp = circular_core::Stamp::from_event_producer(
            Tick::new(999_999),
            named,
            circular_core::Sequence::new(1).unwrap(),
            circular_core::RevisionEpochId::new(1).unwrap(),
        );
        let event = circular_core::admit(
            Run,
            circular_core::Emission::from_runtime(
                circular_core::emit(payload(value)),
                circular_core::Causality::Source,
                None,
            ),
            stamp,
        )
        .unwrap()
        .with_recorded_instant(at.map(circular_core::RecordedInstant::from_millis));
        EmittingActor::<Types, ProductPayload>::on_event(
            actor,
            &ActorInput::new(port(inlet), event),
            &context,
        )
    }
    fn fire(
        actor: &mut AssembleActor<u16, circular_runtime::EffectId>,
        at: u64,
        correlation: u64,
    ) -> ActorEffects<ProductPayload> {
        deliver(actor, at, "_timer", ProductValue::uint(correlation))
    }
    fn outputs(effects: ActorEffects<ProductPayload>) -> Vec<ProductValue> {
        effects
            .into_iter()
            .filter_map(|e| {
                if let ActorEffect::Emit { port, payload, .. } = e {
                    assert_eq!(port.as_str(), "event");
                    Some(payload.value().clone())
                } else {
                    None
                }
            })
            .collect()
    }
    fn expected(events: Vec<ProductValue>, stuck: bool) -> ProductValue {
        ProductValue::object([
            ("key", ProductValue::string("a")),
            ("events", ProductValue::array(events)),
            ("stuck", ProductValue::bool(stuck)),
        ])
        .unwrap()
    }
    #[test]
    fn missing_recorded_time_is_not_fabricated_from_hlc_or_context() {
        let mut actor = actor(10, 100, 2);
        let refused = deliver_recorded(&mut actor, None, "event", event("a", 1));
        let [circular_runtime::ActorEffect::Emit { port, payload, .. }] = refused.as_slice() else {
            panic!("missing time is an explicit error")
        };
        assert_eq!(port.as_str(), "_error");
        assert_eq!(
            payload.value(),
            &ProductValue::string("assemble requires recorded arrival time")
        );
        assert!(actor.windows.is_empty());
        let _ = deliver(&mut actor, 0, "event", event("a", 1));
        assert_eq!(
            actor.windows["a"].started, 0,
            "recorded zero is present, not missing"
        );
    }

    #[test]
    fn inactivity_rearms_and_stale_callbacks_cannot_close_a_window_twice() {
        let mut actor = actor(3000, 30000, 2);
        deliver(&mut actor, 0, "event", event("a", 1));
        deliver(&mut actor, 2000, "event", event("a", 2));
        assert!(fire(&mut actor, 3000, 1).is_empty());
        assert_eq!(
            outputs(fire(&mut actor, 5000, 2)),
            [expected(vec![event("a", 1), event("a", 2)], false)]
        );
        assert!(fire(&mut actor, 5001, 2).is_empty());
        assert!(actor.windows.is_empty());
    }
    #[test]
    fn equal_deadlines_close_normally_and_only_maximum_excess_is_stuck() {
        let mut tied = actor(3000, 3000, 2);
        deliver(&mut tied, 0, "event", event("a", 1));
        assert_eq!(
            outputs(fire(&mut tied, 3000, 1)),
            [expected(vec![event("a", 1)], false)]
        );
        let mut busy = actor(3000, 5000, 2);
        deliver(&mut busy, 0, "event", event("a", 1));
        deliver(&mut busy, 2500, "event", event("a", 2));
        assert!(outputs(fire(&mut busy, 5000, 2)).is_empty());
        assert_eq!(
            outputs(fire(&mut busy, 5001, 2)),
            [expected(vec![event("a", 1), event("a", 2)], true)]
        );
    }
    #[test]
    fn capacity_rejects_only_new_keys_and_keeps_the_original_subject() {
        let mut actor = actor(3000, 30000, 1);
        deliver(&mut actor, 0, "event", event("a", 1));
        let rejected = deliver(&mut actor, 1, "event", event("b", 2));
        let [ActorEffect::DeadLetter { subject, reason }] = rejected.as_slice() else {
            panic!("capacity must retain the input in a dead letter");
        };
        assert_eq!(subject.value(), &event("b", 2));
        assert_eq!(reason.name(), "capacity");
        deliver(&mut actor, 2, "event", event("a", 3));
        assert_eq!(actor.windows["a"].events, [event("a", 1), event("a", 3)]);
    }
    #[test]
    fn replaying_recorded_inputs_restores_the_same_derived_window_without_checkpoint_bytes() {
        let run = || {
            let mut actor = actor(3000, 30000, 2);
            deliver(&mut actor, 10, "event", event("a", 1));
            deliver(&mut actor, 20, "event", event("a", 2));
            assert!(actor.checkpoint().is_none());
            outputs(fire(&mut actor, 3020, 2))
        };
        let expected = [expected(vec![event("a", 1), event("a", 2)], false)];
        assert_eq!(run(), expected);
        assert_eq!(run(), expected);
    }
    #[test]
    fn capacity_is_authored_required_and_uses_the_registered_positive_integer_domain() {
        let slot = crate::get(ActorType::Assemble)
            .config()
            .top_level_slot("capacity")
            .unwrap();
        assert!(matches!(
            slot.required(),
            crate::config::Required::Mandatory
        ));
        for capacity in [
            None,
            Some(ProductValue::int(0)),
            Some(ProductValue::int(-1)),
            Some(ProductValue::float(1.5)),
            Some(ProductValue::uint(2)),
            Some(ProductValue::string("2")),
        ] {
            let mut fields = vec![
                ("at", ProductValue::array([ProductValue::string("key")])),
                ("inactivity_timeout", ProductValue::int(3000)),
                ("max_window", ProductValue::int(30000)),
            ];
            if let Some(value) = capacity {
                fields.push(("capacity", value));
            }
            let config =
                FoldedConfig::minted(ActorType::Assemble, ProductValue::object(fields).unwrap());
            assert!(AssembleFactory::<Types>::create(&config, &()).is_err());
        }
        let actor = actor(3000, 30000, 1);
        assert!(actor.checkpoint().is_none());
        assert!(actor.windows.is_empty());
    }

    #[test]
    fn config_and_bad_inputs_do_not_create_windows() {
        assert!(AssembleConfig::from_value(&config(0, 30000, 2)).is_err());
        assert!(AssembleConfig::from_value(&config(30001, 30000, 2)).is_err());
        let mut actor = actor(3000, 30000, 2);
        for value in [ProductValue::Null, event("", 1)] {
            let effects = deliver(&mut actor, 0, "event", value);
            assert!(
                matches!(effects.as_slice(), [ActorEffect::Emit { port, .. }] if port.as_str() == "_error")
            );
        }
        assert!(actor.windows.is_empty());
    }
}

pub struct AssembleFactory<T>(PhantomData<fn() -> T>);
impl<T> EmittingActorFactory<ProductPayload> for AssembleFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Event: StampedEvent,
{
    const TYPE: ActorType = ActorType::Assemble;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = AssembleActor<T::StateVersion, T::EffectId>;
    type Error = AssembleConfigError;
    fn create(config: &FoldedConfig, _: &Self::Grants) -> Result<Self::Instance, Self::Error> {
        Ok(AssembleActor {
            config: declared_config(config)?,
            windows: BTreeMap::new(),
            correlation: 0,
            marker: PhantomData,
        })
    }
}

fn declared_config(config: &FoldedConfig) -> Result<AssembleConfig, AssembleConfigError> {
    let value = config
        .for_type(ActorType::Assemble)
        .map_err(|e| AssembleConfigError(e.to_string()))?;
    AssembleConfig::from_value(value)
}

pub(crate) fn judge(
    config: &FoldedConfig,
    _inlets: &crate::inlet_shapes::ResolvedInletShapes,
) -> Result<(), AssembleConfigError> {
    declared_config(config).map(drop)
}
