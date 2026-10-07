
use crate::arming::ArmingCorrelation;
use crate::config::{ConfigRejection, NonZeroInterval, Slot};
use crate::{ActorType, GroundShape, ProductPayload, ProductValue, TIMER_PORT_NAME};
use circular_core::{NonZeroMillis, PortId};
use circular_expr::snippet::Snippet;
use circular_runtime::{
    ActorContext, ActorEffect, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, DeadLettering, EditableActor, Effect, EmittingActor, EmittingActorFactory,
    FoldedConfig, ReasonDecl, ScheduleSpec, Suppression,
};
use std::marker::PhantomData;
use std::sync::LazyLock;

pub const ALERT_STATE_SCHEMA: u16 = 1;
const STATE_TAGS: [&str; 3] = ["Ok", "Firing", "Cooldown"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlertState {
    Ok { armed: bool },
    Firing,
    Cooldown,
}

impl AlertState {
    const fn tag(self) -> &'static str {
        STATE_TAGS[match self {
            Self::Ok { .. } => 0,
            Self::Firing => 1,
            Self::Cooldown => 2,
        }]
    }
}

pub const PREDICATE_FAILED: &str = "predicate_failed";

static SUPPRESSION_REASONS: LazyLock<ReasonDecl<Suppression>> = LazyLock::new(|| {
    ReasonDecl::try_from_names([crate::STALE_TIMER_FIRE]).expect("one stale scheduled-fire reason")
});

static DEAD_LETTER_REASONS: LazyLock<ReasonDecl<DeadLettering>> = LazyLock::new(|| {
    ReasonDecl::try_from_names([PREDICATE_FAILED]).expect("one predicate-failure reason")
});

static TRANSITION_SHAPE: LazyLock<GroundShape> = LazyLock::new(|| {
    GroundShape::try_new(crate::registrations::alert_transition_shape())
        .expect("the registered transition is a ground shape")
});

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered alert port")
}

fn transition(from: AlertPhase, to: AlertPhase) -> ActorEffects<ProductPayload> {
    let value = ProductValue::object([
        ("from", ProductValue::string(from.observation().tag())),
        ("to", ProductValue::string(to.observation().tag())),
    ])
    .expect("the two registered fields differ");
    ActorEffects::emit(
        port("transition"),
        ProductPayload::new(TRANSITION_SHAPE.clone(), value),
    )
}

fn suppress(reason: &str) -> ActorEffects<ProductPayload> {
    ActorEffects::singleton(ActorEffect::suppress(
        SUPPRESSION_REASONS
            .resolve(reason)
            .expect("declared suppression reason"),
    ))
}

fn suppress_stale() -> ActorEffects<ProductPayload> {
    suppress(crate::STALE_TIMER_FIRE)
}

fn predicate_failed(subject: ProductPayload) -> ActorEffects<ProductPayload> {
    ActorEffects::singleton(ActorEffect::dead_letter(
        subject,
        DEAD_LETTER_REASONS
            .resolve(PREDICATE_FAILED)
            .expect("declared dead letter reason"),
    ))
}

#[derive(Clone, Copy)]
enum AlertPhase {
    Clear,
    Arming(ArmingCorrelation),
    Firing,
    Cooldown(ArmingCorrelation),
}

impl AlertPhase {
    const fn observation(self) -> AlertState {
        match self {
            Self::Clear => AlertState::Ok { armed: false },
            Self::Arming(_) => AlertState::Ok { armed: true },
            Self::Firing => AlertState::Firing,
            Self::Cooldown(_) => AlertState::Cooldown,
        }
    }
}

pub struct AlertActor<V, I> {
    phase: AlertPhase,
    last_correlation: u64,
    predicate: Snippet,
    firing: NonZeroMillis,
    recovery: NonZeroMillis,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> AlertActor<V, I> {
    #[must_use]
    pub const fn state(&self) -> AlertState {
        self.phase.observation()
    }

    fn reservation(
        &mut self,
        after: NonZeroMillis,
    ) -> (ArmingCorrelation, ActorEffects<ProductPayload>) {
        let next = self
            .last_correlation
            .checked_add(1)
            .expect("alert arming correlation exhausted its u64 space");
        let correlation = ArmingCorrelation::new(next);
        self.last_correlation = next;
        (
            correlation,
            ActorEffects::external(Effect::schedule(ScheduleSpec::new(
                after,
                correlation.correlation(),
            ))),
        )
    }

    fn timer(&mut self, value: &ProductValue) -> ActorEffects<ProductPayload> {
        let ProductValue::UInt(correlation) = value else {
            return suppress_stale();
        };
        let next = match self.phase {
            AlertPhase::Arming(current) if current.get() == *correlation => AlertPhase::Firing,
            AlertPhase::Cooldown(current) if current.get() == *correlation => AlertPhase::Clear,
            _ => return suppress_stale(),
        };
        let effects = transition(self.phase, next);
        self.phase = next;
        effects
    }
}

impl<V, I> EditableActor for AlertActor<V, I>
where
    V: Clone + From<u16> + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }

    fn checkpoint(&self) -> Option<ActorState<V>> {
        let (tag, armed, current) = match self.phase {
            AlertPhase::Clear => (0, 0, 0),
            AlertPhase::Arming(correlation) => (0, 1, correlation.get()),
            AlertPhase::Firing => (1, 0, 0),
            AlertPhase::Cooldown(correlation) => (2, 0, correlation.get()),
        };
        let mut bytes = Vec::with_capacity(18);
        bytes.extend_from_slice(&[tag, armed]);
        bytes.extend_from_slice(&current.to_be_bytes());
        bytes.extend_from_slice(&self.last_correlation.to_be_bytes());
        Some(ActorState::new(V::from(ALERT_STATE_SCHEMA), bytes))
    }

    fn restore(&mut self, state: ActorState<V>) -> Result<(), ActorRestoreError<V>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(ALERT_STATE_SCHEMA) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let Ok(bytes) = <[u8; 18]>::try_from(bytes.as_ref()) else {
            return Err(ActorRestoreError::DecodeFailed { schema });
        };
        let invariant = || ActorRestoreError::StateInvariantViolated {
            schema: schema.clone(),
        };
        let current = u64::from_be_bytes(bytes[2..10].try_into().expect("eight bytes"));
        let last = u64::from_be_bytes(bytes[10..18].try_into().expect("eight bytes"));
        if current > last {
            return Err(invariant());
        }
        self.phase = match (bytes[0], bytes[1], current) {
            (0, 0, 0) => AlertPhase::Clear,
            (0, 1, 1..) => AlertPhase::Arming(ArmingCorrelation::new(current)),
            (1, 0, 0) => AlertPhase::Firing,
            (2, 0, 1..) => AlertPhase::Cooldown(ArmingCorrelation::new(current)),
            _ => return Err(invariant()),
        };
        self.last_correlation = last;
        Ok(())
    }
}

impl<T> EmittingActor<T, ProductPayload> for AlertActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _ctx: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let payload = input.payload::<T>();
        if input.inlet().as_str() == TIMER_PORT_NAME {
            return self.timer(payload.value());
        }
        if input.inlet().as_str() != "event" {
            return ActorEffects::empty();
        }
        let pass = ActorEffects::emit(port("event"), payload.clone());
        let Ok(violated) = crate::filter_event(&self.predicate, payload) else {
            return pass.concat(predicate_failed(payload.clone()));
        };
        let before = self.phase;
        let effects = match (before, violated) {
            (AlertPhase::Clear, true) => {
                let (correlation, effects) = self.reservation(self.firing);
                self.phase = AlertPhase::Arming(correlation);
                effects
            }
            (AlertPhase::Clear | AlertPhase::Arming(_), false) => {
                self.phase = AlertPhase::Clear;
                ActorEffects::empty()
            }
            (AlertPhase::Firing, false) => {
                let (correlation, reservation) = self.reservation(self.recovery);
                self.phase = AlertPhase::Cooldown(correlation);
                transition(before, self.phase).concat(reservation)
            }
            (AlertPhase::Cooldown(_), true) => {
                self.phase = AlertPhase::Firing;
                transition(before, self.phase)
            }
            _ => ActorEffects::empty(),
        };
        pass.concat(effects)
    }
}

pub(crate) fn judge(
    config: &FoldedConfig,
    inlets: &crate::inlet_shapes::ResolvedInletShapes,
) -> Result<(), AlertFactoryError> {
    let AlertConfig { predicate, .. } = declared_config(config)?;
    match predicate.kind_split(&inlets.shape_env([&port("event")])) {
        Some(split) => Err(AlertFactoryError::PredicateKindSplit(split)),
        None => Ok(()),
    }
}

struct AlertConfig {
    predicate: Snippet,
    firing: NonZeroMillis,
    recovery: NonZeroMillis,
}

fn declared_config(config: &FoldedConfig) -> Result<AlertConfig, AlertFactoryError> {
    let value = config
        .for_type(ActorType::Alert)
        .map_err(|_| AlertFactoryError::InvalidConfig)?;
    let schema = crate::registration(ActorType::Alert).spec().config();
    let mut fields = schema.open(value)?;
    let predicate =
        crate::filter_config::accept_predicate(value).map_err(AlertFactoryError::Predicate)?;
    let firing = schema.read(&mut fields, &FIRING_DELAY)?;
    let recovery = schema.read(&mut fields, &RECOVERY_DELAY)?;
    Ok(AlertConfig {
        predicate,
        firing,
        recovery,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AlertFactoryError {
    InvalidConfig,
    Predicate(crate::filter_config::FilterConfigError),
    Config(ConfigRejection),
    PredicateKindSplit(circular_expr::shapes::KindSplit),
}

impl std::fmt::Display for AlertFactoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(rejection) => rejection.fmt(f),
            Self::InvalidConfig => f.write_str("alert config is not an alert config"),
            Self::Predicate(error) => write!(f, "alert predicate was rejected: {error}"),
            Self::PredicateKindSplit(split) => write!(
                f,
                "alert predicate cannot produce a value for its event inlet: {split}"
            ),
        }
    }
}

pub(crate) const FIRING_DELAY: Slot<NonZeroInterval> = Slot::new("firing_delay", NonZeroInterval);
pub(crate) const RECOVERY_DELAY: Slot<NonZeroInterval> =
    Slot::new("recovery_delay", NonZeroInterval);
impl std::error::Error for AlertFactoryError {}

impl From<crate::config::ConfigRejection> for AlertFactoryError {
    fn from(rejection: crate::config::ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub struct AlertFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for AlertFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: ActorType = ActorType::Alert;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = AlertActor<T::StateVersion, T::EffectId>;
    type Error = AlertFactoryError;

    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        let AlertConfig {
            predicate,
            firing,
            recovery,
        } = declared_config(config)?;
        Ok(AlertActor {
            phase: AlertPhase::Clear,
            last_correlation: 0,
            predicate,
            firing,
            recovery,
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

    use circular_testkit::types::TestRun;
    type TestTypes = circular_testkit::types::TestTypes<ProductPayload>;

    fn config(firing: i64, recovery: i64) -> FoldedConfig {
        FoldedConfig::minted(
            ActorType::Alert,
            ProductValue::object([
                ("predicate", ProductValue::string("event.n > 0")),
                ("firing_delay", ProductValue::Int(firing)),
                ("recovery_delay", ProductValue::Int(recovery)),
            ])
            .unwrap(),
        )
    }
    fn new_actor() -> AlertActor<u16, u64> {
        AlertFactory::<TestTypes>::create(&config(10, 20), &()).unwrap()
    }
    fn payload(value: ProductValue) -> ProductPayload {
        ProductPayload::new(GroundShape::try_new(crate::Shape::Any).unwrap(), value)
    }
    fn drive(f: impl FnOnce(&ActorId, &Incarnation<TestRun>, &Config)) {
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("alert"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .unwrap();
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        f(&named.as_actor_id(), &incarnation, &Config::default());
    }
    fn event(
        actor: &mut AlertActor<u16, u64>,
        ctx: &ActorContext<'_, TestRun, ()>,
        inlet: &str,
        value: ProductValue,
    ) -> ActorEffects<ProductPayload> {
        <AlertActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
            actor,
            &ActorInput::new(port(inlet), payload(value)),
            ctx,
        )
    }
    fn sample(
        actor: &mut AlertActor<u16, u64>,
        ctx: &ActorContext<'_, TestRun, ()>,
        n: i64,
    ) -> ActorEffects<ProductPayload> {
        event(
            actor,
            ctx,
            "event",
            ProductValue::object([("n", ProductValue::Int(n))]).unwrap(),
        )
    }
    fn fire(
        actor: &mut AlertActor<u16, u64>,
        ctx: &ActorContext<'_, TestRun, ()>,
        correlation: u64,
    ) -> ActorEffects<ProductPayload> {
        event(actor, ctx, TIMER_PORT_NAME, ProductValue::UInt(correlation))
    }
    fn assert_pass(effect: &ActorEffect<ProductPayload>, n: i64) {
        let ActorEffect::Emit {
            port,
            payload: actual,
            key,
            result: circular_runtime::EnvelopeResult::Ok,
        } = effect
        else {
            panic!("pass-through")
        };
        assert_eq!(port.as_str(), "event");
        assert_eq!(*key, None);
        assert_eq!(
            *actual,
            payload(ProductValue::object([("n", ProductValue::Int(n))]).unwrap())
        );
    }
    fn assert_transition(effect: &ActorEffect<ProductPayload>, from: &str, to: &str) {
        let ActorEffect::Emit {
            port,
            payload,
            key,
            result: circular_runtime::EnvelopeResult::Ok,
        } = effect
        else {
            panic!("transition")
        };
        assert_eq!(port.as_str(), "transition");
        assert_eq!(*key, None);
        assert_eq!(
            payload.value(),
            &ProductValue::object([
                ("from", ProductValue::string(from)),
                ("to", ProductValue::string(to)),
            ])
            .unwrap()
        );
        assert_eq!(
            payload.shape().as_shape(),
            &crate::Shape::Object {
                fields: crate::FieldMap::try_new(vec![
                    (
                        crate::Name::from_static("from"),
                        crate::Shape::Base(crate::BaseShape::String)
                    ),
                    (
                        crate::Name::from_static("to"),
                        crate::Shape::Base(crate::BaseShape::String)
                    ),
                ])
                .unwrap(),
                open: false,
            }
        );
    }
    fn assert_schedule(effect: &ActorEffect<ProductPayload>, delay: u64, correlation: u64) {
        let ActorEffect::External(Effect::Schedule { spec }) = effect else {
            panic!("Schedule")
        };
        assert_eq!(spec.after().get().get(), delay);
        assert_eq!(spec.correlation().get(), correlation);
    }
    fn assert_stale(effects: &ActorEffects<ProductPayload>) {
        assert!(
            matches!(effects.as_slice(), [ActorEffect::Suppress { reason }] if reason.name() == "stale_timer_fire")
        );
    }

    #[test]
    fn recovery_cancels_candidate_and_rearming_never_reuses_old_correlation() {
        drive(|actor, incarnation, config| {
            let ctx = ActorContext::new(actor, incarnation, config, &());
            let mut actor = new_actor();
            sample(&mut actor, &ctx, 1);
            let cancelled = sample(&mut actor, &ctx, 0);
            assert_eq!(cancelled.len(), 1);
            assert_pass(&cancelled.as_slice()[0], 0);
            assert_eq!(actor.state(), AlertState::Ok { armed: false });
            assert_stale(&fire(&mut actor, &ctx, 1));
            let armed = sample(&mut actor, &ctx, 1);
            assert_schedule(&armed.as_slice()[1], 10, 2);
            assert_stale(&fire(&mut actor, &ctx, 1));
            let fired = fire(&mut actor, &ctx, 2);
            assert_eq!(fired.len(), 1);
            assert_transition(&fired.as_slice()[0], "Ok", "Firing");
        });
    }

    #[test]
    fn renewed_violation_cancels_recovery_and_emits_cooldown_to_firing() {
        drive(|actor, incarnation, config| {
            let ctx = ActorContext::new(actor, incarnation, config, &());
            let mut actor = new_actor();
            sample(&mut actor, &ctx, 1);
            fire(&mut actor, &ctx, 1);
            sample(&mut actor, &ctx, 0);
            let resumed = sample(&mut actor, &ctx, 1);
            assert_eq!(resumed.len(), 2);
            assert_pass(&resumed.as_slice()[0], 1);
            assert_transition(&resumed.as_slice()[1], "Cooldown", "Firing");
            assert_stale(&fire(&mut actor, &ctx, 2));
            assert_eq!(actor.state(), AlertState::Firing);
            let recovery = sample(&mut actor, &ctx, 0);
            assert_schedule(&recovery.as_slice()[2], 20, 3);
            assert_stale(&fire(&mut actor, &ctx, 2));
            assert_eq!(actor.state(), AlertState::Cooldown);
        });
    }

    #[test]
    fn missing_mismatched_and_malformed_correlations_cannot_advance_state() {
        drive(|actor, incarnation, config| {
            let ctx = ActorContext::new(actor, incarnation, config, &());
            let mut actor = new_actor();
            assert_stale(&fire(&mut actor, &ctx, 0));
            sample(&mut actor, &ctx, 1);
            let before = actor.checkpoint().unwrap();
            assert_stale(&fire(&mut actor, &ctx, 99));
            assert_stale(&event(
                &mut actor,
                &ctx,
                TIMER_PORT_NAME,
                ProductValue::Int(1),
            ));
            assert_eq!(actor.checkpoint().unwrap(), before);
        });
    }

    #[test]
    fn checkpoint_round_trips_all_states_and_preserves_next_correlation() {
        drive(|actor, incarnation, config| {
            let ctx = ActorContext::new(actor, incarnation, config, &());
            let mut original = new_actor();
            for phase in 0..5 {
                let checkpoint = original.checkpoint().unwrap();
                let mut restored = new_actor();
                restored.restore(checkpoint.clone()).unwrap();
                assert_eq!(restored.checkpoint().unwrap(), checkpoint);
                assert_eq!(restored.state(), original.state());
                let a = match phase {
                    0 | 4 => sample(&mut original, &ctx, 1),
                    1 => fire(&mut original, &ctx, 1),
                    2 => sample(&mut original, &ctx, 0),
                    3 => fire(&mut original, &ctx, 2),
                    _ => unreachable!(),
                };
                let b = match phase {
                    0 | 4 => sample(&mut restored, &ctx, 1),
                    1 => fire(&mut restored, &ctx, 1),
                    2 => sample(&mut restored, &ctx, 0),
                    3 => fire(&mut restored, &ctx, 2),
                    _ => unreachable!(),
                };
                assert_eq!(a, b);
                if phase == 4 {
                    assert_schedule(&b.as_slice()[1], 10, 3);
                }
            }
        });
    }

    #[test]
    fn restore_rejects_invalid_schema_encoding_and_state_without_partial_mutation() {
        let mut actor = new_actor();
        let before = actor.checkpoint().unwrap();
        assert!(matches!(
            actor.restore(ActorState::new(2, vec![0; 18])),
            Err(ActorRestoreError::SchemaBeyondLadder { .. })
        ));
        assert!(matches!(
            actor.restore(ActorState::new(1, vec![0; 17])),
            Err(ActorRestoreError::DecodeFailed { .. })
        ));
        for (tag, armed, current, last) in [
            (3, 0, 0u64, 0u64),
            (0, 2, 0, 0),
            (0, 1, 0, 0),
            (1, 0, 1, 1),
            (2, 0, 2, 1),
            (2, 1, 1, 1),
        ] {
            let mut bytes = vec![tag, armed];
            bytes.extend_from_slice(&current.to_be_bytes());
            bytes.extend_from_slice(&last.to_be_bytes());
            assert!(matches!(
                actor.restore(ActorState::new(1, bytes)),
                Err(ActorRestoreError::StateInvariantViolated { .. })
            ));
            assert_eq!(actor.checkpoint().unwrap(), before);
        }
    }

    #[test]
    fn every_alert_phase_preserves_its_schema_one_literal_bytes() {
        drive(|actor, incarnation, config| {
            let ctx = ActorContext::new(actor, incarnation, config, &());
            let mut actor = new_actor();
            for (step, bytes) in [
                (0, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
                (1, [0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1]),
                (2, [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]),
                (3, [2, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 2]),
                (4, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]),
            ] {
                match step {
                    0 => {}
                    1 => {
                        sample(&mut actor, &ctx, 1);
                    }
                    2 => {
                        fire(&mut actor, &ctx, 1);
                    }
                    3 => {
                        sample(&mut actor, &ctx, 0);
                    }
                    4 => {
                        fire(&mut actor, &ctx, 2);
                    }
                    _ => unreachable!(),
                }
                assert_eq!(actor.checkpoint().unwrap(), ActorState::new(1, bytes));
                let mut restored = new_actor();
                restored.restore(ActorState::new(1, bytes)).unwrap();
                assert_eq!(restored.state(), actor.state());
            }
        });
    }

    #[test]
    fn factory_rejects_missing_predicate_delay_unknown_field_and_negative_delay() {
        for value in [
            ProductValue::Null,
            ProductValue::object([
                ("firing_delay", ProductValue::Int(10)),
                ("recovery_delay", ProductValue::Int(20)),
            ])
            .unwrap(),
            ProductValue::object([
                ("predicate", ProductValue::string("true")),
                ("firing_delay", ProductValue::Int(10)),
            ])
            .unwrap(),
            ProductValue::object([
                ("predicate", ProductValue::string("(")),
                ("firing_delay", ProductValue::Int(10)),
                ("recovery_delay", ProductValue::Int(20)),
            ])
            .unwrap(),
            ProductValue::object([
                ("predicate", ProductValue::string("true")),
                ("firing_delay", ProductValue::Int(10)),
                ("recovery_delay", ProductValue::Int(20)),
                ("extra", ProductValue::Null),
            ])
            .unwrap(),
            config(-1, 20).value().clone(),
        ] {
            assert!(
                AlertFactory::<TestTypes>::create(
                    &FoldedConfig::minted(ActorType::Alert, value),
                    &()
                )
                .is_err()
            );
        }
    }

    fn assert_predicate_failed(effects: &ActorEffects<ProductPayload>, value: &ProductValue) {
        let [
            ActorEffect::Emit {
                port,
                payload: actual,
                ..
            },
            ActorEffect::DeadLetter { subject, reason },
        ] = effects.as_slice()
        else {
            panic!("pass-through then predicate_failed dead letter: {effects:?}")
        };
        assert_eq!(port.as_str(), "event");
        assert_eq!(*actual, payload(value.clone()));
        assert_eq!(
            *subject,
            payload(value.clone()),
            "the subject is the input that was not adjudicated"
        );
        assert_eq!(reason.name(), "predicate_failed");
    }

    #[test]
    fn a_string_number_sample_is_judged_through_the_standard_conversion() {
        drive(|actor, incarnation, config| {
            let ctx = ActorContext::new(actor, incarnation, config, &());
            let mut actor = AlertFactory::<TestTypes>::create(
                &FoldedConfig::minted(
                    ActorType::Alert,
                    ProductValue::object([
                        (
                            "predicate",
                            ProductValue::string(
                                "event.data.result.exists(r, double(r.value[1]) > 0.05)",
                            ),
                        ),
                        ("firing_delay", ProductValue::Int(10)),
                        ("recovery_delay", ProductValue::Int(20)),
                    ])
                    .unwrap(),
                ),
                &(),
            )
            .unwrap();
            let prometheus = |value: &str| {
                ProductValue::object([(
                    "data",
                    ProductValue::object([(
                        "result",
                        ProductValue::Array(vec![
                            ProductValue::object([(
                                "value",
                                ProductValue::Array(vec![
                                    ProductValue::float(1_727_000_000.5),
                                    ProductValue::string(value),
                                ]),
                            )])
                            .unwrap(),
                        ]),
                    )])
                    .unwrap(),
                )])
                .unwrap()
            };

            let calm = event(&mut actor, &ctx, "event", prometheus("0.01"));
            assert_eq!(calm.len(), 1, "{calm:?}");
            assert_eq!(actor.state(), AlertState::Ok { armed: false });

            let violated = event(&mut actor, &ctx, "event", prometheus("0.16"));
            assert_eq!(violated.len(), 2, "{violated:?}");
            assert_schedule(&violated.as_slice()[1], 10, 1);
            assert_eq!(actor.state(), AlertState::Ok { armed: true });

            let broken = prometheus("not-a-number");
            let failed = event(&mut actor, &ctx, "event", broken.clone());
            assert_predicate_failed(&failed, &broken);
            assert_eq!(actor.state(), AlertState::Ok { armed: true });
        });
    }

    #[test]
    fn factory_recreation_starts_ok_unarmed_before_c2() {
        drive(|actor, incarnation, plan_config| {
            let ctx = ActorContext::new(actor, incarnation, plan_config, &());
            let mut original = new_actor();
            sample(&mut original, &ctx, 1);
            fire(&mut original, &ctx, 1);
            let before = original.checkpoint().unwrap();
            assert_eq!(
                original.on_config_change(&config(30, 40)),
                ConfigChangeOutcome::ReplaceIncarnation
            );
            assert_eq!(original.checkpoint().unwrap(), before);
            let mut recreated = new_actor();
            assert_eq!(recreated.state(), AlertState::Ok { armed: false });
            assert_stale(&fire(&mut recreated, &ctx, 1));
            let effects = sample(&mut recreated, &ctx, 1);
            assert_schedule(&effects.as_slice()[1], 10, 1);
        });
    }
}
