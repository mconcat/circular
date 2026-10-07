
use crate::actor_support::{StampedEvent, error_payload};
use crate::config::{ConfigRejection, NonZeroInterval, PositiveCount, Slot, Tags};
use crate::{
    ActorType, BaseShape, FieldMap, GroundShape, Name, ProductPayload, ProductValue, Shape,
};
use circular_core::{Boundary, Ceilings, FloatValue, PortId, RecordedInstant};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, EmittingActor, EmittingActorFactory, FoldedConfig,
};
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;

pub const EMA_STATE_SCHEMA: u16 = 4;
pub(crate) const EMA_DISPOSITION: circular_runtime::EditDisposition =
    circular_runtime::EditDisposition::Restarts;
pub(crate) const EMA_CHECKPOINTS: bool = true;

#[derive(Clone, Debug, PartialEq)]
pub enum EmaState {
    Empty,
    Ready {
        last_value: FloatValue,
        n: u64,
        last_at: Option<RecordedInstant>,
    },
}

const TAG_EMPTY: u8 = 0;
const TAG_READY: u8 = 1;
const READY_PREFIX_LEN: usize = 1 + 8 + 8;

impl EmaState {
    #[must_use]
    pub fn encode(&self) -> Box<[u8]> {
        match self {
            Self::Empty => Box::new([TAG_EMPTY]),
            Self::Ready {
                last_value,
                n,
                last_at,
            } => {
                let mut bytes = vec![0_u8; READY_PREFIX_LEN];
                bytes[0] = TAG_READY;
                bytes[1..9].copy_from_slice(&last_value.to_bits().to_be_bytes());
                bytes[9..17].copy_from_slice(&n.to_be_bytes());
                let at = last_at.map_or(ProductValue::Null, |at| ProductValue::UInt(at.millis()));
                bytes.extend(
                    circular_core::encode(&at, Ceilings::for_boundary(Boundary::ActorState))
                        .expect("EMA timestamp value"),
                );
                bytes.into_boxed_slice()
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, EmaStateError> {
        match bytes {
            [TAG_EMPTY] => Ok(Self::Empty),
            [TAG_READY, rest @ ..] if rest.len() > 16 => {
                let bits = u64::from_be_bytes(rest[..8].try_into().expect("first eight bytes"));
                let value = f64::from_bits(bits);
                if !value.is_finite() {
                    return Err(EmaStateError::Invariant);
                }
                let n = u64::from_be_bytes(rest[8..16].try_into().expect("last eight bytes"));
                if n == 0 {
                    return Err(EmaStateError::Invariant);
                }
                let last_at = match circular_core::decode(
                    &rest[16..],
                    Ceilings::for_boundary(Boundary::ActorState),
                )
                .map_err(|_| EmaStateError::Decode)?
                {
                    ProductValue::Null => None,
                    ProductValue::UInt(at) => Some(RecordedInstant::from_millis(at)),
                    _ => return Err(EmaStateError::Decode),
                };
                Ok(Self::Ready {
                    last_value: FloatValue::new(value),
                    n,
                    last_at,
                })
            }
            _ => Err(EmaStateError::Decode),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmaStateError {
    Decode,
    Invariant,
}

impl EmaStateError {
    fn into_restore<V>(self, schema: V) -> ActorRestoreError<V> {
        match self {
            Self::Decode => ActorRestoreError::DecodeFailed { schema },
            Self::Invariant => ActorRestoreError::StateInvariantViolated { schema },
        }
    }
}

impl fmt::Display for EmaStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode => formatter.write_str("ema state bytes do not match this schema"),
            Self::Invariant => {
                formatter.write_str("ema state values are outside the declared domain")
            }
        }
    }
}

impl std::error::Error for EmaStateError {}

pub struct EmaActor<V, I> {
    state: EmaState,
    half_life: NonZeroU64,
    wallclock: bool,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> EmaActor<V, I> {
    #[must_use]
    pub const fn new(half_life: NonZeroU64) -> Self {
        Self {
            state: EmaState::Empty,
            half_life,
            wallclock: false,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn state(&self) -> &EmaState {
        &self.state
    }

    pub fn set_state(&mut self, state: EmaState) {
        self.state = state;
    }
}

impl<V, I> EditableActor for EmaActor<V, I>
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
        Some(ActorState::new(
            V::from(EMA_STATE_SCHEMA),
            self.state.encode(),
        ))
    }

    fn restore(
        &mut self,
        state: ActorState<Self::StateVersion>,
    ) -> Result<(), ActorRestoreError<Self::StateVersion>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(EMA_STATE_SCHEMA) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let mut restored = EmaState::decode(&bytes).map_err(|error| error.into_restore(schema))?;
        if !self.wallclock
            && let EmaState::Ready { last_at, .. } = &mut restored
        {
            *last_at = None;
        }
        self.state = restored;
        Ok(())
    }
}

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered EMA port")
}

impl<T> EmittingActor<T, ProductPayload> for EmaActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Event: StampedEvent,
    T::StateVersion: From<u16> + PartialEq,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        if input.inlet() != &port("sample") {
            return ActorEffects::emit(
                port("_error"),
                error_payload("InputOutOfDomain: unknown ema inlet"),
            );
        }
        let ProductValue::Float(sample) = input.payload::<T>().value() else {
            return ActorEffects::emit(
                port("_error"),
                error_payload("InputOutOfDomain: ema sample must be Float"),
            );
        };
        let x = sample.get();
        if !x.is_finite() {
            return ActorEffects::emit(
                port("_error"),
                error_payload("InputOutOfDomain: ema sample must be finite"),
            );
        }
        let at = if self.wallclock {
            input.event().recorded_instant()
        } else {
            None
        };
        let (y, n) = match self.state {
            EmaState::Empty => (x, 1),
            EmaState::Ready {
                last_value,
                n,
                last_at,
            } => {
                let distance = match (at, last_at) {
                    (Some(now), Some(previous)) => now.millis().saturating_sub(previous.millis()),
                    (Some(_), None) => 0,
                    (None, _) => 1,
                };
                let y = if distance == 0 {
                    last_value.get()
                } else {
                    x + (last_value.get() - x)
                        * (-(distance as f64) / self.half_life.get() as f64).exp2()
                };
                let n = n
                    .checked_add(1)
                    .expect("the accepted sample count has not been exhausted");
                (y, n)
            }
        };
        let samples =
            i64::try_from(n).expect("the accepted sample ordinal still fits an Int");
        let payload = ProductPayload::new(
            GroundShape::try_new(Shape::Object {
                fields: FieldMap::try_new(vec![
                    (Name::from_static("value"), Shape::Base(BaseShape::Float)),
                    (Name::from_static("samples"), Shape::Base(BaseShape::Int)),
                ])
                .expect("distinct fields"),
                open: false,
            })
            .expect("closed EMA shape"),
            ProductValue::object([
                ("value", ProductValue::Float(FloatValue::new(y))),
                ("samples", ProductValue::Int(samples)),
            ])
            .expect("distinct fields"),
        );
        self.state = EmaState::Ready {
            last_value: FloatValue::new(y),
            n,
            last_at: at,
        };
        ActorEffects::emit(port("ema"), payload)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmaFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
}

impl fmt::Display for EmaFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("ema config was folded for another actor"),
            Self::Config(rejection) => rejection.fmt(formatter),
        }
    }
}

pub(crate) const TIME_BASIS: Slot<Tags> = Slot::new("time_basis", Tags(&["samples", "wallclock"]));
pub(crate) const HALF_LIFE: Slot<NonZeroInterval> = Slot::new("half_life", NonZeroInterval);
const HALF_LIFE_SAMPLES: Slot<PositiveCount> = Slot::new("half_life", PositiveCount);
impl std::error::Error for EmaFactoryError {}

impl From<crate::config::ConfigRejection> for EmaFactoryError {
    fn from(rejection: crate::config::ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

/// Product EMA factory for the two approved time bases.
pub struct EmaFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for EmaFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Event: StampedEvent,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: ActorType = ActorType::Ema;
    const DISPOSITION: circular_runtime::EditDisposition = EMA_DISPOSITION;
    const CHECKPOINTS: bool = EMA_CHECKPOINTS;
    type Grants = T::Grants;
    type Types = T;
    type Instance = EmaActor<T::StateVersion, T::EffectId>;
    type Error = EmaFactoryError;

    fn create(config: &FoldedConfig, _grants: &T::Grants) -> Result<Self::Instance, Self::Error> {
        let value = config
            .for_type(ActorType::Ema)
            .map_err(|_| EmaFactoryError::InvalidConfig)?;
        let schema = crate::registration(ActorType::Ema).spec().config();
        let mut fields = schema.open(value)?;
        let wallclock = schema.read(&mut fields, &TIME_BASIS)? == "wallclock";
        let half_life = if wallclock {
            schema.read(&mut fields, &HALF_LIFE)?.non_zero()
        } else {
            schema.read(&mut fields, &HALF_LIFE_SAMPLES)?
        };
        let mut actor = EmaActor::new(half_life);
        actor.wallclock = wallclock;
        Ok(actor)
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
    use circular_runtime::ActorEffect;
    use std::convert::Infallible;

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct Run;
    impl StreamIdentity for Run {}
    #[derive(Clone, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
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
    struct PayloadTypes;
    impl ActorTypes for PayloadTypes {
        type Stream = Run;
        type Event = ProductPayload;
        type Payload = ProductPayload;
        type EffectId = u64;
        type StateVersion = u16;
        type Observation = ();
        type Grants = ();
        fn payload(event: &Self::Event) -> &Self::Payload {
            event
        }
    }
    type Actor = EmaActor<u16, u64>;

    fn config(value: ProductValue) -> FoldedConfig {
        FoldedConfig::minted(
            ActorType::Ema,
            ProductValue::object([("half_life", value)]).unwrap(),
        )
    }
    fn actor() -> Actor {
        EmaFactory::<Types>::create(&config(ProductValue::Int(1)), &()).unwrap()
    }
    fn ready(value: f64, n: u64) -> EmaState {
        EmaState::Ready {
            last_value: FloatValue::new(value),
            n,
            last_at: None,
        }
    }
    fn event(
        inlet: &str,
        value: ProductValue,
        time: u64,
    ) -> ActorInput<<Types as ActorTypes>::Event> {
        let payload = ProductPayload::new(GroundShape::try_new(Shape::Any).unwrap(), value);
        let stamp = Stamp::from_event_producer(
            Tick::new(time),
            Producer,
            Sequence::new(1).unwrap(),
            circular_core::RevisionEpochId::new(1).expect("first revision"),
        );
        ActorInput::new(
            port(inlet),
            admit(
                Run,
                Emission::from_runtime(emit(payload), Causality::Source, None),
                stamp,
            )
            .unwrap(),
        )
    }
    fn drive(
        actor: &mut Actor,
        inlet: &str,
        value: ProductValue,
        time: u64,
    ) -> ActorEffects<ProductPayload> {
        drive_input::<Types>(actor, &event(inlet, value, time))
    }
    fn drive_input<T>(
        actor: &mut Actor,
        input: &ActorInput<T::Event>,
    ) -> ActorEffects<ProductPayload>
    where
        T: ActorTypes<
                Stream = Run,
                Payload = ProductPayload,
                StateVersion = u16,
                EffectId = u64,
                Grants = (),
            >,
        T::Event: StampedEvent,
    {
        let named = NamedActorId::new(ScopeId::root(), circular_plan::Name::from_normalized("ema"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .unwrap();
        let incarnation = Incarnation::new(Run, named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let config = Config::default();
        let context = ActorContext::new(&actor_id, &incarnation, &config, &());
        <Actor as EmittingActor<T, ProductPayload>>::on_event(actor, input, &context)
    }
    fn sample(actor: &mut Actor, value: f64, time: u64, expected: f64, samples: i64) {
        let effects = drive(actor, "sample", ProductValue::float(value), time);
        let [
            ActorEffect::Emit {
                port: outlet,
                payload,
                ..
            },
        ] = effects.as_slice()
        else {
            panic!("EMA Emit")
        };
        assert_eq!(outlet, &port("ema"));
        assert_eq!(
            payload.value(),
            &ProductValue::object([
                ("value", ProductValue::float(expected)),
                ("samples", ProductValue::Int(samples))
            ])
            .unwrap()
        );
        assert_eq!(
            actor.state(),
            &ready(expected, u64::try_from(samples).unwrap())
        );
    }

    #[test]
    fn accepted_samples_initialize_and_halve_at_each_sample() {
        let mut actor = actor();
        sample(&mut actor, 10.0, 0, 10.0, 1);
        sample(&mut actor, 0.0, 100, 5.0, 2);
        sample(&mut actor, 0.0, 200, 2.5, 3);
    }
    #[test]
    fn equal_and_backward_stamps_advance_one_sample_each() {
        let mut actor = actor();
        sample(&mut actor, 10.0, 100, 10.0, 1);
        sample(&mut actor, 0.0, 100, 5.0, 2);
        sample(&mut actor, 0.0, 0, 2.5, 3);
    }
    #[test]
    fn unstamped_payload_events_advance_one_sample_each() {
        let mut actor =
            EmaFactory::<PayloadTypes>::create(&config(ProductValue::Int(1)), &()).unwrap();
        for (x, y, n) in [(10.0, 10.0, 1), (0.0, 5.0, 2), (0.0, 2.5, 3)] {
            let input = ActorInput::new(
                port("sample"),
                ProductPayload::new(
                    GroundShape::try_new(Shape::Base(BaseShape::Float)).unwrap(),
                    ProductValue::float(x),
                ),
            );
            let effects = drive_input::<PayloadTypes>(&mut actor, &input);
            let [
                ActorEffect::Emit {
                    port: outlet,
                    payload,
                    ..
                },
            ] = effects.as_slice()
            else {
                panic!("EMA Emit")
            };
            assert_eq!(outlet, &port("ema"));
            assert_eq!(
                payload.value(),
                &ProductValue::object([
                    ("value", ProductValue::float(y)),
                    ("samples", ProductValue::Int(n))
                ])
                .unwrap()
            );
            assert_eq!(actor.state(), &ready(y, u64::try_from(n).unwrap()));
        }
    }
    #[test]
    fn half_life_two_uses_one_sample_distance() {
        let mut actor = EmaFactory::<Types>::create(&config(ProductValue::Int(2)), &()).unwrap();
        sample(&mut actor, 10.0, 0, 10.0, 1);
        let effects = drive(&mut actor, "sample", ProductValue::float(0.0), 10_000);
        let [
            ActorEffect::Emit {
                port: outlet,
                payload,
                ..
            },
        ] = effects.as_slice()
        else {
            panic!("EMA Emit")
        };
        assert_eq!(outlet, &port("ema"));
        let ProductValue::Object(fields) = payload.value() else {
            panic!("EMA object")
        };
        let Some(ProductValue::Float(value)) = fields.get("value") else {
            panic!("EMA value")
        };
        assert!((value.get() - 7.071_067_811_865_475_5).abs() < 1e-9);
        assert_eq!(fields.get("samples"), Some(&ProductValue::Int(2)));
        assert_eq!(actor.state(), &ready(value.get(), 2));
    }
    #[test]
    fn rejected_samples_and_unknown_inlets_leave_empty_and_ready_state_unchanged() {
        for state in [EmaState::Empty, ready(10.0, 100)] {
            let mut actor = actor();
            actor.set_state(state.clone());
            for (inlet, value) in [
                ("sample", ProductValue::float(f64::NAN)),
                ("sample", ProductValue::float(f64::INFINITY)),
                ("sample", ProductValue::float(f64::NEG_INFINITY)),
                ("sample", ProductValue::Int(10)),
                ("unknown", ProductValue::float(1.0)),
            ] {
                let before = actor.checkpoint();
                let effects = drive(&mut actor, inlet, value, 200);
                let [
                    ActorEffect::Emit {
                        port: outlet,
                        payload,
                        ..
                    },
                ] = effects.as_slice()
                else {
                    panic!("error Emit only")
                };
                assert_eq!(outlet, &port("_error"));
                let ProductValue::String(message) = payload.value() else {
                    panic!("error message")
                };
                assert!(message.starts_with("InputOutOfDomain:"));
                assert_eq!(actor.checkpoint(), before);
            }
            match state {
                EmaState::Empty => sample(&mut actor, 0.0, 200, 0.0, 1),
                EmaState::Ready { n, .. } => {
                    sample(&mut actor, 0.0, 200, 5.0, i64::try_from(n + 1).unwrap())
                }
            }
        }
    }
    #[test]
    fn checkpoint_round_trip_preserves_both_fields_and_continues_the_recurrence() {
        for state in [
            EmaState::Empty,
            ready(0.0, 1),
            ready(-0.0, 100),
            ready(f64::MIN_POSITIVE, 200),
            ready(-1.7976931348623157e308, u64::MAX),
        ] {
            let mut source = actor();
            source.set_state(state.clone());
            let checkpoint = source.checkpoint().unwrap();
            assert_eq!(*checkpoint.schema(), 4);
            let mut restored = actor();
            restored.restore(checkpoint.clone()).unwrap();
            assert_eq!(restored.state(), &state);
            assert_eq!(restored.checkpoint(), Some(checkpoint));
        }
        let mut source = actor();
        sample(&mut source, 10.0, 0, 10.0, 1);
        let mut restored = actor();
        restored.restore(source.checkpoint().unwrap()).unwrap();
        sample(&mut restored, 0.0, 100, 5.0, 2);
        assert_eq!(actor().state(), &EmaState::Empty);
        assert_ne!(ready(0.0, 100).encode(), ready(-0.0, 100).encode());
        assert_eq!(
            ready(10.0, 100).encode().as_ref(),
            &[1, 64, 36, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100, 1]
        );
    }
    #[test]
    fn old_schema_returns_diagnostic_and_fresh_actor_stays_empty() {
        let mut actor = actor();
        let old = vec![1_u8, 64, 36, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100];
        assert!(matches!(
            actor.restore(ActorState::new(3, old)),
            Err(ActorRestoreError::SchemaBeyondLadder { schema: 3 })
        ));
        assert_eq!(actor.state(), &EmaState::Empty);
        sample(&mut actor, 10.0, 100, 10.0, 1);
    }
    #[test]
    fn malformed_and_nonfinite_state_bytes_are_refused_without_partial_restore() {
        for bytes in [vec![], vec![9], vec![1, 2], vec![0, 0]] {
            assert_eq!(EmaState::decode(&bytes), Err(EmaStateError::Decode));
            let mut actor = actor();
            assert!(matches!(
                actor.restore(ActorState::new(4, bytes)),
                Err(ActorRestoreError::DecodeFailed { schema: 4 })
            ));
            assert_eq!(actor.state(), &EmaState::Empty);
        }
        assert_eq!(
            EmaState::decode(&ready(5.0, 0).encode()),
            Err(EmaStateError::Invariant)
        );
        let mut actor = actor();
        actor.set_state(ready(5.0, 100));
        for bits in [
            0x7ff8_0000_0000_0000_u64,
            0x7ff0_0000_0000_0001,
            0x7ff0_0000_0000_0000,
            0xfff0_0000_0000_0000,
        ] {
            let mut bytes = vec![1];
            bytes.extend_from_slice(&bits.to_be_bytes());
            bytes.extend_from_slice(&200_u64.to_be_bytes());
            bytes.push(1);
            assert_eq!(EmaState::decode(&bytes), Err(EmaStateError::Invariant));
            assert!(matches!(
                actor.restore(ActorState::new(4, bytes)),
                Err(ActorRestoreError::StateInvariantViolated { schema: 4 })
            ));
            assert_eq!(actor.state(), &ready(5.0, 100));
        }
    }
    #[test]
    fn half_life_factory_and_admission_share_positive_count() {
        let schema = crate::get(ActorType::Ema).config();
        for value in [
            ProductValue::Int(0),
            ProductValue::Int(-1),
            ProductValue::float(1.5),
            ProductValue::float(1.0),
        ] {
            let config = config(value);
            assert!(EmaFactory::<Types>::create(&config, &()).is_err());
            assert!(
                schema
                    .create_inputs()
                    .unwrap()
                    .admit(config.value())
                    .is_err()
            );
        }
        for value in [ProductValue::Int(1), ProductValue::Int(100)] {
            let config = config(value);
            assert!(EmaFactory::<Types>::create(&config, &()).is_ok());
            assert!(
                schema
                    .create_inputs()
                    .unwrap()
                    .admit(config.value())
                    .is_ok()
            );
        }
        let mut actor = actor();
        sample(&mut actor, 10.0, 0, 10.0, 1);
        assert_eq!(
            actor.on_config_change(&config(ProductValue::Int(200))),
            ConfigChangeOutcome::ReplaceIncarnation
        );
        assert_eq!(actor.state(), &ready(10.0, 1));
        assert_eq!(
            EmaFactory::<Types>::create(&config(ProductValue::Int(200)), &())
                .unwrap()
                .state(),
            &EmaState::Empty
        );
    }

    struct RecordedSample {
        payload: ProductPayload,
        at: Option<RecordedInstant>,
    }
    impl StampedEvent for RecordedSample {
        fn recorded_instant(&self) -> Option<RecordedInstant> {
            self.at
        }
    }
    struct RecordedTypes;
    impl ActorTypes for RecordedTypes {
        type Stream = Run;
        type Event = RecordedSample;
        type Payload = ProductPayload;
        type EffectId = u64;
        type StateVersion = u16;
        type Observation = ();
        type Grants = ();
        fn payload(event: &Self::Event) -> &ProductPayload {
            &event.payload
        }
    }

    fn timed_actor(half_life: i64, basis: &str) -> Actor {
        EmaFactory::<RecordedTypes>::create(
            &FoldedConfig::minted(
                ActorType::Ema,
                ProductValue::object([
                    ("half_life", ProductValue::Int(half_life)),
                    ("time_basis", ProductValue::String(basis.to_owned())),
                ])
                .unwrap(),
            ),
            &(),
        )
        .unwrap()
    }

    fn recorded_sample(actor: &mut Actor, x: f64, at: Option<u64>, expected: f64, n: i64) {
        let input = ActorInput::new(
            port("sample"),
            RecordedSample {
                payload: ProductPayload::new(
                    GroundShape::try_new(Shape::Base(BaseShape::Float)).unwrap(),
                    ProductValue::float(x),
                ),
                at: at.map(RecordedInstant::from_millis),
            },
        );
        let effects = drive_input::<RecordedTypes>(actor, &input);
        let [
            ActorEffect::Emit {
                port: outlet,
                payload,
                ..
            },
        ] = effects.as_slice()
        else {
            panic!("single EMA")
        };
        assert_eq!(outlet.as_str(), "ema");
        assert_eq!(
            payload.value(),
            &ProductValue::object([
                ("value", ProductValue::float(expected)),
                ("samples", ProductValue::Int(n))
            ])
            .unwrap()
        );
        assert_eq!(
            actor.state(),
            &EmaState::Ready {
                last_value: FloatValue::new(expected),
                n: n as u64,
                last_at: if actor.wallclock {
                    at.map(RecordedInstant::from_millis)
                } else {
                    None
                }
            }
        );
    }

    #[test]
    fn recorded_millisecond_replay_restores_same_next_value_with_or_without_checkpoint() {
        for restore_between in [false, true] {
            let mut actor = timed_actor(1000, "wallclock");
            for (x, at, y, n) in [(0.0, 0, 0.0, 1), (8.0, 1000, 4.0, 2), (0.0, 2000, 2.0, 3)] {
                recorded_sample(&mut actor, x, Some(at), y, n);
                let checkpoint = actor.checkpoint().unwrap();
                let mut restored = timed_actor(1000, "wallclock");
                restored.restore(checkpoint).unwrap();
                assert_eq!(restored.state(), actor.state());
                if restore_between {
                    actor = restored;
                }
            }
            let (schema, bytes) = actor.checkpoint().unwrap().into_parts();
            assert_eq!(schema, 4);
            assert_eq!(
                bytes.as_ref(),
                &[
                    1, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 9, 0, 0, 0, 0, 0, 0, 7, 208
                ]
            );
        }
    }

    #[test]
    fn zero_and_backward_distance_preserve_value_without_reordering_recorded_time() {
        let mut actor = timed_actor(1000, "wallclock");
        for (x, at, y, n) in [
            (8.0, 1000, 8.0, 1),
            (0.0, 1000, 8.0, 2),
            (16.0, 900, 8.0, 3),
            (0.0, 1900, 4.0, 4),
        ] {
            recorded_sample(&mut actor, x, Some(at), y, n);
        }
    }

    #[test]
    fn missing_arrival_uses_samples_and_missing_last_at_does_not_decay() {
        let mut actor = timed_actor(1, "wallclock");
        for (x, at, y, n) in [
            (0.0, None, 0.0, 1),
            (8.0, None, 4.0, 2),
            (0.0, Some(1000), 4.0, 3),
            (0.0, Some(1001), 2.0, 4),
        ] {
            recorded_sample(&mut actor, x, at, y, n);
        }
        let mut actor = timed_actor(1, "wallclock");
        sample(&mut actor, 0.0, 123, 0.0, 1);
        sample(&mut actor, 8.0, 456, 4.0, 2);
        sample(&mut actor, 0.0, 789, 2.0, 3);
    }

    #[test]
    fn samples_ignore_recorded_times_and_keep_null_after_mode_transition() {
        let mut actor = timed_actor(1, "samples");
        for (x, at, y, n) in [
            (0.0, 2000, 0.0, 1),
            (8.0, 1000, 4.0, 2),
            (0.0, 1000, 2.0, 3),
        ] {
            recorded_sample(&mut actor, x, Some(at), y, n);
        }
        let mut timed = timed_actor(1000, "wallclock");
        timed.restore(actor.checkpoint().unwrap()).unwrap();
        recorded_sample(&mut timed, 99.0, Some(5000), 2.0, 4);
        actor.restore(timed.checkpoint().unwrap()).unwrap();
        assert_eq!(actor.state(), &ready(2.0, 4));
        recorded_sample(&mut actor, 0.0, Some(6000), 1.0, 5);
    }

    #[test]
    fn time_basis_is_closed_and_defaulted_by_admission_and_factory() {
        let missing = ProductValue::object([("half_life", ProductValue::Int(1))]).unwrap();
        let admitted = crate::admit_registered_create(ActorType::Ema, &missing).unwrap();
        assert_eq!(
            admitted.config(),
            &ProductValue::object([
                ("half_life", ProductValue::Int(1)),
                ("time_basis", ProductValue::String("samples".to_owned()))
            ])
            .unwrap()
        );
        for value in [
            ProductValue::String("samples".to_owned()),
            ProductValue::String("wallclock".to_owned()),
            ProductValue::String("clock".to_owned()),
            ProductValue::Null,
            ProductValue::Int(0),
        ] {
            let valid =
                matches!(&value, ProductValue::String(s) if s == "samples" || s == "wallclock");
            let value = ProductValue::object([
                ("half_life", ProductValue::Int(1000)),
                ("time_basis", value),
            ])
            .unwrap();
            assert_eq!(
                crate::admit_registered_create(ActorType::Ema, &value).is_ok(),
                valid
            );
            assert_eq!(
                EmaFactory::<RecordedTypes>::create(
                    &FoldedConfig::minted(ActorType::Ema, value),
                    &()
                )
                .is_ok(),
                valid
            );
        }
    }
}
