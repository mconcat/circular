
use crate::actor_support::StampedEvent;
use crate::arming::ArmingCorrelation;
use crate::config::{ConfigRejection, Interval, NonZeroInterval, Slot};
use crate::{ActorType, ProductPayload, ProductValue, TIMER_PORT_NAME};
use circular_core::{
    Boundary, Ceilings, GroundShape, Millis, NonZeroMillis, PortId, RecordedInstant,
};
use circular_expr::shapes::ShapeEnv;
use circular_expr::snippet::{self, Snippet};
use circular_runtime::{
    ActorContext, ActorEffect, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, DeadLettering, EditableActor, Effect, EmittingActor, EmittingActorFactory,
    FoldedConfig, ReasonDecl, ScheduleSpec, Suppression,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;
use std::sync::LazyLock;

pub const WINDOWED_REDUCE_STATE_SCHEMA: u16 = 1;

pub const REDUCE_FAILED: &str = "reduce_failed";

pub const SAMPLE_INLET: &str = "sample";
pub const AGGREGATE_OUTLET: &str = "aggregate";

static SUPPRESSION_REASONS: LazyLock<ReasonDecl<Suppression>> = LazyLock::new(|| {
    ReasonDecl::try_from_names([crate::STALE_TIMER_FIRE]).expect("one stale scheduled-fire reason")
});

static DEAD_LETTER_REASONS: LazyLock<ReasonDecl<DeadLettering>> =
    LazyLock::new(|| ReasonDecl::try_from_names([REDUCE_FAILED]).expect("one fold-failure reason"));

fn suppress(reason: &str) -> ActorEffects<ProductPayload> {
    ActorEffects::singleton(ActorEffect::suppress(
        SUPPRESSION_REASONS
            .resolve(reason)
            .expect("declared suppression reason"),
    ))
}

fn reduce_failed(sample: ProductValue) -> ActorEffects<ProductPayload> {
    let shape = sample
        .kind()
        .base_shape()
        .map_or(crate::Shape::Any, crate::Shape::Base);
    ActorEffects::singleton(ActorEffect::dead_letter(
        ProductPayload::new(
            GroundShape::try_new(shape).expect("the default shape is a ground shape"),
            sample,
        ),
        DEAD_LETTER_REASONS
            .resolve(REDUCE_FAILED)
            .expect("declared dead letter reason"),
    ))
}

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered windowed_reduce port")
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowedReduceSample {
    value: ProductValue,
    admitted_at: RecordedInstant,
}

impl WindowedReduceSample {
    #[must_use]
    pub const fn value(&self) -> &ProductValue {
        &self.value
    }

    #[must_use]
    pub const fn admitted_at(&self) -> RecordedInstant {
        self.admitted_at
    }
}

pub struct WindowedReduceActor<V, I> {
    samples: Vec<WindowedReduceSample>,
    arming: Option<ArmingCorrelation>,
    last_correlation: u64,
    window_length: Millis,
    emission_period: NonZeroMillis,
    seed: ProductValue,
    reduce: Snippet,
    aggregate_shape: GroundShape<crate::Name>,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> WindowedReduceActor<V, I> {
    #[must_use]
    pub fn samples(&self) -> &[WindowedReduceSample] {
        &self.samples
    }

    #[must_use]
    pub const fn window_length(&self) -> Millis {
        self.window_length
    }

    #[must_use]
    pub const fn emission_period(&self) -> NonZeroMillis {
        self.emission_period
    }

    fn arm(&mut self) -> ActorEffects<ProductPayload> {
        let next = self
            .last_correlation
            .checked_add(1)
            .expect("windowed_reduce arming correlation exhausted its u64 space");
        let correlation = ArmingCorrelation::new(next);
        self.last_correlation = next;
        self.arming = Some(correlation);
        ActorEffects::external(Effect::schedule(ScheduleSpec::new(
            self.emission_period,
            correlation.correlation(),
        )))
    }

    fn window_start(&self, at: RecordedInstant) -> RecordedInstant {
        RecordedInstant::from_millis(at.millis().saturating_sub(self.window_length.get()))
    }

    fn fold(
        &self,
        at: RecordedInstant,
    ) -> Result<Option<ProductValue>, (ProductValue, circular_expr::eval::EvalError)> {
        let start = self.window_start(at);
        let mut accumulator = self.seed.clone();
        let mut members = 0usize;
        for sample in &self.samples {
            if sample.admitted_at < start || sample.admitted_at >= at {
                continue;
            }
            members += 1;
            let bindings = BTreeMap::from([
                (
                    circular_expr::REDUCE_ACCUMULATOR.to_owned(),
                    accumulator.clone(),
                ),
                (SAMPLE_INLET.to_owned(), sample.value.clone()),
            ]);
            accumulator = self
                .reduce
                .evaluate(&bindings)
                .map_err(|error| (sample.value.clone(), error))?;
        }
        Ok((members > 0).then_some(accumulator))
    }

    fn prune(&mut self, at: RecordedInstant) {
        let next_fire = at.saturating_add(self.emission_period.get());
        let keep_from = self.window_start(next_fire);
        self.samples
            .retain(|sample| sample.admitted_at >= keep_from);
    }

    fn fire(
        &mut self,
        value: &ProductValue,
        at: Option<RecordedInstant>,
    ) -> ActorEffects<ProductPayload> {
        let ProductValue::UInt(correlation) = value else {
            return suppress(crate::STALE_TIMER_FIRE);
        };
        if self.arming.map(ArmingCorrelation::get) != Some(*correlation) {
            return suppress(crate::STALE_TIMER_FIRE);
        }
        self.arming = None;
        let Some(at) = at else {
            return suppress(crate::STALE_TIMER_FIRE);
        };
        let folded = self.fold(at);
        self.prune(at);
        let emission = match folded {
            Ok(Some(value)) => ActorEffects::emit(port(AGGREGATE_OUTLET), self.aggregate(value)),
            Ok(None) => ActorEffects::empty(),
            Err((sample, _diagnosis)) => reduce_failed(sample),
        };
        if self.samples.is_empty() {
            return emission;
        }
        emission.concat(self.arm())
    }

    fn aggregate(&self, value: ProductValue) -> ProductPayload {
        ProductPayload::new(self.aggregate_shape.clone(), value)
    }
}

impl<V, I> EditableActor for WindowedReduceActor<V, I>
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
        self.try_checkpoint()
            .expect("canonical encoding of the windowed_reduce state")
    }

    fn try_checkpoint(&self) -> Result<Option<ActorState<V>>, circular_core::CodecError> {
        let samples = self.samples.iter().map(|sample| {
            ProductValue::array([
                sample.value.clone(),
                ProductValue::UInt(sample.admitted_at.millis()),
            ])
        });
        let value = ProductValue::array([
            ProductValue::Array(samples.collect()),
            ProductValue::UInt(self.last_correlation),
        ]);
        let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::ActorState))?;
        Ok(Some(ActorState::new(
            V::from(WINDOWED_REDUCE_STATE_SCHEMA),
            bytes,
        )))
    }

    fn restore(&mut self, state: ActorState<V>) -> Result<(), ActorRestoreError<V>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(WINDOWED_REDUCE_STATE_SCHEMA) {
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
        let Ok([samples, last]) = <[ProductValue; 2]>::try_from(fields) else {
            return Err(decode());
        };
        let ProductValue::UInt(last) = last else {
            return Err(decode());
        };
        let ProductValue::Array(samples) = samples else {
            return Err(decode());
        };
        let mut restored = Vec::with_capacity(samples.len());
        let mut previous: Option<RecordedInstant> = None;
        for sample in samples {
            let ProductValue::Array(fields) = sample else {
                return Err(decode());
            };
            let Ok([value, at]) = <[ProductValue; 2]>::try_from(fields) else {
                return Err(decode());
            };
            let ProductValue::UInt(at) = at else {
                return Err(decode());
            };
            if !matches!(
                value,
                ProductValue::Int(_) | ProductValue::UInt(_) | ProductValue::Float(_)
            ) {
                return Err(invariant());
            }
            let at = RecordedInstant::from_millis(at);
            if previous.is_some_and(|earlier| at < earlier) {
                return Err(invariant());
            }
            previous = Some(at);
            restored.push(WindowedReduceSample {
                value,
                admitted_at: at,
            });
        }
        self.samples = restored;
        self.arming = None;
        self.last_correlation = last;
        Ok(())
    }
}

impl<T> EmittingActor<T, ProductPayload> for WindowedReduceActor<T::StateVersion, T::EffectId>
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
        let payload = input.payload::<T>();
        let at = input.event().recorded_instant();
        if input.inlet().as_str() == TIMER_PORT_NAME {
            return self.fire(payload.value(), at);
        }
        if input.inlet().as_str() != SAMPLE_INLET {
            return ActorEffects::empty();
        }
        if !matches!(
            payload.value(),
            ProductValue::Int(_) | ProductValue::UInt(_) | ProductValue::Float(_)
        ) {
            return ActorEffects::empty();
        }
        let Some(admitted_at) = at else {
            return ActorEffects::empty();
        };
        self.samples.push(WindowedReduceSample {
            value: payload.value().clone(),
            admitted_at,
        });
        if self.arming.is_some() {
            return ActorEffects::empty();
        }
        self.arm()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WindowedReduceFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
    Reduce(String),
    ReduceOutputShapeUnresolved,
    ReduceKindSplit(circular_expr::shapes::KindSplit),
}

impl fmt::Display for WindowedReduceFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => {
                formatter.write_str("windowed_reduce config was folded for another actor")
            }
            Self::Config(rejection) => rejection.fmt(formatter),
            Self::Reduce(detail) => {
                let detail = detail.strip_prefix("ConfigRejected: ").unwrap_or(detail);
                write!(formatter, "config.reduce was rejected: {detail}")
            }
            Self::ReduceOutputShapeUnresolved => {
                formatter.write_str("config.reduce output shape is not ground")
            }
            Self::ReduceKindSplit(split) => write!(
                formatter,
                "config.seed and the sample inlet split the fold's kinds: {split}"
            ),
        }
    }
}

impl std::error::Error for WindowedReduceFactoryError {}

impl From<crate::config::ConfigRejection> for WindowedReduceFactoryError {
    fn from(rejection: crate::config::ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub(crate) const WINDOW_LENGTH: Slot<Interval> = Slot::new("window_length", Interval);
pub(crate) const EMISSION_PERIOD: Slot<NonZeroInterval> =
    Slot::new("emission_period", NonZeroInterval);

fn sample_inlet_shape() -> circular_core::Shape<String> {
    let ports = crate::registration(ActorType::WindowedReduce)
        .spec()
        .ports();
    let inlet = ports
        .fixed()
        .inlets()
        .iter()
        .find(|inlet| inlet.id().as_str() == SAMPLE_INLET)
        .expect("registered sample inlet");
    crate::types::to_unnamed_shape(inlet.ty().item())
}

fn seed_shape(seed: &ProductValue) -> circular_core::Shape<String> {
    seed.kind()
        .base_shape()
        .map_or(circular_core::Shape::Any, circular_core::Shape::Base)
}

pub struct WindowedReduceFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for WindowedReduceFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Event: StampedEvent,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: ActorType = ActorType::WindowedReduce;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = WindowedReduceActor<T::StateVersion, T::EffectId>;
    type Error = WindowedReduceFactoryError;

    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        let WindowedReduceConfig {
            window_length,
            emission_period,
            seed,
            reduce,
            aggregate_shape,
        } = WindowedReduceConfig::read(config)?;
        Ok(WindowedReduceActor {
            samples: Vec::new(),
            arming: None,
            last_correlation: 0,
            window_length,
            emission_period,
            seed,
            reduce,
            aggregate_shape,
            marker: PhantomData,
        })
    }
}

pub(crate) struct WindowedReduceConfig {
    window_length: Millis,
    emission_period: NonZeroMillis,
    seed: ProductValue,
    reduce: Snippet,
    aggregate_shape: GroundShape<crate::Name>,
}

impl WindowedReduceConfig {
    pub(crate) fn read(config: &FoldedConfig) -> Result<Self, WindowedReduceFactoryError> {
        let value = config
            .for_type(ActorType::WindowedReduce)
            .map_err(|_| WindowedReduceFactoryError::InvalidConfig)?;
        let schema = crate::registration(ActorType::WindowedReduce)
            .spec()
            .config();
        let mut fields = schema.open(value)?;
        let window_length = schema.read(&mut fields, &WINDOW_LENGTH)?;
        let emission_period = schema.read(&mut fields, &EMISSION_PERIOD)?;
        let seed = schema.raw(&mut fields, "seed")?.clone();
        let reduce = schema.raw(&mut fields, "reduce")?;
        let reduce =
            snippet::from_config_value(reduce, &reduce_bindings(), circular_expr::EvalMode::Reduce)
                .map_err(|error| WindowedReduceFactoryError::Reduce(error.to_string()))?;
        if let Some(split) = reduce.kind_split(&ShapeEnv::from([
            (
                circular_expr::REDUCE_ACCUMULATOR.to_owned(),
                seed_shape(&seed),
            ),
            (SAMPLE_INLET.to_owned(), sample_inlet_shape()),
        ])) {
            return Err(WindowedReduceFactoryError::ReduceKindSplit(split));
        }
        let environment = ShapeEnv::from([(SAMPLE_INLET.to_owned(), sample_inlet_shape())]);
        let produced = reduce.output_shape(&environment);
        let aggregate_shape = GroundShape::try_new(crate::types::from_unnamed_shape(&produced))
            .map_err(|_| WindowedReduceFactoryError::ReduceOutputShapeUnresolved)?;
        Ok(Self {
            window_length,
            emission_period,
            seed,
            reduce,
            aggregate_shape,
        })
    }
}

pub(crate) fn judge(
    config: &FoldedConfig,
    _inlets: &crate::inlet_shapes::ResolvedInletShapes,
) -> Result<(), WindowedReduceFactoryError> {
    WindowedReduceConfig::read(config).map(drop)
}

fn reduce_bindings() -> BTreeSet<String> {
    let schema = crate::registration(ActorType::WindowedReduce)
        .spec()
        .config();
    let slot = schema
        .top_level_slot("reduce")
        .and_then(crate::config::ConfigSlot::snippet)
        .expect("the registered reduce snippet field");
    slot.inlets()
        .iter()
        .map(|inlet| inlet.as_str().to_owned())
        .chain(
            slot.mode()
                .implicit_bindings()
                .iter()
                .map(|name| (*name).to_owned()),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{
        Causality, Emission, Event, ProducerIdentity, Sequence, Stamp, StreamIdentity, Tick, admit,
        emit,
    };
    use circular_plan::{Config, Generation, GenerationVector, Incarnation, NamedActorId, ScopeId};
    use circular_runtime::{EditDisposition, ScheduleCorrelation};
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

    type Actor = WindowedReduceActor<u16, u64>;

    fn config_of(window: i64, period: i64, reduce: &str, seed: ProductValue) -> FoldedConfig {
        FoldedConfig::minted(
            ActorType::WindowedReduce,
            ProductValue::object([
                ("window_length", ProductValue::Int(window)),
                ("emission_period", ProductValue::Int(period)),
                ("reduce", ProductValue::String(reduce.to_owned())),
                ("seed", seed),
            ])
            .expect("four fields"),
        )
    }

    fn new_actor(window: i64, period: i64) -> Actor {
        WindowedReduceFactory::<Types>::create(
            &config_of(window, period, "acc + sample", ProductValue::float(0.0)),
            &(),
        )
        .expect("the four registered fields")
    }

    fn payload(value: ProductValue) -> ProductPayload {
        ProductPayload::new(
            GroundShape::try_new(crate::Shape::Any).expect("Any is a ground shape"),
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
            circular_plan::Name::from_normalized("windowed_reduce"),
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

    fn sample(actor: &mut Actor, value: f64, at: u64) -> ActorEffects<ProductPayload> {
        drive(
            actor,
            &arrival(SAMPLE_INLET, ProductValue::float(value), Some(at)),
        )
    }

    fn fire(actor: &mut Actor, correlation: u64, at: u64) -> ActorEffects<ProductPayload> {
        drive(
            actor,
            &arrival(TIMER_PORT_NAME, ProductValue::UInt(correlation), Some(at)),
        )
    }

    fn correlation_of(correlation: ScheduleCorrelation) -> u64 {
        correlation.get()
    }

    fn armed(effects: &ActorEffects<ProductPayload>) -> Option<(u64, NonZeroMillis)> {
        effects.iter().find_map(|effect| match effect {
            ActorEffect::External(Effect::Schedule { spec }) => {
                Some((correlation_of(spec.correlation()), spec.after()))
            }
            _ => None,
        })
    }

    fn scheduled(effects: &ActorEffects<ProductPayload>) -> (u64, NonZeroMillis) {
        armed(effects).expect("one schedule")
    }

    fn aggregate(effects: &ActorEffects<ProductPayload>) -> Option<ProductValue> {
        effects.iter().find_map(|effect| match effect {
            ActorEffect::Emit {
                port: outlet,
                payload,
                ..
            } => {
                assert_eq!(
                    outlet.as_str(),
                    AGGREGATE_OUTLET,
                    "the outlet is the single aggregate"
                );
                Some(payload.value().clone())
            }
            _ => None,
        })
    }

    fn suppressed(effects: &ActorEffects<ProductPayload>) -> Option<String> {
        effects.iter().find_map(|effect| match effect {
            ActorEffect::Suppress { reason } => Some(reason.name().to_owned()),
            _ => None,
        })
    }

    fn dead_lettered(effects: &ActorEffects<ProductPayload>) -> Option<(String, ProductPayload)> {
        effects.iter().find_map(|effect| match effect {
            ActorEffect::DeadLetter { subject, reason } => {
                Some((reason.name().to_owned(), subject.clone()))
            }
            _ => None,
        })
    }

    #[test]
    fn samples_keep_consumption_order_and_only_unreachable_ones_are_dropped() {
        let mut actor = new_actor(100, 50);
        for (value, at) in [(1.0, 10), (2.0, 60), (3.0, 110)] {
            sample(&mut actor, value, at);
        }
        assert_eq!(
            actor
                .samples()
                .iter()
                .map(|sample| sample.admitted_at().millis())
                .collect::<Vec<_>>(),
            vec![10, 60, 110],
            "sequence order is consumption order"
        );
        fire(&mut actor, 1, 150);
        assert_eq!(
            actor
                .samples()
                .iter()
                .map(|sample| sample.admitted_at().millis())
                .collect::<Vec<_>>(),
            vec![110]
        );
    }

    #[test]
    fn the_checkpoint_round_trips_the_sample_sequence() {
        let mut actor = new_actor(1_000, 100);
        for (value, at) in [(1.0, 10), (2.0, 20)] {
            sample(&mut actor, value, at);
        }
        let state = actor
            .checkpoint()
            .expect("checkpoint of the sample sequence");
        assert_eq!(state.schema(), &WINDOWED_REDUCE_STATE_SCHEMA);
        let mut restored = new_actor(1_000, 100);
        restored
            .restore(state.clone())
            .expect("restore under the same schema");
        assert_eq!(restored.samples(), actor.samples());
        assert_eq!(
            restored.checkpoint().map(ActorState::into_parts),
            Some(state.into_parts()),
            "re-serialized bytes are the same"
        );

        let effects = sample(&mut restored, 3.0, 30);
        let (correlation, after) = scheduled(&effects);
        assert_eq!(after.get().get(), 100);
        let fired = fire(&mut restored, correlation, 40);
        assert_eq!(aggregate(&fired), Some(ProductValue::float(6.0)));
    }

    #[test]
    fn restore_separates_the_ladder_the_decode_and_the_invariant() {
        let mut actor = new_actor(1_000, 100);
        assert!(matches!(
            actor.restore(ActorState::new(
                WINDOWED_REDUCE_STATE_SCHEMA + 1,
                Vec::new()
            )),
            Err(ActorRestoreError::SchemaBeyondLadder { .. })
        ));
        assert!(matches!(
            actor.restore(ActorState::new(
                WINDOWED_REDUCE_STATE_SCHEMA,
                vec![0xff, 0xff, 0xff]
            )),
            Err(ActorRestoreError::DecodeFailed { .. })
        ));
        let backwards = circular_core::encode(
            &ProductValue::array([
                ProductValue::Array(vec![
                    ProductValue::array([ProductValue::Int(1), ProductValue::UInt(30)]),
                    ProductValue::array([ProductValue::Int(2), ProductValue::UInt(10)]),
                ]),
                ProductValue::UInt(0),
            ]),
            Ceilings::for_boundary(Boundary::ActorState),
        )
        .expect("encoding");
        assert!(
            matches!(
                actor.restore(ActorState::new(WINDOWED_REDUCE_STATE_SCHEMA, backwards)),
                Err(ActorRestoreError::StateInvariantViolated { .. })
            ),
            "a sequence whose recorded times go backwards is not this element's state"
        );
    }

    #[test]
    fn the_first_sample_arms_and_later_samples_do_not_rearm() {
        let mut actor = new_actor(1_000, 250);
        let first = sample(&mut actor, 1.0, 10);
        let (correlation, after) = scheduled(&first);
        assert_eq!(
            after.get().get(),
            250,
            "the scheduled interval is the config's emission period"
        );
        let second = sample(&mut actor, 2.0, 20);
        assert!(
            second.as_slice().is_empty(),
            "a sample while armed adds no schedule: {second:?}"
        );
        assert_eq!(correlation, 1);
    }

    #[test]
    fn the_reservation_is_released_when_the_buffer_empties() {
        let mut actor = new_actor(10, 100);
        sample(&mut actor, 5.0, 10);
        let fired = fire(&mut actor, 1, 200);
        assert_eq!(
            armed(&fired),
            None,
            "without a sample it does not re-arm: {fired:?}"
        );
        let rearmed = sample(&mut actor, 6.0, 210);
        assert_eq!(scheduled(&rearmed).0, 2);
    }

    #[test]
    fn a_stale_or_shapeless_fire_is_a_declared_suppression() {
        let mut actor = new_actor(1_000, 100);
        sample(&mut actor, 1.0, 10);
        assert_eq!(
            suppressed(&fire(&mut actor, 99, 110)).as_deref(),
            Some(crate::STALE_TIMER_FIRE)
        );
        let shapeless = drive(
            &mut actor,
            &arrival(TIMER_PORT_NAME, ProductValue::Null, Some(110)),
        );
        assert_eq!(
            suppressed(&shapeless).as_deref(),
            Some(crate::STALE_TIMER_FIRE)
        );
    }

    #[test]
    fn unknown_inlets_and_non_numeric_arrivals_are_not_sampled() {
        let mut actor = new_actor(1_000, 100);
        let unknown = drive(
            &mut actor,
            &arrival("event", ProductValue::Int(1), Some(10)),
        );
        assert!(unknown.as_slice().is_empty());
        let text = drive(
            &mut actor,
            &arrival(SAMPLE_INLET, ProductValue::String("x".into()), Some(10)),
        );
        assert!(text.as_slice().is_empty());
        let untimed = drive(
            &mut actor,
            &arrival(SAMPLE_INLET, ProductValue::Int(1), None),
        );
        assert!(untimed.as_slice().is_empty());
        assert!(actor.samples().is_empty(), "none of the three is a sample");
    }

    #[test]
    fn every_config_edit_replaces_the_incarnation() {
        let mut actor = new_actor(1_000, 100);
        assert_eq!(
            actor.on_config_change(&config_of(
                2_000,
                100,
                "acc + sample",
                ProductValue::float(0.0)
            )),
            ConfigChangeOutcome::ReplaceIncarnation
        );
        assert_eq!(
            <WindowedReduceFactory<Types> as EmittingActorFactory<ProductPayload>>::DISPOSITION,
            EditDisposition::Restarts
        );
        assert!(
            <WindowedReduceFactory<Types> as EmittingActorFactory<ProductPayload>>::CHECKPOINTS
        );
    }

    #[test]
    fn each_config_rejection_names_its_own_slot() {
        let reject = |config: ProductValue| {
            WindowedReduceFactory::<Types>::create(
                &FoldedConfig::minted(ActorType::WindowedReduce, config),
                &(),
            )
            .err()
            .expect("refusal")
        };
        assert_eq!(
            reject(ProductValue::Int(1)),
            WindowedReduceFactoryError::Config(crate::config::ConfigRejection::NotObject(
                circular_core::NotObject {
                    at: circular_core::FieldPath::root(),
                    actual: circular_core::ValueKind::Int,
                }
            ))
        );
        let with = |extra: &[(&str, ProductValue)]| {
            let mut entries = vec![
                ("window_length".to_owned(), ProductValue::Int(1_000)),
                ("emission_period".to_owned(), ProductValue::Int(100)),
                (
                    "reduce".to_owned(),
                    ProductValue::String("acc + sample".to_owned()),
                ),
                ("seed".to_owned(), ProductValue::float(0.0)),
            ];
            for (key, value) in extra {
                entries.retain(|(existing, _)| existing != key);
                entries.push(((*key).to_owned(), value.clone()));
            }
            ProductValue::object(entries).expect("field names are unique")
        };
        assert_eq!(
            reject(with(&[("unknown", ProductValue::Int(0))])),
            WindowedReduceFactoryError::Config(crate::config::ConfigRejection::Unknown(
                circular_core::UnknownField {
                    at: circular_core::FieldPath::root(),
                    key: "unknown".to_owned(),
                }
            ))
        );
        assert_eq!(
            reject(with(&[("emission_period", ProductValue::Int(0))])),
            WindowedReduceFactoryError::Config(crate::config::ConfigRejection::Slot {
                slot: "emission_period",
                error: crate::config::ConfigDecodeError::ZeroInterval,
            }),
            "0 is not a period"
        );
        assert!(
            matches!(
                reject(with(&[(
                    "reduce",
                    ProductValue::String("stranger + sample".to_owned())
                )])),
                WindowedReduceFactoryError::Reduce(_)
            ),
            "an undeclared name is a config refusal"
        );
    }

    #[test]
    fn recorded_arrivals_emit_each_window_sum_and_skip_the_empty_window() {
        let script: [(bool, f64, u64); 7] = [
            (true, 1.0, 10),
            (true, 2.0, 20),
            (false, 0.0, 100),
            (true, 3.0, 110),
            (true, 4.0, 150),
            (false, 0.0, 200),
            (false, 0.0, 300),
        ];
        let mut actor = new_actor(100, 100);
        let mut correlation = 0;
        let mut emitted = Vec::new();
        for (is_sample, value, at) in script {
            let effects = if is_sample {
                sample(&mut actor, value, at)
            } else {
                fire(&mut actor, correlation, at)
            };
            if let Some((next, _)) = armed(&effects) {
                correlation = next;
            }
            emitted.push(aggregate(&effects));
        }
        assert_eq!(
            emitted,
            [
                None,
                None,
                Some(ProductValue::float(3.0)),
                None,
                None,
                Some(ProductValue::float(7.0)),
                None
            ],
        );
    }
}
