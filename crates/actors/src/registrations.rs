
use circular_core::Value;
use circular_expr::{ConfigPath, EvalMode};
use circular_protocol::declaration_payload::PreprocessKind;

use crate::capabilities::{
    Condition, Durability, EffectCtor, EffectDecl, ExternalEffect, NoExternalEffect, RequireRule,
    RequireRules, StandIn, StandIns,
};
use crate::config::{
    ClosedTags, CompareTarget, ConfigConstraint, ConfigSchema, ConfigSlot, ConfigSpace,
    CreateInputRelation, IntervalDomain, Required, SnippetSlot, SuppressDecl, SuppressRule,
    TextDomain, interval_space,
};
use crate::ports::{
    Arity, AuthoredPortId, BoundaryPortRule, DynamicRule, ExpansionRule, FlowChoice, FlowCtor,
    InletSpec, LabelRule, OutletSpec, OutletTemplate, PortId, PortRule, PortSet, PortTemplate,
    Presence, Side, TypeRule,
};
use crate::spec::{Description, FactoryArm, Label, SpecSource};
use crate::table::{
    ActorType, Deferral, FixtureAssumption, FixtureManifest, GraphPresentationRole, Registration,
    RegistrationScope,
};
use crate::types::{BaseShape, FieldMap, Flow, Name, Shape};
use circular_runtime::{Capability, EffectFailure, OutcomePayload};

pub(crate) static JUDGMENT_11_DEFERRED: Deferral = Deferral::new(
    "inactive registration that resume-and-continue does not reach; held outside the published catalog",
);

pub(crate) static JUDGMENT_T135_COMBINATORS: Deferral = Deferral::new(
    "combinator: neither a node nor an actor but logic on the wire, evaluated at the destination inlet",
);

pub(crate) struct RegistrationAuthority(());

static REGISTRATION_AUTHORITY: RegistrationAuthority = RegistrationAuthority(());

pub(crate) fn register<E: EffectDecl>(
    source: SpecSource<E>,
    factory: FactoryArm,
    scope: RegistrationScope,
    presentation: GraphPresentationRole,
    adjudication: &'static str,
    semantics_key: &'static str,
) -> Registration {
    Registration::from_source(
        &REGISTRATION_AUTHORITY,
        source,
        factory,
        scope,
        presentation,
        adjudication,
        semantics_key,
    )
}

mod accumulator;
mod boundary_sink;
mod boundary_source;
mod control;
mod fixture;
mod otlp;
mod peer;
mod selection;
mod structure;
mod transform;

pub(crate) use accumulator::{
    assemble_source, counter_source, ema_source, join_source, keyed_reduce_source,
    token_cost_meter_source, windowed_reduce_source,
};
pub(crate) use boundary_sink::{
    agent_source, cli_source, file_source, notify_source, request_source, tool_executor_source,
};
pub(crate) use boundary_source::{form_source, json_source, listener_source, timer_source};
pub(crate) use control::{alert_source, debounce_source, throttle_source};
pub(crate) use fixture::{
    EDITABLE_AUX_MANIFEST, EDITABLE_COUNTER_MANIFEST, EDITABLE_SCOPE_PROBE_MANIFEST,
    FILTER_MANIFEST, FIXTURE_PANIC_MANIFEST, INPUT_MANIFEST, MAP_MANIFEST, PROJECT_OUTPUT_MANIFEST,
    TAP_MANIFEST, editable_aux_source, editable_counter_source, editable_scope_probe_source,
    fixture_filter_source, fixture_input_source, fixture_map_source, fixture_panic_source,
    fixture_project_output_source, fixture_tap_source,
};
pub(crate) use otlp::otlp_source;
pub(crate) use peer::peer_source;
pub(crate) use selection::{dedup_source, filter_source, match_source, route_source};
pub(crate) use structure::{input_source, output_source, pipeline_actor_source, replicator_source};
pub(crate) use transform::{bang_source, map_source, parse_source, tap_source};

fn published_effect_free_source(
    label: Label,
    description: Description,
    ports: PortRule,
    config: ConfigSchema,
) -> SpecSource<NoExternalEffect> {
    SpecSource::new(
        &REGISTRATION_AUTHORITY,
        label,
        description,
        ports,
        RequireRules::none(),
        NoExternalEffect,
        config,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "the arguments mirror the exhaustive SpecSource fields after fixed Live lifecycle"
)]
fn published_external_source(
    label: Label,
    description: Description,
    ports: PortRule,
    requires: RequireRules<ExternalEffect>,
    effect: ExternalEffect,
    config: ConfigSchema,
) -> SpecSource<ExternalEffect> {
    SpecSource::new(
        &REGISTRATION_AUTHORITY,
        label,
        description,
        ports,
        requires,
        effect,
        config,
    )
}

fn failed_stand_ins(first: EffectCtor, rest: impl IntoIterator<Item = EffectCtor>) -> StandIns {
    StandIns::try_new(
        (first, StandIn::Failed(EffectFailure::EndpointGone)),
        rest.into_iter()
            .map(|constructor| (constructor, StandIn::Failed(EffectFailure::EndpointGone))),
    )
    .expect("published external declarations list each constructor once")
}

fn single_request_result_ports(
    request_id: &'static str,
    request_label: &'static str,
    result_id: &'static str,
    result_label: &'static str,
) -> PortSet {
    PortSet::try_new(
        vec![required_primary_inlet(
            request_id,
            Flow::Stream(open_object_shape()),
            request_label,
        )],
        vec![primary_outlet(
            result_id,
            Flow::Stream(open_object_shape()),
            result_label,
        )],
    )
    .expect("request and result live in distinct direction domains")
}

fn single_sink_inlet(id: &'static str, label: &'static str) -> PortSet {
    PortSet::try_new(
        vec![required_primary_inlet(id, Flow::Stream(Shape::Any), label)],
        Vec::new(),
    )
    .expect("single sink inlet is unique")
}

fn event_passthrough_rule(item: Shape) -> PortRule {
    let flow = Flow::Stream(item);
    let fixed = PortSet::try_new(
        vec![required_primary_inlet("event", flow.clone(), "Event")],
        vec![primary_outlet("event", flow, "Event")],
    )
    .expect("the shared event inlet and outlet use separate direction domains");
    PortRule::new(fixed, Box::new([]))
}

fn required_primary_inlet(id: &'static str, ty: Flow, label: &'static str) -> InletSpec {
    required_inlet(id, ty, true, label)
}

fn required_inlet(id: &'static str, ty: Flow, primary: bool, label: &'static str) -> InletSpec {
    InletSpec::try_new(
        authored_port_id(id),
        ty,
        Arity::Many,
        Presence::Required,
        primary,
        port_label(label),
    )
    .expect("required published inlet has no default to mismatch")
}

fn primary_outlet(id: &'static str, ty: Flow, label: &'static str) -> OutletSpec {
    outlet(id, ty, true, label)
}

fn outlet(id: &'static str, ty: Flow, primary: bool, label: &'static str) -> OutletSpec {
    OutletSpec::new(
        authored_port_id(id),
        ty,
        Arity::Many,
        primary,
        port_label(label),
    )
}

fn polymorphic_event_flow() -> Flow {
    Flow::Stream(Shape::Var(Name::from_static("T")))
}

pub(crate) fn alert_transition_shape() -> Shape {
    Shape::Object {
        fields: FieldMap::try_new(vec![
            (Name::from_static("from"), Shape::Base(BaseShape::String)),
            (Name::from_static("to"), Shape::Base(BaseShape::String)),
        ])
        .expect("alert transition fields are distinct"),
        open: false,
    }
}

fn open_object_shape() -> Shape {
    Shape::Object {
        fields: FieldMap::try_new(Vec::new()).expect("empty field map is unique"),
        open: true,
    }
}

fn config_schema<const N: usize>(entries: [(ConfigPath, ConfigSlot); N]) -> ConfigSchema {
    ConfigSchema::try_from_parts(entries, None).expect("published config paths are distinct")
}

fn blocked_config_schema<const N: usize>(
    entries: [(ConfigPath, ConfigSlot); N],
    stop_line: &'static str,
) -> ConfigSchema {
    config_schema(entries).with_create_input_stop_line(stop_line)
}

fn create_config_schema<const N: usize>(entries: [(ConfigPath, ConfigSlot); N]) -> ConfigSchema {
    ConfigSchema::try_from_create_parts(entries, [], None)
        .expect("published create inputs have a complete top-level admission contract")
}

fn preprocess_create_config_schema(
    kind: PreprocessKind,
    suppress: Option<SuppressDecl>,
) -> ConfigSchema {
    ConfigSchema::try_from_create_parts(crate::preprocess_schema::config_slots(kind), [], suppress)
        .expect(
            "each preprocess field is one top-level key, so the create-input contract is complete",
        )
}

fn suppressed_create_config_schema<const N: usize>(
    entries: [(ConfigPath, ConfigSlot); N],
    suppress: SuppressDecl,
) -> ConfigSchema {
    ConfigSchema::try_from_create_parts(entries, [], Some(suppress)).expect(
        "published create inputs and suppression metadata have a complete admission contract",
    )
}

fn mandatory_slot(shape: Shape, snippet: Option<SnippetSlot>) -> ConfigSlot {
    mandatory_slot_in(ConfigSpace::unconstrained(shape), snippet)
}

fn mandatory_slot_in(space: ConfigSpace, snippet: Option<SnippetSlot>) -> ConfigSlot {
    ConfigSlot::try_new(space, Required::Mandatory, snippet)
        .expect("mandatory config slots have no default to mismatch")
}

const EFFECT_POLICY: &str = "Effect policy";

const TIMING: &str = "Timing";

/// The grants an effect-bearing actor holds. `demand` is what makes a declared policy part of
/// every authored config: this slot's requirement is the one place it is declared,
/// published as it stands and read back by acceptance (`capability_config::demand`).
/// `description` is this actor's sentence for the grants.
fn capabilities_slot(
    demand: crate::capability_config::PolicyDemand,
    description: &str,
) -> (ConfigPath, ConfigSlot) {
    let slot = ConfigSlot::try_new(
        ConfigSpace::unconstrained(open_object_shape()),
        demand.requirement(),
        None,
    )
    .expect("the empty policy object fits the open object space");
    let slot = match slot.required() {
        Required::Optional { .. } => slot.preserving_omission(),
        _ => slot,
    };
    (
        config_path(crate::capability_config::FIELD),
        slot.with_text("Permissions", description)
            .in_group(EFFECT_POLICY),
    )
}

/// Whether the actor's effect waits for approval. `description` is this actor's sentence for it.
fn approval_slot(description: &str) -> (ConfigPath, ConfigSlot) {
    (
        config_path(crate::approval_config::APPROVAL_FIELD),
        optional_slot_in(
            crate::approval_config::approval_space(),
            Value::String(crate::approval_config::APPROVAL_VALUES[0].into()),
            None,
        )
        .preserving_omission()
        .with_text("Require approval", description)
        .in_group(EFFECT_POLICY),
    )
}

fn retry_slot(description: &str) -> (ConfigPath, ConfigSlot) {
    (
        config_path(crate::retry_config::RETRY_FIELD),
        optional_slot_in(
            crate::config::interval_list_space(IntervalDomain::NonZeroMilliseconds),
            Value::array(
                crate::retry_config::CREATOR_DEFAULT_MS
                    .map(|ms| Value::Int(i64::try_from(ms).expect("small milliseconds"))),
            ),
            None,
        )
        .preserving_omission()
        .with_text("Retry waits", description)
        .in_group(EFFECT_POLICY),
    )
}

fn described(
    (path, slot): (ConfigPath, ConfigSlot),
    label: &str,
    description: &str,
) -> (ConfigPath, ConfigSlot) {
    (path, slot.with_text(label, description))
}

/// A mandatory slot entry with the value a create form opens it at. The slot stays
/// mandatory; the value is the registration's suggestion, published in the create draft.
fn starting((path, slot): (ConfigPath, ConfigSlot), value: Value) -> (ConfigPath, ConfigSlot) {
    (path, slot.starting_at(value))
}

/// The same for a millisecond interval slot, drawn under the timing group.
fn timed(
    entry: (ConfigPath, ConfigSlot),
    label: &str,
    description: &str,
) -> (ConfigPath, ConfigSlot) {
    let (path, slot) = described(entry, label, description);
    (path, slot.in_group(TIMING))
}

fn view_defaults<const N: usize>(entries: [(&'static str, Value); N]) -> Value {
    Value::object(entries).expect("view default keys are distinct")
}

fn optional_slot(shape: Shape, default: Value, snippet: Option<SnippetSlot>) -> ConfigSlot {
    optional_slot_in(ConfigSpace::unconstrained(shape), default, snippet)
}

fn optional_slot_in(
    space: ConfigSpace,
    default: Value,
    snippet: Option<SnippetSlot>,
) -> ConfigSlot {
    ConfigSlot::try_new(space, Required::Optional { default }, snippet)
        .expect("published optional defaults match their declared spaces")
}

fn constrained_space(shape: Shape, constraint: ConfigConstraint) -> ConfigSpace {
    ConfigSpace::try_new(shape, constraint)
        .expect("published unary constraint matches its canonical Value shape")
}

fn mandatory_typed<K: crate::config::SlotKind>(
    slot: &crate::config::Slot<K>,
) -> (ConfigPath, ConfigSlot) {
    (slot.path(), mandatory_slot_in(slot.space(), None))
}

fn optional_typed<K: crate::config::SlotKind>(
    slot: &crate::config::Slot<K>,
    default: Value,
) -> (ConfigPath, ConfigSlot) {
    (slot.path(), optional_slot_in(slot.space(), default, None))
}

fn decision_suppression(reason: &'static str) -> SuppressDecl {
    SuppressDecl::try_from_entries([(Name::from_static(reason), SuppressRule::NamedOnly)])
        .expect("a published suppression declaration contains one unique reason")
}

fn predicate_snippet() -> SnippetSlot {
    SnippetSlot::new(
        EvalMode::Predicate,
        vec![port_reference("event")].into_boxed_slice(),
    )
}

fn single_input_snippet(mode: EvalMode, inlet: &'static str) -> SnippetSlot {
    SnippetSlot::new(mode, vec![port_reference(inlet)].into_boxed_slice())
}

fn config_path(value: &'static str) -> ConfigPath {
    ConfigPath::root().join_key(value)
}

fn port_reference(value: &'static str) -> PortId {
    PortId::try_new(value.to_owned()).expect("registration port reference is canonical")
}

fn fixture_effect_free_source(
    label: &'static str,
    description: &'static str,
    fixed: PortSet,
) -> SpecSource<NoExternalEffect> {
    SpecSource::new(
        &REGISTRATION_AUTHORITY,
        actor_label(label),
        Description::from_static(description),
        PortRule::new(fixed, Box::new([])),
        RequireRules::none(),
        NoExternalEffect,
        ConfigSchema::empty(),
    )
}

fn fixture_effect_free_source_with_config(
    label: &'static str,
    description: &'static str,
    fixed: PortSet,
    config: ConfigSchema,
) -> SpecSource<NoExternalEffect> {
    SpecSource::new(
        &REGISTRATION_AUTHORITY,
        actor_label(label),
        Description::from_static(description),
        PortRule::new(fixed, Box::new([])),
        RequireRules::none(),
        NoExternalEffect,
        config,
    )
}

fn fixture_external_source(
    label: &'static str,
    description: &'static str,
    fixed: PortSet,
    requires: RequireRules<ExternalEffect>,
    effect: ExternalEffect,
) -> SpecSource<ExternalEffect> {
    SpecSource::new(
        &REGISTRATION_AUTHORITY,
        actor_label(label),
        Description::from_static(description),
        PortRule::new(fixed, Box::new([])),
        requires,
        effect,
        create_config_schema([capabilities_slot(
            crate::capability_config::PolicyDemand::EveryDeclared,
            "The grants for this fixture's effects.",
        )]),
    )
}

fn authored_port_id(value: &'static str) -> AuthoredPortId {
    PortId::try_authored_static(value).expect("registration port id is canonical")
}

fn port_label(value: &'static str) -> Label {
    Label::try_from_static(value).expect("registration port label is canonical")
}

fn actor_label(value: &'static str) -> Label {
    Label::try_from_static(value).expect("registration actor label is canonical")
}

mod support {
    pub(super) use super::PreprocessKind;
    pub(super) use super::approval_slot;
    pub(super) use super::retry_slot;
    pub(super) use super::{
        ActorType, Arity, BaseShape, BoundaryPortRule, Capability, ClosedTags, CompareTarget,
        Condition, ConfigConstraint, ConfigPath, ConfigSchema, ConfigSlot, ConfigSpace,
        CreateInputRelation, Description, Durability, DynamicRule, EffectCtor, EffectFailure,
        EvalMode, ExpansionRule, ExternalEffect, FieldMap, FixtureAssumption, FixtureManifest,
        Flow, FlowChoice, FlowCtor, InletSpec, IntervalDomain, LabelRule, Name, NoExternalEffect,
        OutcomePayload, OutletSpec, OutletTemplate, PortRule, PortSet, PortTemplate, Presence,
        RequireRule, RequireRules, Required, Shape, Side, SpecSource, StandIn, StandIns,
        SuppressDecl, SuppressRule, TextDomain, TypeRule, Value, actor_label,
        alert_transition_shape, authored_port_id, blocked_config_schema, capabilities_slot,
        config_path, config_schema, constrained_space, create_config_schema, decision_suppression,
        described, event_passthrough_rule, failed_stand_ins, fixture_effect_free_source,
        fixture_effect_free_source_with_config, fixture_external_source, interval_space,
        mandatory_slot, mandatory_slot_in, mandatory_typed, open_object_shape, optional_slot,
        optional_slot_in, optional_typed, outlet, polymorphic_event_flow, port_label,
        port_reference, predicate_snippet, preprocess_create_config_schema, primary_outlet,
        published_effect_free_source, published_external_source, required_inlet,
        required_primary_inlet, single_input_snippet, single_request_result_ports,
        single_sink_inlet, starting, suppressed_create_config_schema, timed, view_defaults,
    };
    pub(super) use crate::capability_config::PolicyDemand;
}

