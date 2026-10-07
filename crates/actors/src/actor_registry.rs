
use crate::actor_support::StampedEvent;
use crate::assemble::AssembleFactory;
use crate::bang::EmptyConfigFactoryError;
use crate::boundary_actor::{FormFactory, InputFactory, OutputFactory};
use crate::counter::{CounterActor, CounterFactory};
use crate::debounce::DebounceFactory;
use crate::ema_state::EmaFactory;
use crate::join::JoinFactory;
use crate::listener_actor::ListenerFactory;
use crate::match_actor::{MatchActor, MatchFactory};
use crate::peer_actor::PeerFactory;
use crate::replicator_actor::ReplicatorFactory;
use crate::route_actor::{RouteActor, RouteFactory};
use crate::route_config::RouteConfigError;
use crate::tap::{TapActor, TapFactory};
use crate::windowed_reduce::WindowedReduceFactory;
use crate::{ActorSpec, ActorType, Name, get};
use crate::{
    AgentFactory, AlertFactory, FileFactory, JsonFactory, NotifyFactory, RequestFactory,
    TimerFactory, ToolExecutorFactory, fixture_panic::FixturePanicFactory,
    keyed_reduce::KeyedReduceFactory,
};
use circular_core::Payload;
pub use circular_core::Value as ProductValue;
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, EffectOutcome, EmittingActor, EmittingActorFactory,
    FoldedConfig,
};
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

pub type ProductPayload = Payload<Name, ProductValue>;

#[must_use]
pub fn is_entry_actor(actor_type: ActorType) -> bool {
    let registration = crate::registration(actor_type);
    let spec = registration.spec();
    spec.boundary().is_some_and(|boundary| {
        boundary.direction() == circular_protocol::boundary_port::BoundaryPortDirection::Inlet
    }) || spec.is_source()
        || spec.effect().stand_ins().is_some_and(|effects| {
            effects
                .iter()
                .any(|(constructor, _)| constructor.is_external_entry())
        })
        || registration.issues_schedule()
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct EditabilityRun;

impl circular_core::StreamIdentity for EditabilityRun {}

struct EditabilityTypes;

impl ActorTypes for EditabilityTypes {
    type Stream = EditabilityRun;
    type Event = ProductPayload;
    type Payload = ProductPayload;
    type EffectId = circular_runtime::EffectId;
    type StateVersion = u16;
    type Observation = ();
    type Grants = ();

    fn payload(event: &Self::Event) -> &Self::Payload {
        event
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProductFactoryError {
    EmptyConfig(EmptyConfigFactoryError),
    Route(RouteConfigError),
    Ema(crate::ema_state::EmaFactoryError),
    Debounce(crate::debounce::DebounceFactoryError),
    KeyedReduce(crate::keyed_reduce::KeyedReduceConfigError),
    WindowedReduce(crate::windowed_reduce::WindowedReduceFactoryError),
    Join(crate::join::JoinConfigError),
    Agent(crate::agent_actor::AgentFactoryError),
    ToolExecutor(crate::tool_executor_actor::ToolExecutorFactoryError),
    File(crate::file_actor::FileFactoryError),
    Request(crate::request_actor::RequestFactoryError),
    Peer(crate::peer_actor::PeerFactoryError),
    Notify(crate::notify_actor::NotifyFactoryError),
    Timer(crate::timer_actor::TimerFactoryError),
    Json(crate::json_actor::JsonFactoryError),
    Listener(crate::listener::ListenerConfigError),
    Alert(crate::alert_actor::AlertFactoryError),
    Assemble(crate::assemble::AssembleConfigError),
    Replicator(crate::replicator_actor::ReplicatorFactoryError),
    FoldMismatch(circular_runtime::FoldedConfigMismatch),
    FixturePanic(crate::fixture_panic::FixturePanicConfigError),
    Otlp(crate::otlp::OtlpConfigError),
}

impl From<std::convert::Infallible> for ProductFactoryError {
    fn from(never: std::convert::Infallible) -> Self {
        match never {}
    }
}

macro_rules! product_factory_error_rows {
    (@display $formatter:ident, $error:ident, bare) => {
        $error.fmt($formatter)
    };
    (@display $formatter:ident, $error:ident, $label:literal) => {
        write!($formatter, concat!($label, ": {}"), $error)
    };
    ($(($variant:ident, $inner:ty, $($display:tt)+)),+ $(,)?) => {
        $(
            impl From<$inner> for ProductFactoryError {
                fn from(error: $inner) -> Self {
                    Self::$variant(error)
                }
            }
        )+

        impl fmt::Display for ProductFactoryError {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(
                        Self::$variant(error) => product_factory_error_rows!(
                            @display formatter, error, $($display)+
                        ),
                    )+
                }
            }
        }
    };
}

product_factory_error_rows! {
    (Join, crate::join::JoinConfigError, "failed to read join config"),
    (Otlp, crate::otlp::OtlpConfigError, "ConfigRejected"),
    (EmptyConfig, EmptyConfigFactoryError, bare),
    (Route, RouteConfigError, "failed to read route config"),
    (Ema, crate::ema_state::EmaFactoryError, "failed to read ema config"),
    (
        Debounce,
        crate::debounce::DebounceFactoryError,
        "failed to read debounce config"
    ),
    (
        KeyedReduce,
        crate::keyed_reduce::KeyedReduceConfigError,
        "failed to read keyed_reduce config"
    ),
    (
        WindowedReduce,
        crate::windowed_reduce::WindowedReduceFactoryError,
        "failed to read windowed_reduce config"
    ),
    (Agent, crate::agent_actor::AgentFactoryError, "agent failed to start"),
    (
        ToolExecutor,
        crate::tool_executor_actor::ToolExecutorFactoryError,
        "tool_executor failed to start"
    ),
    (File, crate::file_actor::FileFactoryError, "file failed to start"),
    (Request, crate::request_actor::RequestFactoryError, "request failed to start"),
    (Peer, crate::peer_actor::PeerFactoryError, "peer failed to start"),
    (Notify, crate::notify_actor::NotifyFactoryError, "notify failed to start"),
    (Timer, crate::timer_actor::TimerFactoryError, "timer failed to start"),
    (Json, crate::json_actor::JsonFactoryError, "json failed to start"),
    (Listener, crate::listener::ListenerConfigError, "listener failed to start"),
    (Alert, crate::alert_actor::AlertFactoryError, "alert failed to start"),
    (Assemble, crate::assemble::AssembleConfigError, "assemble failed to start"),
    (Replicator, crate::replicator_actor::ReplicatorFactoryError, "replicator failed to start"),
    (FoldMismatch, circular_runtime::FoldedConfigMismatch, bare),
    (
        FixturePanic,
        crate::fixture_panic::FixturePanicConfigError,
        "fixture_panic could not be built"
    ),
}

impl Error for ProductFactoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::EmptyConfig(error) => Some(error),
            Self::FoldMismatch(error) => Some(error),
            Self::Otlp(error) => Some(error),
            Self::Route(_)
            | Self::Ema(_)
            | Self::Debounce(_)
            | Self::Join(_)
            | Self::KeyedReduce(_)
            | Self::WindowedReduce(_)
            | Self::Replicator(_)
            | Self::Agent(_)
            | Self::ToolExecutor(_)
            | Self::File(_)
            | Self::Request(_)
            | Self::Peer(_)
            | Self::Notify(_)
            | Self::Timer(_)
            | Self::Json(_)
            | Self::Listener(_)
            | Self::Alert(_)
            | Self::Assemble(_)
            | Self::FixturePanic(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProductObservation {
    KeyedReduce(crate::keyed_reduce::KeyedReduceTally),
}

impl ProductObservation {
    #[must_use]
    pub fn slots(self) -> Box<[circular_core::Value]> {
        match self {
            Self::KeyedReduce(tally) => Box::new([
                circular_core::Value::Float(tally.total()),
                circular_core::Value::int(tally.keys()),
            ]),
        }
    }
}

pub trait ProductAuthorityBearer:
    circular_runtime::InstanceAuthorityBearer
    + circular_runtime::AgentHarnessAuthorityBearer
    + circular_runtime::FilesystemAuthorityBearer
    + circular_runtime::HttpFetchAuthorityBearer
    + circular_runtime::PeerAuthorityBearer
    + circular_runtime::ProcessAuthorityBearer
    + circular_runtime::UserNotifyAuthorityBearer
{
}

impl<G> ProductAuthorityBearer for G where
    G: circular_runtime::InstanceAuthorityBearer
        + circular_runtime::AgentHarnessAuthorityBearer
        + circular_runtime::FilesystemAuthorityBearer
        + circular_runtime::HttpFetchAuthorityBearer
        + circular_runtime::PeerAuthorityBearer
        + circular_runtime::ProcessAuthorityBearer
        + circular_runtime::UserNotifyAuthorityBearer
{
}

macro_rules! product_actor_table {
    (
        published: [ $( $(#[$pmeta:meta])* $pvariant:ident => {
            actor: $pnode:ident, actor: $pactor:ty, factory: $pfactory:ident,
            $(judge: $pjudge:path,)?
            boarded_without: $pwithout:expr, reason: $preason:literal $(,)?
        } ),+ $(,)? ],
        fixture: [ $( $(#[$fmeta:meta])* $fvariant:ident => {
            actor: $fnode:ident, actor: $factor:ty, factory: $ffactory:ident,
            $(judge: $fjudge:path,)?
            boarded_without: $fwithout:expr, reason: $freason:literal $(,)?
        } ),+ $(,)? ] $(,)?
    ) => {
        pub(crate) const FACTORY_EDITABILITY: [crate::editability::Editability;
            [$(stringify!($pvariant),)+ $(stringify!($fvariant),)+].len()
        ] = [
            $(crate::editability::Editability::from_disposition(
                <$pfactory<EditabilityTypes> as EmittingActorFactory<ProductPayload>>::TYPE,
                concat!(stringify!($pvariant), "Actor"),
                <$pfactory<EditabilityTypes> as EmittingActorFactory<ProductPayload>>::DISPOSITION,
                <$pfactory<EditabilityTypes> as EmittingActorFactory<ProductPayload>>::CHECKPOINTS,
                true,
                $pwithout,
                $preason,
            ),)+
            $(crate::editability::Editability::from_disposition(
                <$ffactory<EditabilityTypes> as EmittingActorFactory<ProductPayload>>::TYPE,
                concat!(stringify!($fvariant), "Actor"),
                <$ffactory<EditabilityTypes> as EmittingActorFactory<ProductPayload>>::DISPOSITION,
                <$ffactory<EditabilityTypes> as EmittingActorFactory<ProductPayload>>::CHECKPOINTS,
                false,
                $fwithout,
                $freason,
            ),)+
        ];

        pub enum ProductActor<T>
        where
            T: ActorTypes,
            T::Grants: circular_runtime::InstanceAuthorityBearer,
        {
            Source(crate::otlp::OtlpSource<T::StateVersion, T::EffectId>),
            $( $(#[$pmeta])* $pvariant($pactor), )+
            $( $(#[$fmeta])* $fvariant($factor), )+
        }

        impl<T> EditableActor for ProductActor<T>
        where
            T: ActorTypes,
            T::StateVersion: From<u16> + PartialEq,
            T::Grants: circular_runtime::InstanceAuthorityBearer,
        {
            type StateVersion = T::StateVersion;
            type EffectId = T::EffectId;

            fn on_instance_disposition(&mut self, intent: &circular_runtime::InstanceIntent,
                disposition: circular_runtime::InstanceDisposition) -> Option<circular_runtime::ScheduleSpec> {
                match self {
                    Self::Source(actor) => actor.on_instance_disposition(intent, disposition),
                    $( Self::$pvariant(actor) => actor.on_instance_disposition(intent, disposition), )+
                    $( Self::$fvariant(actor) => actor.on_instance_disposition(intent, disposition), )+
                }
            }

            fn instance_expiry(&self, intent: &circular_runtime::InstanceIntent) -> Option<circular_runtime::ScheduleSpec> {
                match self {
                    Self::Source(actor) => actor.instance_expiry(intent),
                    $( Self::$pvariant(actor) => actor.instance_expiry(intent), )+
                    $( Self::$fvariant(actor) => actor.instance_expiry(intent), )+
                }
            }

            fn instance_retirements(&self) -> Vec<circular_runtime::InstanceIntent> {
                match self {
                    Self::Source(actor) => actor.instance_retirements(),
                    $( Self::$pvariant(actor) => actor.instance_retirements(), )+
                    $( Self::$fvariant(actor) => actor.instance_retirements(), )+
                }
            }

            fn on_config_change(&mut self, config: &FoldedConfig) -> ConfigChangeOutcome {
                match self {
                    Self::Source(actor) => actor.on_config_change(config),
                    $( Self::$pvariant(actor) => actor.on_config_change(config), )+
                    $( Self::$fvariant(actor) => actor.on_config_change(config), )+
                }
            }

            fn checkpoint(&self) -> Option<ActorState<Self::StateVersion>> {
                match self {
                    Self::Source(actor) => actor.checkpoint(),
                    $( Self::$pvariant(actor) => actor.checkpoint(), )+
                    $( Self::$fvariant(actor) => actor.checkpoint(), )+
                }
            }

            fn try_checkpoint(&self) -> Result<Option<ActorState<Self::StateVersion>>, circular_core::CodecError> {
                match self {
                    Self::Source(actor) => actor.try_checkpoint(),
                    $( Self::$pvariant(actor) => actor.try_checkpoint(), )+
                    $( Self::$fvariant(actor) => actor.try_checkpoint(), )+
                }
            }

            fn stateless(&self) -> bool {
                match self {
                    Self::Source(actor) => actor.stateless(),
                    $( Self::$pvariant(actor) => actor.stateless(), )+
                    $( Self::$fvariant(actor) => actor.stateless(), )+
                }
            }

            fn restore(
                &mut self,
                state: ActorState<Self::StateVersion>,
            ) -> Result<(), ActorRestoreError<Self::StateVersion>> {
                match self {
                    Self::Source(actor) => actor.restore(state),
                    $( Self::$pvariant(actor) => actor.restore(state), )+
                    $( Self::$fvariant(actor) => actor.restore(state), )+
                }
            }
        }

        impl<T> EmittingActor<T, ProductPayload> for ProductActor<T>
        where
            T: ActorTypes<
                    Payload = ProductPayload,
                    Observation = ProductObservation,
                >,
            T::Event: StampedEvent,
            T::StateVersion: From<u16> + PartialEq,
            T::Grants: ProductAuthorityBearer,
        {
            fn observe(&self) -> Option<T::Observation> {
                product_observation(self)
            }

            fn on_event(
                &mut self,
                input: &ActorInput<T::Event>,
                context: &ActorContext<'_, T::Stream, T::Grants>,
            ) -> ActorEffects<ProductPayload> {
                match self {
                    Self::Source(actor) => <crate::otlp::OtlpSource<T::StateVersion, T::EffectId> as EmittingActor<T, ProductPayload>>::on_event(actor, input, context),
                    $( Self::$pvariant(actor) => <$pactor as EmittingActor<T, ProductPayload>>
                        ::on_event(actor, input, context), )+
                    $( Self::$fvariant(actor) => <$factor as EmittingActor<T, ProductPayload>>
                        ::on_event(actor, input, context), )+
                }
            }

            fn on_outcome(
                &mut self,
                outcome: &EffectOutcome<T::EffectId>,
                context: &ActorContext<'_, T::Stream, T::Grants>,
            ) -> ActorEffects<ProductPayload> {
                match self {
                    Self::Source(actor) => <crate::otlp::OtlpSource<T::StateVersion, T::EffectId> as EmittingActor<T, ProductPayload>>::on_outcome(actor, outcome, context),
                    $( Self::$pvariant(actor) => <$pactor as EmittingActor<T, ProductPayload>>
                        ::on_outcome(actor, outcome, context), )+
                    $( Self::$fvariant(actor) => <$factor as EmittingActor<T, ProductPayload>>
                        ::on_outcome(actor, outcome, context), )+
                }
            }

            fn on_lifecycle(
                &mut self,
                life: circular_runtime::ActorLifecycle,
                context: &ActorContext<'_, T::Stream, T::Grants>,
            ) -> ActorEffects<ProductPayload> {
                match self {
                    Self::Source(actor) => <crate::otlp::OtlpSource<T::StateVersion, T::EffectId> as EmittingActor<T, ProductPayload>>::on_lifecycle(actor, life, context),
                    $( Self::$pvariant(actor) => <$pactor as EmittingActor<T, ProductPayload>>
                        ::on_lifecycle(actor, life, context), )+
                    $( Self::$fvariant(actor) => <$factor as EmittingActor<T, ProductPayload>>
                        ::on_lifecycle(actor, life, context), )+
                }
            }

            fn accepts(&self, inlet: &circular_core::PortId) -> bool {
                match self {
                    Self::Source(actor) => <crate::otlp::OtlpSource<T::StateVersion, T::EffectId> as EmittingActor<T, ProductPayload>>::accepts(actor, inlet),
                    $( Self::$pvariant(actor) => <$pactor as EmittingActor<T, ProductPayload>>
                        ::accepts(actor, inlet), )+
                    $( Self::$fvariant(actor) => <$factor as EmittingActor<T, ProductPayload>>
                        ::accepts(actor, inlet), )+
                }
            }
        }

        #[derive(Clone, Copy)]
        enum ProductFactoryKind {
            $( $pvariant, )+
            $( $fvariant, )+
            Source,
        }

        impl<T> ProductActorFactory<T>
        where
            T: ActorTypes<Payload = ProductPayload>,
            T::Event: StampedEvent,
            T::StateVersion: From<u16> + PartialEq,
            T::Grants: ProductAuthorityBearer,
            circular_runtime::InstanceAuthority<
                <T::Grants as circular_runtime::InstanceAuthorityBearer>::Seal,
            >: Clone,
        {
            #[must_use]
            pub const fn actor_type(&self) -> ActorType {
                match self.kind {
                    ProductFactoryKind::Source => ActorType::Otlp,
                    $( ProductFactoryKind::$pvariant => ActorType::$pnode, )+
                    $( ProductFactoryKind::$fvariant => ActorType::$fnode, )+
                }
            }

            #[must_use]
            pub fn spec(&self) -> &'static ActorSpec {
                get(self.actor_type())
            }

            pub fn create(
                &self,
                config: &circular_runtime::FoldedConfig,
                grants: &T::Grants,
                inlets: &crate::inlet_shapes::ResolvedInletShapes,
            ) -> Result<ProductActor<T>, ProductFactoryError> {
                judge_activation_config(self.actor_type(), config, inlets)?;
                match self.kind {
                    ProductFactoryKind::Source => crate::otlp::OtlpSource::create(config)
                        .map(ProductActor::Source)
                        .map_err(ProductFactoryError::Otlp),
                    $( ProductFactoryKind::$pvariant => {
                        <$pfactory<T> as EmittingActorFactory<ProductPayload>>::create(config, grants)
                            .map(ProductActor::$pvariant)
                            .map_err(ProductFactoryError::from)
                    } )+
                    $( ProductFactoryKind::$fvariant => {
                        <$ffactory<T> as EmittingActorFactory<ProductPayload>>::create(config, grants)
                            .map(ProductActor::$fvariant)
                            .map_err(ProductFactoryError::from)
                    } )+
                }
            }
        }

        pub fn judge_activation_config(
            actor_type: ActorType,
            config: &FoldedConfig,
            inlets: &crate::inlet_shapes::ResolvedInletShapes,
        ) -> Result<(), ProductFactoryError> {
            config.for_type(actor_type)?;
            match actor_type {
                $( $( ActorType::$pnode => $pjudge(config, inlets).map_err(ProductFactoryError::from), )? )+
                $( $( ActorType::$fnode => $fjudge(config, inlets).map_err(ProductFactoryError::from), )? )+
                ActorType::Otlp => crate::otlp::judge(config).map_err(ProductFactoryError::Otlp),
                _ => Ok(()),
            }
        }

        #[must_use]
        pub fn product_actor_factory<T>(actor_type: ActorType) -> Option<ProductActorFactory<T>>
        where
            T: ActorTypes,
        {
            match actor_type {
                ActorType::Otlp => Some(ProductActorFactory {
                    kind: ProductFactoryKind::Source,
                    marker: PhantomData,
                }),
                $( ActorType::$pnode => Some(ProductActorFactory {
                    kind: ProductFactoryKind::$pvariant,
                    marker: PhantomData,
                }), )+
                _ => None,
            }
        }

        #[must_use]
        pub fn fixture_actor_factory<T>(actor_type: ActorType) -> Option<ProductActorFactory<T>>
        where
            T: ActorTypes,
        {
            match actor_type {
                $( ActorType::$fnode => Some(ProductActorFactory {
                    kind: ProductFactoryKind::$fvariant,
                    marker: PhantomData,
                }), )+
                other => product_actor_factory(other),
            }
        }
    };
}

product_actor_table! {
    published: [
        Join => {
            actor: Join,
            actor: crate::join::JoinActor<T::StateVersion, T::EffectId>,
            factory: JoinFactory,
            boarded_without: None,
            reason: "changing the key path replaces only the join incarnation",
        },
        Assemble => {
            actor: Assemble,
            actor: crate::assemble::AssembleActor<T::StateVersion, T::EffectId>,
            factory: AssembleFactory,
            judge: crate::assemble::judge,
            boarded_without: Some("checkpoint=None; reconsumption and the checkpoint implementation are separate"),
            reason: "configuration changes replace the window owner; uncompleted derived windows emit nothing on retirement",
        },
        Match => {
            actor: Match,
            actor: MatchActor<T::StateVersion, T::EffectId>,
            factory: MatchFactory,
            boarded_without: None,
            reason: "Stateless envelope branching has no configuration to restart",
        },
        Tap => {
            actor: Tap,
            actor: TapActor<T::StateVersion, T::EffectId>,
            factory: TapFactory,
            boarded_without: None,
            reason: "data-invariant pass-through: config does not decide the output, so absorbing a change does not change what is observed",
        },
        Input => {
            actor: Input,
            actor: crate::boundary_actor::BoundaryActor<T::StateVersion, T::EffectId>,
            factory: InputFactory,
            boarded_without: None,
            reason: "label names the boundary for people; the port identity is the actor key's, so a label edit is absorbed",
        },
        Output => {
            actor: Output,
            actor: crate::boundary_actor::BoundaryActor<T::StateVersion, T::EffectId>,
            factory: OutputFactory,
            boarded_without: None,
            reason: "label names the boundary for people; the port identity is the actor key's, so a label edit is absorbed",
        },
        Form => {
            actor: Form,
            actor: crate::boundary_actor::BoundaryActor<T::StateVersion, T::EffectId>,
            factory: FormFactory,
            boarded_without: None,
            reason: "fields type what may be injected; the pass-through does not read them, so a field edit is absorbed",
        },
        Counter => {
            actor: Counter,
            actor: CounterActor<T::StateVersion, T::EffectId>,
            factory: CounterFactory,
            boarded_without: None,
            reason: "there is no config to change, so a restart only discards the accumulation",
        },
        Route => {
            actor: Route,
            actor: RouteActor<T::StateVersion, T::EffectId>,
            factory: RouteFactory,
            boarded_without: None,
            reason: "editing the cases changes the port set and the selection function together; absorbing would select an outlet that no longer exists",
        },
        Ema => {
            actor: Ema,
            actor: crate::ema_state::EmaActor<T::StateVersion, T::EffectId>,
            factory: EmaFactory,
            boarded_without: None,
            reason: "changing half_life changes what the recurrence means, so a new incarnation",
        },
        Debounce => {
            actor: Debounce,
            actor: crate::debounce::DebounceActor<T::StateVersion, T::EffectId>,
            factory: DebounceFactory,
            boarded_without: None,
            reason: "with nothing pending, a quiet-window edit is absorbed; while pending, this hook cannot re-reserve, so an expiry measured against the old window would read as the new one and the actor restarts",
        },
        WindowedReduce => {
            actor: WindowedReduce,
            actor: crate::windowed_reduce::WindowedReduceActor<T::StateVersion, T::EffectId>,
            factory: WindowedReduceFactory,
            judge: crate::windowed_reduce::judge,
            boarded_without: None,
            reason: "window and period change which sample fell in which window, and reduce and seed change what the folded value was a fold of; there is nothing to absorb",
        },
        KeyedReduce => {
            actor: KeyedReduce,
            actor: crate::keyed_reduce::KeyedReduceActor<T::StateVersion, T::EffectId>,
            factory: KeyedReduceFactory,
            boarded_without: None,
            reason: "changing either path kills what the accumulation was a sum of; there is nothing to absorb",
        },
        Replicator => {
            actor: Replicator,
            actor: crate::replicator_actor::ReplicatorActor<
                <T::Grants as circular_runtime::InstanceAuthorityBearer>::Seal,
                T::StateVersion,
                T::EffectId,
            >,
            factory: ReplicatorFactory,
            boarded_without: None,
            reason: "changing at changes what the key means, so a new incarnation; ttl and capacity are absorbed",
        },
        Agent => {
            actor: Agent,
            actor: crate::agent_actor::AgentActor<T::StateVersion, T::EffectId>,
            factory: AgentFactory,
            boarded_without: Some(
                "provider transport: the state checkpoint and the concrete harness adapter are a later vertical slice",
            ),
            reason: "changing harness, queue or tool changes what the active session and single-flight mean, so a new incarnation is needed",
        },
        ToolExecutor => {
            actor: ToolExecutor,
            actor: crate::tool_executor_actor::ToolExecutorActor<T::StateVersion, T::EffectId>,
            factory: ToolExecutorFactory,
            judge: crate::tool_executor_actor::judge,
            boarded_without: None,
            reason: "changing the allowlist or the effect template changes the external effect the same tool name produces, so the actor restarts",
        },
        Listener => {
            actor: Listener,
            actor: crate::listener_actor::ListenerActor<T::StateVersion, T::EffectId>,
            factory: ListenerFactory,
            judge: crate::listener_actor::judge,
            boarded_without: None,
            reason: "changing the tail path or the policy means a new durable effect and a new admission",
        },
        File => {
            actor: File,
            actor: crate::file_actor::FileActor<T::StateVersion, T::EffectId>,
            factory: FileFactory,
            judge: crate::file_actor::judge,
            boarded_without: None,
            reason: "changing path means a different file, so a new incarnation",
        },
        Peer => {
            actor: Peer,
            actor: crate::peer_actor::PeerActor<T::StateVersion, T::EffectId>,
            factory: PeerFactory,
            judge: crate::peer_actor::judge,
            boarded_without: None,
            reason: "changing adapter, realm, name, policy or capacity changes the binding, so a new incarnation",
        },
        Request => {
            actor: Request,
            actor: crate::request_actor::RequestActor<T::StateVersion, T::EffectId>,
            factory: RequestFactory,
            judge: crate::request_actor::judge,
            boarded_without: None,
            reason: "the method, url and headers config changes the external request an event is projected into, so the actor restarts",
        },
        Notify => {
            actor: Notify,
            actor: crate::notify_actor::NotifyActor<T::StateVersion, T::EffectId>,
            factory: NotifyFactory,
            judge: crate::notify_actor::judge,
            boarded_without: Some(
                "cooldown policy: during_interval accepts suppress, latest and queue, and minimum_interval=0 means no cooling",
            ),
            reason: "changing the channel or the notification disposition policy changes the external recipient and what submission means, so the actor restarts",
        },
        Alert => {
            actor: Alert,
            actor: crate::alert_actor::AlertActor<T::StateVersion, T::EffectId>,
            factory: AlertFactory,
            judge: crate::alert_actor::judge,
            boarded_without: Some("restore on restart is not wired yet"),
            reason: "changing the predicate or the interval changes what sampling and reservation mean, so a new incarnation",
        },
        Json => {
            actor: Json,
            actor: crate::json_actor::JsonActor<T::StateVersion, T::EffectId>,
            factory: JsonFactory,
            boarded_without: None,
            reason: "a readable initial becomes the current value in place, so the edit keeps the incarnation",
        },
        Timer => {
            actor: Timer,
            actor: crate::timer_actor::TimerActor<T::StateVersion, T::EffectId>,
            factory: TimerFactory,
            boarded_without: None,
            reason: "changing every changes what the next relative delay means; the generation and sequence are checkpointed and rearmed by the daemon bang in the new incarnation",
        },
    ],
    fixture: [
        FixturePanic => {
            actor: FixturePanic,
            actor: crate::fixture_panic::FixturePanicActor<T::StateVersion, T::EffectId>,
            factory: FixturePanicFactory,
            boarded_without: None,
            reason: "fixture-local deliberate failure: moving the position while keeping the arrivals already counted would spread the intent across two revisions",
        },
    ],
}

fn product_observation<T>(actor: &ProductActor<T>) -> Option<T::Observation>
where
    T: ActorTypes<Observation = ProductObservation>,
    T::Grants: circular_runtime::InstanceAuthorityBearer,
{
    match actor {
        ProductActor::KeyedReduce(actor) => Some(ProductObservation::KeyedReduce(actor.tally())),
        _ => None,
    }
}

pub struct ProductActorFactory<T> {
    kind: ProductFactoryKind,
    marker: PhantomData<fn() -> T>,
}

#[cfg(test)]
mod tests {
    use super::*;

    use circular_testkit::types::TestRun;

    struct TestTypes;

    impl ActorTypes for TestTypes {
        type Stream = TestRun;
        type Event = ProductPayload;
        type Payload = ProductPayload;
        type EffectId = circular_runtime::EffectId;
        type StateVersion = u16;
        type Observation = ();
        type Grants = ();

        fn payload(event: &Self::Event) -> &Self::Payload {
            event
        }
    }

    struct StampedTypes;

    impl ActorTypes for StampedTypes {
        type Stream = TestRun;
        type Event = circular_core::Event<
            TestRun,
            circular_runtime::ActorId,
            ProductPayload,
            std::convert::Infallible,
        >;
        type Payload = ProductPayload;
        type EffectId = circular_runtime::EffectId;
        type StateVersion = u16;
        type Observation = ProductObservation;
        type Grants = ();

        fn payload(event: &Self::Event) -> &Self::Payload {
            event.payload()
        }
    }

    #[test]
    fn registered_inlet_admission_keeps_authoring_and_activation_distinct() {
        let config = ProductValue::object([
            ("predicate", ProductValue::string("event > 1.0")),
            ("firing_delay", ProductValue::Int(10)),
            ("recovery_delay", ProductValue::Int(20)),
        ])
        .unwrap();
        let admitted = crate::admit_registered_create(ActorType::Alert, &config).unwrap();
        assert_eq!(admitted.folded_config().actor_type(), ActorType::Alert);
        assert_eq!(admitted.folded_config().value(), &config);
        let factory = product_actor_factory::<TestTypes>(ActorType::Alert).unwrap();
        let inlets = |shape| {
            crate::ResolvedInletShapes::new([(
                circular_core::PortId::try_new("event").unwrap(),
                crate::GroundShape::try_new(shape).unwrap(),
            )])
        };
        assert_eq!(
            factory
                .create(
                    admitted.folded_config(),
                    &(),
                    &inlets(crate::Shape::Base(crate::BaseShape::Int)),
                )
                .map(|_| ()),
            Err(ProductFactoryError::Alert(
                crate::AlertFactoryError::PredicateKindSplit(circular_expr::shapes::KindSplit {
                    operator: "_>_",
                    left: crate::BaseShape::Int,
                    right: crate::BaseShape::Float,
                }),
            ))
        );
        for known in [
            crate::ResolvedInletShapes::default(),
            inlets(crate::Shape::Any),
            inlets(crate::Shape::Base(crate::BaseShape::Float)),
        ] {
            assert!(
                factory
                    .create(admitted.folded_config(), &(), &known)
                    .is_ok()
            );
        }
    }

    #[test]
    fn admitted_dynamic_ports_and_factory_consume_the_same_folded_config() {
        let config = ProductValue::object([
            ("at", ProductValue::array([ProductValue::string("status")])),
            (
                "cases",
                ProductValue::object([
                    ("accepted", ProductValue::string("ok")),
                    ("rejected", ProductValue::string("no")),
                ])
                .unwrap(),
            ),
        ])
        .unwrap();
        let admitted = crate::admit_registered_create(ActorType::Route, &config).unwrap();
        assert_eq!(admitted.config(), &config);
        assert_eq!(admitted.ports().inlets().len(), 1);
        assert_eq!(admitted.ports().inlets()[0].id().as_str(), "event");
        let expected_flow = crate::Flow::Stream(crate::Shape::Var(Name::from_static("T")));
        assert_eq!(admitted.ports().inlets()[0].ty(), &expected_flow);
        assert_eq!(
            admitted
                .ports()
                .outlets()
                .iter()
                .map(|port| port.id().as_str())
                .collect::<Vec<_>>(),
            ["unmatched", "route_accepted", "route_rejected"],
        );
        for port in admitted.ports().outlets() {
            assert_eq!(port.ty(), &expected_flow);
        }
        let ProductActor::Route(actor) = product_actor_factory::<TestTypes>(ActorType::Route)
            .unwrap()
            .create(
                admitted.folded_config(),
                &(),
                &crate::ResolvedInletShapes::default(),
            )
            .unwrap()
        else {
            panic!("route factory arm");
        };
        for (status, expected) in [
            ("ok", "route_accepted"),
            ("no", "route_rejected"),
            ("later", "unmatched"),
        ] {
            assert_eq!(
                actor
                    .decide(
                        &ProductValue::object([("status", ProductValue::string(status))]).unwrap()
                    )
                    .as_str(),
                expected,
            );
        }
        let (kind, value, _, identity) = admitted.into_parts();
        assert_eq!(kind, ActorType::Route);
        assert_eq!(value, config);
        assert_eq!(identity, None);
    }

    #[test]
    fn lookup_accepts_exactly_the_boarded_set_and_rejects_every_other_declared_actor_type() {
        let boarded = FACTORY_EDITABILITY
            .into_iter()
            .filter(|row| row.boarded)
            .map(|row| row.actor_type)
            .chain(std::iter::once(ActorType::Otlp))
            .collect::<Vec<_>>();
        for boarded in boarded.iter().copied() {
            let factory = product_actor_factory::<TestTypes>(boarded)
                .expect("boarded actor has a product factory");
            assert_eq!(factory.actor_type(), boarded);
            assert!(std::ptr::eq(factory.spec(), get(boarded)));
            assert_eq!(
                crate::registration(boarded).factory(),
                if matches!(boarded, ActorType::Otlp | ActorType::Listener) {
                    crate::FactoryArm::Source
                } else {
                    crate::FactoryArm::Actor
                }
            );
        }

        let rejected: Vec<_> = ActorType::ALL
            .into_iter()
            .filter(|kind| product_actor_factory::<TestTypes>(*kind).is_none())
            .collect();
        let expected: Vec<_> = ActorType::ALL
            .into_iter()
            .filter(|kind| !boarded.contains(kind))
            .collect();
        assert_eq!(rejected, expected);
        assert_eq!(rejected.len(), ActorType::COUNT - boarded.len());
    }

    #[test]
    fn ema_from_product_table_accepts_unstamped_input_as_first_sample() {
        use circular_plan::{
            Config, Generation, GenerationVector, Incarnation, NamedActorId, ScopeId,
        };
        use circular_runtime::ActorEffect;

        let factory = product_actor_factory::<TestTypes>(ActorType::Ema).unwrap();
        let ProductActor::Ema(mut actor) = factory
            .create(
                &folded(
                    ActorType::Ema,
                    ProductValue::object([("half_life", ProductValue::Int(1))]).unwrap(),
                ),
                &(),
                &crate::ResolvedInletShapes::default(),
            )
            .unwrap()
        else {
            panic!("EMA factory arm");
        };
        let named = NamedActorId::new(ScopeId::root(), circular_plan::Name::from_normalized("ema"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .unwrap();
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let config = Config::default();
        let context = ActorContext::new(&actor_id, &incarnation, &config, &());
        let payload = ProductPayload::new(
            crate::GroundShape::try_new(crate::Shape::Base(crate::BaseShape::Float)).unwrap(),
            ProductValue::float(10.0),
        );
        assert_eq!(payload.recorded_instant(), None);
        let input = ActorInput::new(circular_core::PortId::try_new("sample").unwrap(), payload);
        assert_eq!(actor.state(), &crate::EmaState::Empty);
        let effects = <crate::EmaActor<u16, circular_runtime::EffectId> as EmittingActor<
            TestTypes,
            ProductPayload,
        >>::on_event(&mut actor, &input, &context);
        let [ActorEffect::Emit { port, payload, .. }] = effects.as_slice() else {
            panic!("one EMA Emit");
        };
        assert_eq!(port, &circular_core::PortId::try_new("ema").unwrap());
        assert_eq!(
            payload.value(),
            &ProductValue::object([
                ("value", ProductValue::float(10.0)),
                ("samples", ProductValue::Int(1))
            ])
            .unwrap()
        );
        assert_eq!(
            actor.state(),
            &crate::EmaState::Ready {
                last_value: circular_core::FloatValue::new(10.0),
                n: 1,
                last_at: None,
            }
        );
    }

    #[test]
    fn ema_from_product_table_initializes_and_halves_by_sample_count() {
        use circular_core::{Causality, Emission, Sequence, Stamp, Tick, admit, emit};
        use circular_plan::{
            Config, Generation, GenerationVector, Incarnation, NamedActorId, ScopeId,
        };
        use circular_runtime::ActorEffect;

        let factory = product_actor_factory::<StampedTypes>(ActorType::Ema);
        assert!(factory.is_some());
        let mut actor = factory
            .unwrap()
            .create(
                &folded(
                    ActorType::Ema,
                    ProductValue::object([("half_life", ProductValue::Int(1))]).unwrap(),
                ),
                &(),
                &crate::ResolvedInletShapes::default(),
            )
            .unwrap();
        let named = NamedActorId::new(ScopeId::root(), circular_plan::Name::from_normalized("ema"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .unwrap();
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let config = Config::default();
        let context = ActorContext::new(&actor_id, &incarnation, &config, &());

        for (sequence, time, sample, expected, samples) in
            [(1, 0, 10.0, 10.0, 1), (2, 100, 0.0, 5.0, 2)]
        {
            let payload = ProductPayload::new(
                crate::GroundShape::try_new(crate::Shape::Base(crate::BaseShape::Float)).unwrap(),
                ProductValue::float(sample),
            );
            let stamp = Stamp::from_event_producer(
                Tick::new(time),
                named.clone(),
                Sequence::new(sequence).unwrap(),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            );
            let event = admit(
                TestRun::default(),
                Emission::from_runtime(emit(payload), Causality::Source, None),
                stamp,
            )
            .unwrap();
            assert_eq!(event.recorded_instant(), None);
            let input = ActorInput::new(circular_core::PortId::try_new("sample").unwrap(), event);
            let effects = actor.on_event(&input, &context);
            let [ActorEffect::Emit { port, payload, .. }] = effects.as_slice() else {
                panic!("EMA Emit");
            };
            assert_eq!(port, &circular_core::PortId::try_new("ema").unwrap());
            assert_eq!(
                payload.value(),
                &ProductValue::object([
                    ("value", ProductValue::float(expected)),
                    ("samples", ProductValue::Int(samples)),
                ])
                .unwrap()
            );
        }
    }

    fn empty_config() -> ProductValue {
        ProductValue::object(std::iter::empty::<(String, ProductValue)>())
            .expect("an empty object has no duplicate keys")
    }

    fn folded(actor_type: ActorType, value: ProductValue) -> circular_runtime::FoldedConfig {
        circular_runtime::FoldedConfig::minted(actor_type, value)
    }

    #[test]
    fn a_folded_config_of_another_actor_type_never_reaches_the_arm() {
        let factory = product_actor_factory::<TestTypes>(ActorType::Route).unwrap();
        let Err(error) = factory.create(
            &folded(ActorType::Counter, empty_config()),
            &(),
            &crate::ResolvedInletShapes::default(),
        ) else {
            panic!("a folded value of another kind is refused");
        };
        let ProductFactoryError::FoldMismatch(mismatch) = error else {
            panic!("the reason is a pairing mismatch, not a route config defect");
        };
        assert_eq!(mismatch.expected(), ActorType::Route);
        assert_eq!(mismatch.found(), ActorType::Counter);
    }
}
