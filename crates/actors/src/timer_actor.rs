
use crate::actor_support::error_payload;
use crate::config::{ConfigRejection, NonZeroInterval, Slot};
use crate::{
    ActorType, BaseShape, ERROR_PORT_NAME, FieldMap, GroundShape, Name, ProductPayload,
    ProductValue, Shape, TIMER_PORT_NAME,
};
#[cfg(test)]
use circular_core::Tick;
use circular_core::{NonZeroMillis, PortId};
use circular_runtime::{
    ActorContext, ActorEffect, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, Effect, EffectOutcome, EmittingActor, EmittingActorFactory,
    FoldedConfig, OutcomePayload, ReasonDecl, ScheduleCorrelation, ScheduleSpec, Suppression,
};
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::sync::LazyLock;

pub const TIMER_STATE_SCHEMA: u16 = 1;

pub const STALE_TIMER_FIRE: &str = "stale_timer_fire";

static SUPPRESSION_REASONS: LazyLock<ReasonDecl<Suppression>> = LazyLock::new(|| {
    ReasonDecl::try_from_names([STALE_TIMER_FIRE])
        .expect("the timer suppression reason is one and not empty")
});

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered timer port names are canonical")
}

fn tick_payload(sequence: u64) -> ProductPayload {
    let shape = GroundShape::try_new(Shape::Object {
        fields: FieldMap::try_new(vec![(
            Name::from_static("sequence"),
            Shape::Base(BaseShape::UInt),
        )])
        .expect("the tick payload has one field"),
        open: false,
    })
    .expect("the tick payload shape has no variable");
    let value = ProductValue::object([("sequence", ProductValue::UInt(sequence))])
        .expect("the tick payload has one field");
    ProductPayload::new(shape, value)
}

pub(crate) const EVERY: Slot<NonZeroInterval> = Slot::new("every", NonZeroInterval);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimerFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
}

impl fmt::Display for TimerFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("timer config was folded for another actor"),
            Self::Config(rejection) => rejection.fmt(formatter),
        }
    }
}

impl Error for TimerFactoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(rejection) => Some(rejection),
            Self::InvalidConfig => None,
        }
    }
}

impl From<crate::config::ConfigRejection> for TimerFactoryError {
    fn from(rejection: crate::config::ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub struct TimerActor<V, I> {
    every: NonZeroMillis,
    generation: u64,
    sequence: u64,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> TimerActor<V, I> {
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    fn schedule(&self) -> ActorEffects<ProductPayload> {
        ActorEffects::external(Effect::schedule(ScheduleSpec::new(
            self.every,
            ScheduleCorrelation::new(self.generation),
        )))
    }

    fn arm_new_generation(&mut self) -> ActorEffects<ProductPayload> {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("timer re-arm generation exhausted its u64 space");
        self.schedule()
    }

    fn fire(&mut self) -> ActorEffects<ProductPayload> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .expect("timer sequence exhausted its u64 space");
        ActorEffects::emit(port("tick"), tick_payload(self.sequence)).concat(self.schedule())
    }

    fn suppress_stale() -> ActorEffects<ProductPayload> {
        let reason = SUPPRESSION_REASONS
            .resolve(STALE_TIMER_FIRE)
            .expect("the timer reason name and its declaration come from the same constant");
        ActorEffects::singleton(ActorEffect::suppress(reason))
    }
}

impl<V, I> EditableActor for TimerActor<V, I>
where
    V: Clone + From<u16> + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }

    fn checkpoint(&self) -> Option<ActorState<Self::StateVersion>> {
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&self.generation.to_be_bytes());
        bytes.extend_from_slice(&self.sequence.to_be_bytes());
        Some(ActorState::new(V::from(TIMER_STATE_SCHEMA), bytes))
    }

    fn restore(
        &mut self,
        state: ActorState<Self::StateVersion>,
    ) -> Result<(), ActorRestoreError<Self::StateVersion>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(TIMER_STATE_SCHEMA) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let Ok(bytes) = <[u8; 16]>::try_from(bytes.as_ref()) else {
            return Err(ActorRestoreError::DecodeFailed { schema });
        };
        let generation = u64::from_be_bytes(bytes[..8].try_into().expect("first 8 bytes"));
        let sequence = u64::from_be_bytes(bytes[8..].try_into().expect("last 8 bytes"));
        self.generation = generation;
        self.sequence = sequence;
        Ok(())
    }
}

impl<T> EmittingActor<T, ProductPayload> for TimerActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        if input.inlet() == &port("bang") {
            return self.arm_new_generation();
        }
        if input.inlet() != &port(TIMER_PORT_NAME) {
            return ActorEffects::emit(
                port(ERROR_PORT_NAME),
                error_payload("timer received an unknown inlet"),
            );
        }
        let ProductValue::UInt(correlation) = input.payload::<T>().value() else {
            return ActorEffects::emit(
                port(ERROR_PORT_NAME),
                error_payload("timer correlation payload is not UInt"),
            );
        };
        if *correlation != self.generation {
            return Self::suppress_stale();
        }
        self.fire()
    }

    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        match outcome.result() {
            Ok(OutcomePayload::ScheduleArmed(_)) => ActorEffects::empty(),
            Ok(other) => ActorEffects::emit(
                port(ERROR_PORT_NAME),
                error_payload(format!(
                    "timer received an unexpected outcome: {}",
                    other.kind_tag()
                )),
            ),
            Err(failure) => ActorEffects::emit(
                port(ERROR_PORT_NAME),
                error_payload(format!("timer scheduling failed: {}", failure.kind_tag())),
            ),
        }
    }
}

pub struct TimerFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for TimerFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: circular_core::ActorType = circular_core::ActorType::Timer;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = TimerActor<T::StateVersion, T::EffectId>;
    type Error = TimerFactoryError;

    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        let value = config
            .for_type(ActorType::Timer)
            .map_err(|_| TimerFactoryError::InvalidConfig)?;
        let schema = crate::registration(ActorType::Timer).spec().config();
        let mut fields = schema.open(value)?;
        let every = schema.read(&mut fields, &EVERY)?;
        Ok(TimerActor {
            every,
            generation: 0,
            sequence: 0,
            marker: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_plan::{
        ActorId, Config, Generation, GenerationVector, Incarnation, Name as PlanName, NamedActorId,
        ScopeId,
    };
    use circular_runtime::{ActorEffect, EffectFailure};

    use circular_testkit::types::TestRun;

    type TestTypes = circular_testkit::types::TestTypes<ProductPayload>;

    fn config(value: ProductValue) -> FoldedConfig {
        FoldedConfig::minted(ActorType::Timer, value)
    }

    fn every(value: ProductValue) -> FoldedConfig {
        config(ProductValue::object([("every", value)]).expect("one config key"))
    }

    fn actor() -> TimerActor<u16, u64> {
        <TimerFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
            &every(ProductValue::UInt(100)),
            &(),
        )
        .expect("100ms timer")
    }

    fn payload(value: ProductValue) -> ProductPayload {
        ProductPayload::new(
            GroundShape::try_new(Shape::Any).expect("Any is a ground shape"),
            value,
        )
    }

    fn context<'a>(
        actor: &'a ActorId,
        incarnation: &'a Incarnation<TestRun>,
        config: &'a Config,
    ) -> ActorContext<'a, TestRun, ()> {
        ActorContext::new(actor, incarnation, config, &())
    }

    fn drive<Ret>(f: impl FnOnce(&ActorContext<'_, TestRun, ()>) -> Ret) -> Ret {
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("timer"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor = named.as_actor_id();
        let config = Config::default();
        let context = context(&actor, &incarnation, &config);
        f(&context)
    }

    fn event(
        actor: &mut TimerActor<u16, u64>,
        context: &ActorContext<'_, TestRun, ()>,
        inlet: &str,
        value: ProductValue,
    ) -> ActorEffects<ProductPayload> {
        <TimerActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
            actor,
            &ActorInput::new(port(inlet), payload(value)),
            context,
        )
    }

    fn schedule(effects: &ActorEffects<ProductPayload>) -> Option<ScheduleSpec> {
        effects.iter().find_map(|effect| match effect {
            ActorEffect::External(Effect::Schedule { spec }) => Some(*spec),
            _ => None,
        })
    }

    fn assert_tick_and_rearm(
        effects: &ActorEffects<ProductPayload>,
        sequence: u64,
        generation: u64,
    ) {
        let [
            ActorEffect::Emit { port, payload, .. },
            ActorEffect::External(Effect::Schedule { spec }),
        ] = effects.as_slice()
        else {
            panic!("the current wake yields only one tick and one reschedule");
        };
        assert_eq!(port.as_str(), "tick");
        assert_eq!(
            payload.value(),
            &ProductValue::object([("sequence", ProductValue::UInt(sequence))])
                .expect("tick object")
        );
        assert_eq!(spec.after().get().get(), 100);
        assert_eq!(spec.correlation().get(), generation);
    }

    #[test]
    fn config_rejects_missing_zero_negative_fractional_and_unknown_fields() {
        let create = |config: &FoldedConfig| {
            <TimerFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(config, &())
                .map(|_| ())
        };
        assert_eq!(
            create(&config(
                ProductValue::object(std::iter::empty::<(&str, ProductValue)>())
                    .expect("empty config")
            )),
            Err(TimerFactoryError::Config(ConfigRejection::Missing("every")))
        );
        let every_rejected = |error| {
            Err(TimerFactoryError::Config(ConfigRejection::Slot {
                slot: "every",
                error,
            }))
        };
        assert_eq!(
            create(&every(ProductValue::UInt(0))),
            every_rejected(crate::config::ConfigDecodeError::ZeroInterval)
        );
        assert_eq!(
            create(&every(ProductValue::Int(-3))),
            every_rejected(crate::config::ConfigDecodeError::NegativeInterval { actual: -3 })
        );
        assert_eq!(
            create(&every(ProductValue::Float(circular_core::FloatValue::new(
                1.5
            )))),
            every_rejected(crate::config::ConfigDecodeError::IntervalKind {
                actual: circular_core::ValueKind::Float
            })
        );
        assert_eq!(
            create(&config(
                ProductValue::object([
                    ("every", ProductValue::Int(100)),
                    ("extra", ProductValue::Null),
                ])
                .expect("distinct config keys")
            )),
            Err(TimerFactoryError::Config(
                crate::config::ConfigRejection::Unknown(circular_core::UnknownField {
                    at: circular_core::FieldPath::root(),
                    key: "extra".to_owned(),
                })
            ))
        );
        assert!(create(&every(ProductValue::Int(100))).is_ok());
        assert!(create(&every(ProductValue::UInt(100))).is_ok());
    }

    #[test]
    fn bang_arms_without_opening_payload() {
        drive(|context| {
            let mut actor = actor();
            let effects = event(
                &mut actor,
                context,
                "bang",
                ProductValue::String("ignored".to_owned()),
            );
            let spec = schedule(&effects).expect("yields one Schedule");
            assert_eq!(spec.after().get().get(), 100);
            assert_eq!(spec.correlation().get(), 1);
            assert_eq!(actor.generation(), 1);
            assert_eq!(actor.sequence(), 0);
        });
    }

    #[test]
    fn current_fire_emits_one_tick_and_rearms_the_same_generation() {
        drive(|context| {
            let mut actor = actor();
            event(&mut actor, context, "bang", ProductValue::Null);
            let effects = event(&mut actor, context, TIMER_PORT_NAME, ProductValue::UInt(1));
            assert_eq!(effects.len(), 2);
            let ActorEffect::Emit { port, payload, .. } = &effects.as_slice()[0] else {
                panic!("the first term is tick");
            };
            assert_eq!(port.as_str(), "tick");
            assert_eq!(
                payload.value(),
                &ProductValue::object([("sequence", ProductValue::UInt(1))]).expect("tick object")
            );
            assert_eq!(
                schedule(&effects)
                    .expect("the same fire re-arms")
                    .correlation()
                    .get(),
                1
            );
            assert_eq!(actor.sequence(), 1);
        });
    }

    #[test]
    fn stale_fire_is_observed_as_suppression_without_emission_or_rearm() {
        drive(|context| {
            let mut actor = actor();
            event(&mut actor, context, "bang", ProductValue::Null);
            let effects = event(&mut actor, context, TIMER_PORT_NAME, ProductValue::UInt(0));
            assert!(matches!(
                effects.as_slice(),
                [ActorEffect::Suppress { reason }] if reason.name() == STALE_TIMER_FIRE
            ));
            assert_eq!(actor.sequence(), 0);
        });
    }

    #[test]
    fn repeated_bangs_only_allow_the_latest_generation_to_emit() {
        drive(|context| {
            for bang_count in [2_u64, 8] {
                let mut actor = actor();
                for generation in 1..=bang_count {
                    let armed = event(&mut actor, context, "bang", ProductValue::Null);
                    assert!(matches!(
                        armed.as_slice(),
                        [ActorEffect::External(Effect::Schedule { .. })]
                    ));
                    let spec = schedule(&armed).expect("bang yields only a new schedule");
                    assert_eq!(spec.after().get().get(), 100);
                    assert_eq!(spec.correlation().get(), generation);
                    if generation > 1 {
                        let stale = event(
                            &mut actor,
                            context,
                            TIMER_PORT_NAME,
                            ProductValue::UInt(generation - 1),
                        );
                        assert!(matches!(
                            stale.as_slice(),
                            [ActorEffect::Suppress { reason }] if reason.name() == STALE_TIMER_FIRE
                        ));
                    }
                    assert_eq!(actor.sequence(), 0);
                }

                let current = event(
                    &mut actor,
                    context,
                    TIMER_PORT_NAME,
                    ProductValue::UInt(bang_count),
                );
                assert_tick_and_rearm(&current, 1, bang_count);
                assert_eq!(actor.generation(), bang_count);
                assert_eq!(actor.sequence(), 1);
            }
        });
    }

    #[test]
    fn bang_and_fire_order_at_the_same_context_sample_changes_the_output() {
        drive(|context| {
            let mut bang_first = actor();
            let mut fire_first = actor();
            for actor in [&mut bang_first, &mut fire_first] {
                event(actor, context, "bang", ProductValue::Null);
            }

            let rearmed = event(&mut bang_first, context, "bang", ProductValue::Null);
            assert!(matches!(
                rearmed.as_slice(),
                [ActorEffect::External(Effect::Schedule { .. })]
            ));
            let stale = event(
                &mut bang_first,
                context,
                TIMER_PORT_NAME,
                ProductValue::UInt(1),
            );
            assert!(matches!(
                stale.as_slice(),
                [ActorEffect::Suppress { reason }] if reason.name() == STALE_TIMER_FIRE
            ));

            let emitted = event(
                &mut fire_first,
                context,
                TIMER_PORT_NAME,
                ProductValue::UInt(1),
            );
            assert_tick_and_rearm(&emitted, 1, 1);
            let rearmed = event(&mut fire_first, context, "bang", ProductValue::Null);
            assert!(matches!(
                rearmed.as_slice(),
                [ActorEffect::External(Effect::Schedule { .. })]
            ));
            assert_eq!(bang_first.generation(), 2);
            assert_eq!(fire_first.generation(), 2);
            assert_eq!(bang_first.sequence(), 0);
            assert_eq!(fire_first.sequence(), 1);

            for (actor, next_sequence) in [(&mut bang_first, 1), (&mut fire_first, 2)] {
                let next = event(actor, context, TIMER_PORT_NAME, ProductValue::UInt(2));
                assert_tick_and_rearm(&next, next_sequence, 2);
            }
        });
    }

    #[test]
    fn wake_context_sample_does_not_change_single_tick_or_full_rearm() {
        drive(|context| {
            for sample in [0, 100, 350, 10_000] {
                let mut actor = actor();
                event(&mut actor, context, "bang", ProductValue::Null);
                let wake_context = ActorContext::new(
                    context.id(),
                    context.incarnation(),
                    context.config(),
                    context.grants(),
                );
                assert_eq!(actor.sequence(), 0);
                let effects = event(
                    &mut actor,
                    &wake_context,
                    TIMER_PORT_NAME,
                    ProductValue::UInt(1),
                );
                assert_tick_and_rearm(&effects, 1, 1);
                assert_eq!(actor.sequence(), 1);
            }
        });
    }

    #[test]
    fn schedule_receipt_is_silent_and_failure_emits_error() {
        drive(|context| {
            let mut actor = actor();
            let armed =
                <TimerActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
                    &mut actor,
                    &EffectOutcome::new(
                        1,
                        Ok(OutcomePayload::ScheduleArmed(ScheduleCorrelation::new(1))),
                    ),
                    context,
                );
            assert!(armed.is_empty());
            let failed =
                <TimerActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
                    &mut actor,
                    &EffectOutcome::new(2, Err(EffectFailure::EndpointGone)),
                    context,
                );
            assert!(matches!(
                failed.as_slice(),
                [ActorEffect::Emit { port, .. }] if port.as_str() == ERROR_PORT_NAME
            ));
        });
    }

    #[test]
    fn checkpoint_round_trips_generation_and_sequence_without_a_schedule() {
        drive(|context| {
            let mut original = actor();
            event(&mut original, context, "bang", ProductValue::Null);
            event(
                &mut original,
                context,
                TIMER_PORT_NAME,
                ProductValue::UInt(1),
            );
            let checkpoint = original.checkpoint().expect("timer carries state");

            let mut restored = actor();
            restored
                .restore(checkpoint)
                .expect("restore under the current schema");
            assert_eq!(restored.generation(), 1);
            assert_eq!(restored.sequence(), 1);
            let rearmed = event(&mut restored, context, "bang", ProductValue::Null);
            assert_eq!(
                schedule(&rearmed)
                    .expect("bang after restore")
                    .correlation()
                    .get(),
                2
            );
        });
    }

    #[test]
    fn checkpoint_bytes_and_restore_rejections_are_literal() {
        drive(|context| {
            let mut actor = actor();
            event(&mut actor, context, "bang", ProductValue::Null);
            event(&mut actor, context, "_timer", ProductValue::UInt(1));
            let (schema, bytes) = actor.checkpoint().expect("checkpoint").into_parts();
            assert_eq!(schema, 1);
            assert_eq!(
                bytes.as_ref(),
                &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1]
            );
            for (schema, bytes, expected) in [
                (
                    2,
                    vec![0; 16],
                    ActorRestoreError::SchemaBeyondLadder { schema: 2 },
                ),
                (
                    1,
                    vec![0; 15],
                    ActorRestoreError::DecodeFailed { schema: 1 },
                ),
                (
                    1,
                    vec![0; 17],
                    ActorRestoreError::DecodeFailed { schema: 1 },
                ),
            ] {
                assert_eq!(actor.restore(ActorState::new(schema, bytes)), Err(expected));
                assert_eq!((actor.generation(), actor.sequence()), (1, 1));
            }
            actor
                .restore(ActorState::new(
                    1,
                    vec![0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 9],
                ))
                .expect("independent literal state");
            assert_eq!((actor.generation(), actor.sequence()), (7, 9));
            let rearmed = event(&mut actor, context, "bang", ProductValue::Null);
            let spec = schedule(&rearmed).expect("new generation");
            assert_eq!(spec.after().get().get(), 100);
            assert_eq!(spec.correlation().get(), 8);
            assert_eq!(actor.sequence(), 9);
        });
    }

    #[test]
    fn invalid_input_and_failed_schedule_keep_state_and_exact_error_copy() {
        drive(|context| {
            let mut actor = actor();
            event(&mut actor, context, "bang", ProductValue::Null);
            for (inlet, value, message) in [
                (
                    "unknown",
                    ProductValue::Null,
                    "timer received an unknown inlet",
                ),
                (
                    "_timer",
                    ProductValue::Int(1),
                    "timer correlation payload is not UInt",
                ),
            ] {
                let effects = event(&mut actor, context, inlet, value);
                let [ActorEffect::Emit { port, payload, .. }] = effects.as_slice() else {
                    panic!("one error emission");
                };
                assert_eq!(port.as_str(), "_error");
                assert_eq!(payload.value(), &ProductValue::String(message.to_owned()));
                assert_eq!((actor.generation(), actor.sequence()), (1, 0));
            }
            let failed =
                <TimerActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
                    &mut actor,
                    &EffectOutcome::new(1, Err(EffectFailure::EndpointGone)),
                    context,
                );
            let [ActorEffect::Emit { port, payload, .. }] = failed.as_slice() else {
                panic!("one error emission");
            };
            assert_eq!(port.as_str(), "_error");
            assert_eq!(
                payload.value(),
                &ProductValue::String("timer scheduling failed: endpoint_gone".to_owned())
            );
            assert_eq!((actor.generation(), actor.sequence()), (1, 0));
        });
    }
}
