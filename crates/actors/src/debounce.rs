
use crate::actor_support::StampedEvent;
use crate::arming::ArmingCorrelation;
use crate::config::{ConfigRejection, Interval, Slot};
use crate::payload_value::{decode_payload, encode_payload};
use crate::{ActorType, ProductPayload, ProductValue, TIMER_PORT_NAME};
use circular_core::{Boundary, Ceilings, Millis, NonZeroMillis, PortId, RecordedInstant};
use circular_runtime::{
    ActorContext, ActorEffect, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, Effect, EmittingActor, EmittingActorFactory, FoldedConfig,
    ReasonDecl, ScheduleSpec, Suppression,
};
use std::fmt;
use std::marker::PhantomData;
use std::sync::LazyLock;

pub const DEBOUNCE_STATE_SCHEMA: u16 = 1;

static SUPPRESSION_REASONS: LazyLock<ReasonDecl<Suppression>> = LazyLock::new(|| {
    ReasonDecl::try_from_names([crate::STALE_TIMER_FIRE]).expect("one stale scheduled-fire reason")
});

fn suppress_stale() -> ActorEffects<ProductPayload> {
    ActorEffects::singleton(ActorEffect::suppress(
        SUPPRESSION_REASONS
            .resolve(crate::STALE_TIMER_FIRE)
            .expect("declared suppression reason"),
    ))
}

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered debounce port")
}

#[derive(Clone, Debug, PartialEq)]
pub enum DebounceState {
    Empty,
    Pending {
        payload: ProductPayload,
        admitted_at: Option<RecordedInstant>,
        due: Option<RecordedInstant>,
    },
}

/// A pending payload always owns its one valid reservation. Checkpoint and
/// observation shapes are projections; they are not a second mutable state.
struct PendingDebounce {
    payload: ProductPayload,
    admitted_at: Option<RecordedInstant>,
    due: Option<RecordedInstant>,
    arming: ArmingCorrelation,
}

pub struct DebounceActor<V, I> {
    pending: Option<PendingDebounce>,
    last_correlation: u64,
    quiet_window: Millis,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> DebounceActor<V, I> {
    #[must_use]
    pub fn state(&self) -> DebounceState {
        match &self.pending {
            None => DebounceState::Empty,
            Some(pending) => DebounceState::Pending {
                payload: pending.payload.clone(),
                admitted_at: pending.admitted_at,
                due: pending.due,
            },
        }
    }

    #[must_use]
    pub const fn quiet_window(&self) -> Millis {
        self.quiet_window
    }

    fn arm(
        &mut self,
        after: NonZeroMillis,
        payload: ProductPayload,
        admitted_at: Option<RecordedInstant>,
        due: Option<RecordedInstant>,
    ) -> ActorEffects<ProductPayload> {
        let next = self
            .last_correlation
            .checked_add(1)
            .expect("debounce arming correlation exhausted its u64 space");
        let correlation = ArmingCorrelation::new(next);
        self.last_correlation = next;
        self.pending = Some(PendingDebounce {
            payload,
            admitted_at,
            due,
            arming: correlation,
        });
        ActorEffects::external(Effect::schedule(ScheduleSpec::new(
            after,
            correlation.correlation(),
        )))
    }

    fn expire(&mut self, value: &ProductValue) -> ActorEffects<ProductPayload> {
        let ProductValue::UInt(correlation) = value else {
            return suppress_stale();
        };
        let Some(pending) = &self.pending else {
            return suppress_stale();
        };
        if pending.arming.get() != *correlation {
            return suppress_stale();
        }
        let effects = ActorEffects::emit(port("event"), pending.payload.clone());
        self.pending = None;
        effects
    }

    fn quiet_window_of(config: &FoldedConfig) -> Result<Millis, DebounceFactoryError> {
        let value = config
            .for_type(ActorType::Debounce)
            .map_err(|_| DebounceFactoryError::InvalidConfig)?;
        let schema = crate::registration(ActorType::Debounce).spec().config();
        let mut fields = schema.open(value)?;
        Ok(schema.read(&mut fields, &QUIET_WINDOW)?)
    }
}

impl<V, I> EditableActor for DebounceActor<V, I>
where
    V: Clone + From<u16> + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, config: &FoldedConfig) -> ConfigChangeOutcome {
        if self.pending.is_some() {
            return ConfigChangeOutcome::ReplaceIncarnation;
        }
        match Self::quiet_window_of(config) {
            Ok(window) => {
                self.quiet_window = window;
                ConfigChangeOutcome::Absorbed
            }
            Err(_) => ConfigChangeOutcome::ReplaceIncarnation,
        }
    }

    fn checkpoint(&self) -> Option<ActorState<V>> {
        self.try_checkpoint()
            .expect("canonical encoding of the debounce state")
    }

    fn try_checkpoint(&self) -> Result<Option<ActorState<V>>, circular_core::CodecError> {
        let instant = |at: Option<RecordedInstant>| {
            at.map_or(ProductValue::Null, |at| ProductValue::UInt(at.millis()))
        };
        let state = match &self.pending {
            None => ProductValue::Null,
            Some(PendingDebounce {
                payload,
                admitted_at,
                due,
                ..
            }) => ProductValue::array([
                encode_payload(payload),
                instant(*admitted_at),
                instant(*due),
            ]),
        };
        let value = ProductValue::array([
            state,
            self.pending.as_ref().map_or(ProductValue::Null, |pending| {
                ProductValue::UInt(pending.arming.get())
            }),
            ProductValue::UInt(self.last_correlation),
        ]);
        let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::ActorState))?;
        Ok(Some(ActorState::new(V::from(DEBOUNCE_STATE_SCHEMA), bytes)))
    }

    fn restore(&mut self, state: ActorState<V>) -> Result<(), ActorRestoreError<V>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(DEBOUNCE_STATE_SCHEMA) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let decode = || ActorRestoreError::DecodeFailed {
            schema: schema.clone(),
        };
        let invariant = || ActorRestoreError::StateInvariantViolated {
            schema: schema.clone(),
        };
        let value = circular_core::decode(&bytes, Ceilings::for_boundary(Boundary::ActorState))
            .map_err(|_| decode())?;
        let ProductValue::Array(fields) = value else {
            return Err(decode());
        };
        let Ok([state, current, last]) = <[ProductValue; 3]>::try_from(fields) else {
            return Err(decode());
        };
        let ProductValue::UInt(last) = last else {
            return Err(decode());
        };
        let arming = match current {
            ProductValue::Null => None,
            ProductValue::UInt(current) if current != 0 && current <= last => {
                Some(ArmingCorrelation::new(current))
            }
            _ => return Err(invariant()),
        };
        let instant = |value: ProductValue| match value {
            ProductValue::Null => Ok(None),
            ProductValue::UInt(at) => Ok(Some(RecordedInstant::from_millis(at))),
            _ => Err(()),
        };
        let pending = match (state, arming) {
            (ProductValue::Null, None) => None,
            (ProductValue::Array(fields), Some(arming)) => {
                let Ok([payload, admitted_at, due]) = <[ProductValue; 3]>::try_from(fields) else {
                    return Err(decode());
                };
                let payload = decode_payload(payload).ok_or_else(decode)?;
                let admitted_at = instant(admitted_at).map_err(|()| decode())?;
                let due = instant(due).map_err(|()| decode())?;
                if let (Some(admitted_at), Some(due)) = (admitted_at, due)
                    && due < admitted_at
                {
                    return Err(invariant());
                }
                Some(PendingDebounce {
                    payload,
                    admitted_at,
                    due,
                    arming,
                })
            }
            _ => return Err(invariant()),
        };
        self.pending = pending;
        self.last_correlation = last;
        Ok(())
    }
}

impl<T> EmittingActor<T, ProductPayload> for DebounceActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Event: crate::StampedEvent,
    T::StateVersion: From<u16> + PartialEq,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let payload = input.payload::<T>();
        if input.inlet().as_str() == TIMER_PORT_NAME {
            return self.expire(payload.value());
        }
        if input.inlet().as_str() != "event" {
            return ActorEffects::empty();
        }
        let admitted_at = input.event().recorded_instant();
        let Ok(after) = NonZeroMillis::new(self.quiet_window.get()) else {
            self.pending = None;
            return ActorEffects::emit(port("event"), payload.clone());
        };
        let due = admitted_at.map(|at| at.saturating_add(self.quiet_window));
        self.arm(after, payload.clone(), admitted_at, due)
    }
}

pub(crate) const QUIET_WINDOW: Slot<Interval> = Slot::new("quiet_window", Interval);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DebounceFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
}

impl fmt::Display for DebounceFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => {
                formatter.write_str("debounce config was folded for another actor")
            }
            Self::Config(rejection) => rejection.fmt(formatter),
        }
    }
}

impl std::error::Error for DebounceFactoryError {}

impl From<crate::config::ConfigRejection> for DebounceFactoryError {
    fn from(rejection: crate::config::ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub struct DebounceFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for DebounceFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Event: crate::StampedEvent,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: ActorType = ActorType::Debounce;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::AbsorbsSome;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = DebounceActor<T::StateVersion, T::EffectId>;
    type Error = DebounceFactoryError;

    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        Ok(DebounceActor {
            pending: None,
            last_correlation: 0,
            quiet_window: DebounceActor::<T::StateVersion, T::EffectId>::quiet_window_of(config)?,
            marker: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{
        Causality, Emission, Event, ProducerIdentity, Sequence, Stamp, StreamIdentity, Tick, admit,
        emit,
    };
    use circular_plan::{Config, Generation, GenerationVector, Incarnation, NamedActorId, ScopeId};
    use circular_runtime::{ActorEffect, EditDisposition, ScheduleCorrelation};
    use std::convert::Infallible;

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct Run;
    impl StreamIdentity for Run {}
    #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    struct Producer;
    impl ProducerIdentity for Producer {
        type EventProducer = Self;
        fn from_event_producer(producer: Self) -> Self {
            producer
        }
    }

    struct Types;
    impl ActorTypes for Types {
        type Stream = Run;
        type Event = Event<Run, Producer, ProductPayload, Infallible>;
        type Payload = ProductPayload;
        type EffectId = u64;
        type StateVersion = u16;
        type Observation = ();
        type Grants = ();
        fn payload(event: &Self::Event) -> &Self::Payload {
            event.payload()
        }
    }

    type Actor = DebounceActor<u16, u64>;

    fn folded(window: ProductValue) -> FoldedConfig {
        FoldedConfig::minted(
            ActorType::Debounce,
            ProductValue::object([("quiet_window", window)]).expect("one field"),
        )
    }

    fn new_actor(window: u64) -> Actor {
        DebounceFactory::<Types>::create(&folded(ProductValue::UInt(window)), &())
            .expect("registered interval in milliseconds")
    }

    fn payload(value: ProductValue) -> ProductPayload {
        ProductPayload::new(
            crate::GroundShape::try_new(crate::Shape::Any).expect("Any is a ground shape"),
            value,
        )
    }

    fn arrival(
        inlet: &str,
        value: ProductValue,
        at: Option<u64>,
    ) -> ActorInput<<Types as ActorTypes>::Event> {
        let stamp = Stamp::from_event_producer(
            Tick::new(1),
            Producer,
            Sequence::new(1).expect("first ordinal"),
            circular_core::RevisionEpochId::new(1).expect("first revision"),
        );
        let event = admit(
            Run,
            Emission::from_runtime(emit(payload(value)), Causality::Source, None),
            stamp,
        )
        .expect("accepted")
        .with_recorded_instant(at.map(RecordedInstant::from_millis));
        ActorInput::new(port(inlet), event)
    }

    fn drive(
        actor: &mut Actor,
        input: &ActorInput<<Types as ActorTypes>::Event>,
    ) -> ActorEffects<ProductPayload> {
        let named = NamedActorId::new(
            ScopeId::root(),
            circular_plan::Name::from_normalized("debounce"),
        );
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation = Incarnation::new(Run, named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let config = Config::default();
        let context = ActorContext::new(&actor_id, &incarnation, &config, &());
        <Actor as EmittingActor<Types, ProductPayload>>::on_event(actor, input, &context)
    }

    fn event(
        actor: &mut Actor,
        value: ProductValue,
        at: Option<u64>,
    ) -> ActorEffects<ProductPayload> {
        drive(actor, &arrival("event", value, at))
    }

    fn fire(actor: &mut Actor, correlation: u64) -> ActorEffects<ProductPayload> {
        drive(
            actor,
            &arrival(TIMER_PORT_NAME, ProductValue::UInt(correlation), Some(0)),
        )
    }

    fn scheduled(effects: &ActorEffects<ProductPayload>) -> (u64, NonZeroMillis) {
        effects
            .iter()
            .find_map(|effect| match effect {
                ActorEffect::External(Effect::Schedule { spec }) => {
                    Some((correlation_of(spec.correlation()), spec.after()))
                }
                _ => None,
            })
            .expect("one arrival schedules one fire")
    }

    fn correlation_of(correlation: ScheduleCorrelation) -> u64 {
        correlation.get()
    }

    fn emitted(effects: &ActorEffects<ProductPayload>) -> ProductValue {
        let [
            ActorEffect::Emit {
                port: outlet,
                payload,
                key,
                ..
            },
        ] = effects.as_slice()
        else {
            panic!("an expiry is one emission: {effects:?}")
        };
        assert_eq!(outlet.as_str(), "event", "the fixed outlet is event");
        assert!(key.is_none(), "debounce carries no coordinate envelope");
        payload.value().clone()
    }

    fn assert_suppressed(effects: &ActorEffects<ProductPayload>) {
        assert!(
            matches!(effects.as_slice(), [ActorEffect::Suppress { reason }]
                if reason.name() == crate::STALE_TIMER_FIRE),
            "expected one declared suppression: {effects:?}"
        );
    }

    #[test]
    fn both_states_round_trip_through_the_checkpoint() {
        let mut actor = new_actor(100);
        let empty = actor
            .checkpoint()
            .expect("Empty is also a variant of the schema");
        assert_eq!(empty.schema(), &DEBOUNCE_STATE_SCHEMA);
        event(&mut actor, ProductValue::String("last".into()), Some(7));
        let pending = actor.checkpoint().expect("checkpoint of the pending state");

        for state in [empty, pending] {
            let mut restored = new_actor(100);
            restored
                .restore(state.clone())
                .expect("restore under the same schema");
            assert_eq!(restored.checkpoint().expect("re-serialize"), state);
        }
    }

    #[test]
    fn the_expiry_emits_the_last_admitted_payload() {
        let mut actor = new_actor(100);
        let mut correlation = 0;
        for value in 1..=5_i64 {
            correlation = scheduled(&event(
                &mut actor,
                ProductValue::Int(value),
                Some(value.unsigned_abs()),
            ))
            .0;
        }
        assert_eq!(
            emitted(&fire(&mut actor, correlation)),
            ProductValue::Int(5)
        );
    }

    #[test]
    fn an_expiry_without_a_live_schedule_is_declared_suppression() {
        let mut actor = new_actor(100);
        assert_suppressed(&fire(&mut actor, 1));
        let (correlation, _) = scheduled(&event(&mut actor, ProductValue::Int(1), Some(0)));
        fire(&mut actor, correlation);
        assert_suppressed(&fire(&mut actor, correlation));
        assert_suppressed(&drive(
            &mut actor,
            &arrival(TIMER_PORT_NAME, ProductValue::Null, Some(0)),
        ));
    }

    #[test]
    fn an_unknown_inlet_emits_nothing_because_there_is_no_error_port() {
        let mut actor = new_actor(100);
        assert!(
            event(&mut actor, ProductValue::Int(1), Some(0))
                .iter()
                .count()
                > 0
        );
        let before = actor.checkpoint();
        assert!(
            drive(
                &mut actor,
                &arrival("nonesuch", ProductValue::Int(9), Some(1))
            )
            .is_empty()
        );
        assert_eq!(actor.checkpoint(), before);
    }

    #[test]
    fn a_zero_quiet_window_is_the_immediate_boundary() {
        let mut actor = new_actor(0);
        let effects = event(&mut actor, ProductValue::Int(7), Some(10));
        assert_eq!(emitted(&effects), ProductValue::Int(7));
        assert_eq!(
            actor.state(),
            DebounceState::Empty,
            "no pending value is created"
        );
        assert!(
            effects
                .iter()
                .all(|effect| !matches!(effect, ActorEffect::External(_))),
            "the expiry falls on the acceptance itself, so nothing is scheduled"
        );
    }

    #[test]
    fn an_unstamped_arrival_leaves_the_window_endpoints_empty() {
        let mut actor = new_actor(100);
        let (correlation, _) = scheduled(&event(&mut actor, ProductValue::Int(1), None));
        assert_eq!(
            actor.state(),
            DebounceState::Pending {
                payload: payload(ProductValue::Int(1)),
                admitted_at: None,
                due: None,
            }
        );
        assert_eq!(
            emitted(&fire(&mut actor, correlation)),
            ProductValue::Int(1)
        );
    }

    #[test]
    fn the_registration_declares_no_external_effect() {
        let spec = crate::registration(ActorType::Debounce).spec();
        assert_eq!(spec.effect(), &crate::capabilities::EffectDeclaration::None);
        assert!(spec.requires().is_empty());
        assert!(!spec.is_source(), "debounce is not a source");
        assert_eq!(
            <DebounceFactory<Types> as EmittingActorFactory<ProductPayload>>::DISPOSITION,
            EditDisposition::AbsorbsSome
        );
        assert!(<DebounceFactory<Types> as EmittingActorFactory<ProductPayload>>::CHECKPOINTS);
    }

    #[test]
    fn config_rejection_is_the_only_failure_and_it_has_no_instance() {
        let create = |config: ProductValue| {
            DebounceFactory::<Types>::create(
                &FoldedConfig::minted(ActorType::Debounce, config),
                &(),
            )
            .err()
            .expect("refusal")
        };
        assert_eq!(
            create(
                ProductValue::object(Vec::<(String, ProductValue)>::new()).expect("empty object")
            ),
            DebounceFactoryError::Config(crate::config::ConfigRejection::Missing("quiet_window"))
        );
        assert_eq!(
            create(ProductValue::Null),
            DebounceFactoryError::Config(crate::config::ConfigRejection::NotObject(
                circular_core::NotObject {
                    at: circular_core::FieldPath::root(),
                    actual: circular_core::ValueKind::Null,
                }
            ))
        );
        assert_eq!(
            create(
                ProductValue::object([("quiet_window", ProductValue::String("soon".into()))])
                    .expect("one field")
            ),
            DebounceFactoryError::Config(crate::config::ConfigRejection::Slot {
                slot: "quiet_window",
                error: crate::config::ConfigDecodeError::IntervalKind {
                    actual: circular_core::ValueKind::String,
                },
            })
        );
        assert_eq!(
            create(
                ProductValue::object([
                    ("quiet_window", ProductValue::UInt(1)),
                    ("nonesuch", ProductValue::UInt(1)),
                ])
                .expect("two fields")
            ),
            DebounceFactoryError::Config(crate::config::ConfigRejection::Unknown(
                circular_core::UnknownField {
                    at: circular_core::FieldPath::root(),
                    key: "nonesuch".to_owned(),
                }
            ))
        );
        for error in [
            DebounceFactoryError::InvalidConfig,
            DebounceFactoryError::Config(crate::config::ConfigRejection::Missing("quiet_window")),
        ] {
            assert!(
                !error.to_string().is_empty(),
                "every refusal carries a diagnostic"
            );
        }
    }

    #[test]
    fn pending_checkpoint_keeps_the_existing_shape_and_failed_restore_is_atomic() {
        let mut actor = new_actor(100);
        event(&mut actor, ProductValue::Int(7), Some(20));
        let pending = ProductValue::array([
            ProductValue::array([
                ProductValue::array([
                    ProductValue::array([ProductValue::Int(1)]),
                    ProductValue::Int(7),
                ]),
                ProductValue::UInt(20),
                ProductValue::UInt(120),
            ]),
            ProductValue::UInt(1),
            ProductValue::UInt(1),
        ]);
        let checkpoint = actor.checkpoint().unwrap();
        assert_eq!(
            checkpoint,
            ActorState::new(
                1,
                circular_core::encode(&pending, Ceilings::for_boundary(Boundary::ActorState))
                    .unwrap()
            ),
        );
        for invalid in [
            ProductValue::array([
                ProductValue::Null,
                ProductValue::UInt(1),
                ProductValue::UInt(1),
            ]),
            ProductValue::array([
                pending.as_array().unwrap()[0].clone(),
                ProductValue::Null,
                ProductValue::UInt(1),
            ]),
        ] {
            assert!(matches!(
                actor.restore(ActorState::new(
                    1,
                    circular_core::encode(&invalid, Ceilings::for_boundary(Boundary::ActorState))
                        .unwrap()
                )),
                Err(ActorRestoreError::StateInvariantViolated { schema: 1 }),
            ));
            assert_eq!(actor.checkpoint().unwrap(), checkpoint);
        }
        assert_eq!(emitted(&fire(&mut actor, 1)), ProductValue::Int(7));
    }
}
