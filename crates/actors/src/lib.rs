#![forbid(unsafe_code)]

mod actor_registry;
mod actor_support;
pub mod agent_actor;
pub mod agent_tool_wire;
pub mod alert_actor;
pub mod approval_config;
pub mod arming;
pub mod assemble;
mod bang;
pub mod boundary_actor;
pub mod capabilities;
pub mod capability_config;
pub mod config;
mod counter;
mod create_input;
pub mod debounce;
pub mod editability;
pub mod ema_state;
pub mod file_actor;
mod filter_config;
mod filter_preprocess;
pub mod fixture_panic;
pub mod flatten;
pub mod inlet_shapes;
pub mod instance_value;
pub mod json_actor;
pub mod keyed_reduce;
pub mod listener;
pub mod map_config;
mod map_preprocess;
pub mod notify_actor;
pub mod otlp;
pub mod payload_value;
pub mod peer_actor;
pub use parse_config::json_value;
pub mod parse_config;
pub mod ports;
pub mod preprocess_schema;
mod query;
mod registrations;
pub mod replicator_actor;
pub mod request_actor;
pub mod retry_config;
mod route_actor;
mod route_config;
pub mod spec;
mod table;
mod tap;
pub mod timer_actor;
pub mod tool_executor_actor;
pub mod types;
pub mod windowed_reduce;

pub use actor_registry::{
    ProductActor, ProductActorFactory, ProductFactoryError, ProductObservation, ProductPayload,
    ProductValue, fixture_actor_factory, is_entry_actor, judge_activation_config,
    product_actor_factory,
};
pub use actor_support::StampedEvent;
pub use agent_actor::{AgentActor, AgentActorPhase, AgentFactory, AgentFactoryError};
pub use alert_actor::{
    ALERT_STATE_SCHEMA, AlertActor, AlertFactory, AlertFactoryError, AlertState,
};
pub use bang::{EmptyConfigFactoryError, bang_event, reject_nonempty_config};
pub use capabilities::{
    Condition, Durability, EffectCtor, EffectDecl, EffectDeclaration, ExternalEffect,
    NoExternalEffect, RequireRule, RequireRules, StandIn, StandIns,
};
pub use circular_expr::{ConfigPath, EvalMode, Segment};
pub use config::{
    AdmittedCreateConfig, ClosedTags, ClosedTagsError, CompareTarget, ConfigConstraint,
    ConfigSchema, ConfigSlot, ConfigSlotError, ConfigSpace, ConfigSpaceError,
    ConfigUnaryReadinessError, CreateInputAdmissionError, CreateInputDraft, CreateInputRelation,
    CreateInputSchema, CreateInputSealError, CreateInputUnavailable, DuplicateConfigPath,
    IntegerBound, IntegerBoundError, IntegerCount, IntegerCountError, IntegerMinimum,
    IntervalDomain, PayloadPath, PayloadRoot, Required, SnippetSlot, SuppressDecl,
    SuppressDeclError, SuppressRule, SuppressionAdmissionError, TextDomain,
    UnresolvedConfigConstraint,
};
pub use counter::{COUNTER_STATE_SCHEMA, CounterActor, CounterFactory};
pub use create_input::{
    AdmittedActorCreate, RegisteredCreateAdmissionError, admit_registered_create,
    admit_registered_create_at, registered_create_draft, registered_create_inputs,
};
pub use debounce::{
    DEBOUNCE_STATE_SCHEMA, DebounceActor, DebounceFactory, DebounceFactoryError, DebounceState,
};
pub use ema_state::{
    EMA_STATE_SCHEMA, EmaActor, EmaFactory, EmaFactoryError, EmaState, EmaStateError,
};
pub use file_actor::{FileActor, FileFactory, FileFactoryError};
pub use filter_config::{FilterConfigError, accept_predicate};
pub use filter_preprocess::{FilterFailure, filter_event};
pub use inlet_shapes::ResolvedInletShapes;
pub use json_actor::{JSON_STATE_SCHEMA, JsonActor, JsonFactory, JsonFactoryError};
pub use map_config::accept_transform;
pub use map_preprocess::{MapFailure, map_event};
pub use notify_actor::{NotifyActor, NotifyConfig, NotifyFactory, NotifyFactoryError, NotifyState};
pub use parse_config::{CompiledDecoder, ParseConfig, parse_event};
pub use ports::{
    Arity, AuthoredPortId, BoundaryPortRule, DefaultValueShapeMismatch, Direction, DuplicatePortId,
    DynamicRule, Expansion, ExpansionElement, ExpansionRule, ExpansionStorage, FlowChoice,
    FlowCtor, GeneratedPortIdError, In, InletSpec, InletTemplate, InvalidCountReason, LabelRule,
    Out, OutletSpec, OutletTemplate, PortExpansionError, PortId, PortIdError, PortOrigin, PortRule,
    PortSet, PortSpec, PortTemplate, PortTemplateOf, Presence, Side, TypeRule, TypeRuleKind,
};
pub use query::{
    ERROR_PORT_NAME, EndpointAnswer, LIFECYCLE_PORT_NAME, StaticPort, StaticPortQueryError,
    StaticPortRequest, TIMER_PORT_NAME, answer_endpoint, available_ports, derived_error_port,
    derived_error_port_for_empty_config, requires, resolve_static_port,
};
pub use request_actor::{
    RequestActor, RequestConfig, RequestFactory, RequestFactoryError, RequestPayloadError,
};
pub use route_actor::RouteActor;
pub use route_config::{
    RouteCase, RouteCases, RouteCasesError, RouteConfig, RouteConfigError, select,
};
pub use spec::{ActorSpec, Description, Display, FactoryArm, Label, SpecSource};
pub use table::{
    ActorType, BoundaryDirection, ContainerCardinality, Deferral, FixtureAssumption,
    FixtureManifest, GraphPresentationRole, Registration, RegistrationScope, derived_error_outlet,
    get, registration, start_inlet,
};
pub use tap::{TapActor, TapFactory};
pub use timer_actor::{
    STALE_TIMER_FIRE, TIMER_STATE_SCHEMA, TimerActor, TimerFactory, TimerFactoryError,
};
pub use tool_executor_actor::{
    ToolEffectTemplate, ToolExecutorActor, ToolExecutorConfig, ToolExecutorFactory,
    ToolExecutorFactoryError, ToolExecutorState,
};
pub use types::{
    Assigned, BaseShape, FieldMap, Flow, GroundFlow, GroundShape, Name, RateExpr, Shape,
    Substitution, VariableConflict, connectable, flow_from_port_type, port_type_from_flow,
    value_inhabits,
};
pub use windowed_reduce::{
    REDUCE_FAILED, WINDOWED_REDUCE_STATE_SCHEMA, WindowedReduceActor, WindowedReduceFactory,
    WindowedReduceFactoryError, WindowedReduceSample,
};

pub use peer_actor::{PeerActor, PeerConfig, PeerFactory, PeerFactoryError, PeerState};

mod failure_detail;
pub use failure_detail::FailureDetail;

mod failure_value;
pub use failure_value::{
    dead_letter_reason_value, dead_letter_reason_with_failure_point, effect_failure_detail,
};

mod match_actor;

pub mod join;

mod listener_actor;
