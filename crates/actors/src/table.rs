
pub use circular_core::ActorType;

use crate::capabilities::EffectDecl;
use crate::registrations::RegistrationAuthority;
use crate::spec::{ActorSpec, FactoryArm, SpecSource};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureAssumption {
    field: &'static str,
    value: &'static str,
    reason: &'static str,
}

impl FixtureAssumption {
    #[must_use]
    pub const fn new(field: &'static str, value: &'static str, reason: &'static str) -> Self {
        Self {
            field,
            value,
            reason,
        }
    }

    #[must_use]
    pub const fn field(self) -> &'static str {
        self.field
    }

    #[must_use]
    pub const fn value(self) -> &'static str {
        self.value
    }

    #[must_use]
    pub const fn reason(self) -> &'static str {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixtureManifest {
    source: &'static str,
    assumptions: &'static [FixtureAssumption],
}

impl FixtureManifest {
    #[must_use]
    pub const fn new(source: &'static str, assumptions: &'static [FixtureAssumption]) -> Self {
        Self {
            source,
            assumptions,
        }
    }

    #[must_use]
    pub const fn source(self) -> &'static str {
        self.source
    }

    #[must_use]
    pub const fn assumptions(self) -> &'static [FixtureAssumption] {
        self.assumptions
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Deferral {
    reason: &'static str,
}

impl Deferral {
    #[must_use]
    pub const fn new(reason: &'static str) -> Self {
        Self { reason }
    }

    #[must_use]
    pub const fn reason(self) -> &'static str {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistrationScope {
    Published,
    FixtureLocal(&'static FixtureManifest),
    Deferred(&'static Deferral),
}

/// Direction of a semantic system boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryDirection {
    Source,
    Sink,
}

/// Declared runtime cardinality of a recursive container family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContainerCardinality {
    One,
    KeyedMany,
}

/// Canonical graph-presentation family owned by the actor registration.
///
/// This is deliberately not derived from port cardinality, graph degree,
/// factory arm, catalog family, or a renderer-owned list.  Those axes do not
/// determine presentation semantics: for example `windowed_reduce` is backed
/// by a source factory but is an inline stream combinator, while several
/// request/result boundary actors remain full bodies.  Every registration row
/// therefore makes one closed choice and the daemon projects that choice to
/// clients.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphPresentationRole {
    Instrument,
    InlineOperation,
    SystemBoundary(BoundaryDirection),
    Container(ContainerCardinality),
}

#[must_use]
pub const fn start_inlet(actor_type: ActorType) -> Option<&'static str> {
    match actor_type {
        ActorType::Json | ActorType::Timer => Some("bang"),
        ActorType::Listener => Some("control"),
        _ => None,
    }
}

/// Sealed presentation designation for the complete registered domain.
///
/// The inline-operation set is an explicit registration designation.  It intentionally
/// includes stateful fittings such as `collect`, `ema`, and `latch`, and
/// intentionally excludes similarly shaped bodies such as `store`, `gate`,
/// and `agent_action`.  Boundary roles likewise name semantic
/// system edges rather than asking the renderer to count ports.
const fn canonical_presentation(actor_type: ActorType) -> GraphPresentationRole {
    match actor_type {
        ActorType::Input
        | ActorType::Json
        | ActorType::Timer
        | ActorType::Listener
        | ActorType::Otlp => GraphPresentationRole::SystemBoundary(BoundaryDirection::Source),
        ActorType::Output
        | ActorType::Notify
        | ActorType::Request => GraphPresentationRole::SystemBoundary(BoundaryDirection::Sink),
        ActorType::Debounce
        | ActorType::Throttle
        | ActorType::Filter
        | ActorType::Dedup
        | ActorType::Map
        | ActorType::Parse
        | ActorType::Tap
        | ActorType::Ema
        | ActorType::WindowedReduce
        | ActorType::Form => GraphPresentationRole::InlineOperation,
        ActorType::PipelineActor => GraphPresentationRole::Container(ContainerCardinality::One),
        ActorType::Replicator => GraphPresentationRole::Container(ContainerCardinality::KeyedMany),
        ActorType::Route
        | ActorType::Match
        | ActorType::Join
        | ActorType::Assemble
        | ActorType::FixtureInput
        | ActorType::FixtureMap
        | ActorType::FixtureFilter
        | ActorType::FixtureTap
        | ActorType::FixtureProjectOutput
        | ActorType::EditableCounter
        | ActorType::EditableAux
        | ActorType::EditableScopeProbe
        | ActorType::Alert
        | ActorType::Agent
        | ActorType::TokenCostMeter
        | ActorType::Counter
        | ActorType::ToolExecutor
        | ActorType::Cli
        | ActorType::Bang
        | ActorType::Peer
        | ActorType::File
        | ActorType::KeyedReduce
        | ActorType::FixturePanic => GraphPresentationRole::Instrument,
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum CatalogFamily {
    BoundarySource,
    BoundarySink,
    Transform,
    Selection,
    Accumulator,
    Grid,
    Control,
    Structure,
    Observation,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct PublishedCatalogRegistration {
    name: &'static str,
    family: CatalogFamily,
}

#[cfg(test)]
impl PublishedCatalogRegistration {
    const fn new(name: &'static str, family: CatalogFamily) -> Self {
        Self { name, family }
    }

    #[must_use]
    const fn name(self) -> &'static str {
        self.name
    }

    #[must_use]
    const fn family(self) -> CatalogFamily {
        self.family
    }
}

#[doc(hidden)]
#[derive(Debug, PartialEq)]
pub struct Registration {
    spec: ActorSpec,
    factory: FactoryArm,
    issues_schedule: bool,
    scope: RegistrationScope,
    presentation: GraphPresentationRole,
    adjudication: &'static str,
    semantics_key: &'static str,
}

impl Registration {
    pub(crate) fn from_source<E: EffectDecl>(
        authority: &RegistrationAuthority,
        source: SpecSource<E>,
        factory: FactoryArm,
        scope: RegistrationScope,
        presentation: GraphPresentationRole,
        adjudication: &'static str,
        semantics_key: &'static str,
    ) -> Self {
        Self {
            spec: source.derive(factory, authority),
            factory,
            issues_schedule: false,
            scope,
            presentation,
            adjudication,
            semantics_key,
        }
    }

    #[must_use]
    pub const fn spec(&self) -> &ActorSpec {
        &self.spec
    }

    #[must_use]
    pub const fn factory(&self) -> FactoryArm {
        self.factory
    }

    pub(crate) const fn issues_schedule(&self) -> bool {
        self.issues_schedule
    }

    #[must_use]
    pub const fn scope(&self) -> RegistrationScope {
        self.scope
    }

    /// The registration-authoritative graph-presentation family.
    #[must_use]
    pub const fn presentation(&self) -> GraphPresentationRole {
        self.presentation
    }

    #[must_use]
    pub const fn adjudication(&self) -> &'static str {
        self.adjudication
    }

    #[must_use]
    pub const fn semantics_key(&self) -> &'static str {
        self.semantics_key
    }
}

macro_rules! erased_registration_scope {
    (Published($family:ident)) => {
        RegistrationScope::Published
    };
    (Deferred($deferral:expr)) => {
        RegistrationScope::Deferred($deferral)
    };
    (FixtureLocal($manifest:expr)) => {
        RegistrationScope::FixtureLocal($manifest)
    };
}

#[cfg(test)]
macro_rules! catalog_projection_entry {
    (Published($family:ident), $variant:ident) => {
        Some(PublishedCatalogRegistration::new(
            ActorType::$variant.as_str(),
            CatalogFamily::$family,
        ))
    };
    (Deferred($deferral:expr), $variant:ident) => {
        None
    };
    (FixtureLocal($manifest:expr), $variant:ident) => {
        None
    };
}

macro_rules! define_actor_specs {
    (
        $(
                $variant:ident => {
                    source: $source:expr,
                    factory: $factory:expr,
                    $(issues_schedule: $issues_schedule:literal,)?
                    scope: $scope_kind:ident($($scope_value:tt)+),
                    adjudication: $adjudication:literal,
                    semantics: $semantics:literal $(,)?
            }
        ),+ $(,)?
    ) => {
        static TABLE: ::std::sync::LazyLock<[$crate::Registration; ActorType::COUNT]> =
            ::std::sync::LazyLock::new(|| ActorType::ALL.map(|actor_type| {
                #[deny(unreachable_patterns)]
                match actor_type {
                    $(ActorType::$variant => $crate::Registration {
                        $(issues_schedule: $issues_schedule,)?
                        ..$crate::registrations::register(
                            $source,
                            $factory,
                            erased_registration_scope!($scope_kind($($scope_value)+)),
                            canonical_presentation(actor_type),
                            $adjudication,
                            $semantics,
                        )
                    }),+
                }
            }));

        #[must_use]
        pub fn get(actor_type: ActorType) -> &'static $crate::ActorSpec {
            TABLE[actor_type.registration_index()].spec()
        }

        #[must_use]
        pub fn registration(actor_type: ActorType) -> &'static $crate::Registration {
            &TABLE[actor_type.registration_index()]
        }

        #[cfg(test)]
        static PUBLISHED_CATALOG_REGISTRATIONS:
            [Option<PublishedCatalogRegistration>; ActorType::COUNT] = [$(
                catalog_projection_entry!($scope_kind($($scope_value)+), $variant)
            ),+];

        #[cfg(test)]
        fn published_catalog_registrations(
        ) -> impl Iterator<Item = PublishedCatalogRegistration> {
            PUBLISHED_CATALOG_REGISTRATIONS.iter().copied().flatten()
        }

        #[cfg(test)]
        fn derive_registered_for_test(actor_type: ActorType) -> $crate::ActorSpec {
            #[deny(unreachable_patterns)]
            match actor_type {
                $(ActorType::$variant => $crate::registrations::register(
                    $source,
                    $factory,
                    erased_registration_scope!($scope_kind($($scope_value)+)),
                    canonical_presentation(ActorType::$variant),
                    $adjudication,
                    $semantics,
                ).spec),+
            }
        }
    };
}

define_actor_specs! {
    Join => {
        source: crate::registrations::join_source(),
        factory: FactoryArm::Actor,
        scope: Published(Accumulator),
        adjudication: "published accumulator: latest reference state per key, trigger join, recorded envelope result",
        semantics: "join",
    },
    Assemble => {
        source: crate::registrations::assemble_source(),
        factory: FactoryArm::Actor,
        issues_schedule: true,
        scope: Published(Accumulator),
        adjudication: "published accumulator: actor-owned windows, one object on closure, strictly excessive maximum is stuck, no checkpoint body",
        semantics: "assemble",
    },
    Match => {
        source: crate::registrations::match_source(),
        factory: FactoryArm::Actor,
        scope: Published(Selection),
        adjudication: "published selection: envelope result branching; realization fields follow the fixed stream-port convention",
        semantics: "match",
    },
    Route => {
        source: crate::registrations::route_source(),
        factory: FactoryArm::Actor,
        scope: Published(Selection),
        adjudication: "published pilot: route catalog contract and semantics; realization choices are documented at the SpecSource",
        semantics: "route",
    },
    FixtureInput => {
        source: crate::registrations::fixture_input_source(),
        factory: FactoryArm::Source,
        scope: FixtureLocal(&crate::registrations::INPUT_MANIFEST),
        adjudication: "fixture-local: engine Slice fixture source; not the published input actor",
        semantics: "input",
    },
    FixtureMap => {
        source: crate::registrations::fixture_map_source(),
        factory: FactoryArm::Actor,
        scope: FixtureLocal(&crate::registrations::MAP_MANIFEST),
        adjudication: "fixture-local: engine map hook remains distinct from the published product map registration",
        semantics: "map",
    },
    FixtureFilter => {
        source: crate::registrations::fixture_filter_source(),
        factory: FactoryArm::Actor,
        scope: FixtureLocal(&crate::registrations::FILTER_MANIFEST),
        adjudication: "fixture-local: engine filter hook remains distinct from the published product filter registration",
        semantics: "filter",
    },
    FixtureTap => {
        source: crate::registrations::fixture_tap_source(),
        factory: FactoryArm::Actor,
        scope: FixtureLocal(&crate::registrations::TAP_MANIFEST),
        adjudication: "fixture-local: engine Slice pass-through with FsRead/FsWrite effects remains distinct from the effect-free published tap registration",
        semantics: "tap",
    },
    FixtureProjectOutput => {
        source: crate::registrations::fixture_project_output_source(),
        factory: FactoryArm::Actor,
        scope: FixtureLocal(&crate::registrations::PROJECT_OUTPUT_MANIFEST),
        adjudication: "fixture-local: retired project_output spelling is isolated from the published output actor",
        semantics: "project_output",
    },
    EditableCounter => {
        source: crate::registrations::editable_counter_source(),
        factory: FactoryArm::Actor,
        scope: FixtureLocal(&crate::registrations::EDITABLE_COUNTER_MANIFEST),
        adjudication: "fixture-local: edited record/replay counter actor",
        semantics: "fixture/editable_counter",
    },
    EditableAux => {
        source: crate::registrations::editable_aux_source(),
        factory: FactoryArm::Actor,
        scope: FixtureLocal(&crate::registrations::EDITABLE_AUX_MANIFEST),
        adjudication: "fixture-local: edited record/replay auxiliary actor",
        semantics: "fixture/editable_aux",
    },
    EditableScopeProbe => {
        source: crate::registrations::editable_scope_probe_source(),
        factory: FactoryArm::Actor,
        scope: FixtureLocal(&crate::registrations::EDITABLE_SCOPE_PROBE_MANIFEST),
        adjudication: "fixture-local: edited nested-scope actor",
        semantics: "fixture/editable_scope_probe",
    },
    PipelineActor => {
        source: crate::registrations::pipeline_actor_source(),
        factory: FactoryArm::Actor,
        scope: Published(Structure),
        adjudication: "published structure: pipeline_actor ports are the daemon-derived fold of child input/output declarations and are published by authoring.actor-ports",
        semantics: "pipeline_actor",
    },
    Debounce => {
        source: crate::registrations::debounce_source(),
        factory: FactoryArm::Actor,
        issues_schedule: true,
        scope: Published(Control),
        adjudication: "published control: quiet-window pending replacement with the sibling arming correlation; the canonical TimerToken value and the InputRef cause stay outside the state schema",
        semantics: "debounce",
    },
    Throttle => {
        source: crate::registrations::throttle_source(),
        factory: FactoryArm::Actor,
        scope: Deferred(&crate::registrations::JUDGMENT_11_DEFERRED),
        adjudication: "published pilot: throttle catalog contract and semantics; realization choices are documented at the SpecSource",
        semantics: "throttle",
    },
    Alert => {
        source: crate::registrations::alert_source(),
        factory: FactoryArm::Actor,
        issues_schedule: true,
        scope: Published(Control),
        adjudication: "published pilot: alert catalog contract and semantics; realization choices are documented at the SpecSource",
        semantics: "alert",
    },
    Filter => {
        source: crate::registrations::filter_source(),
        factory: FactoryArm::Actor,
        scope: Deferred(&crate::registrations::JUDGMENT_T135_COMBINATORS),
        adjudication: "retained combinator config/spec; evaluated on the destination inlet, not published as an actor",
        semantics: "filter",
    },
    Dedup => {
        source: crate::registrations::dedup_source(),
        factory: FactoryArm::Actor,
        scope: Deferred(&crate::registrations::JUDGMENT_11_DEFERRED),
        adjudication: "published pilot: dedup catalog contract and semantics; realization choices are documented at the SpecSource",
        semantics: "dedup",
    },
    Map => {
        source: crate::registrations::map_source(),
        factory: FactoryArm::Actor,
        scope: Deferred(&crate::registrations::JUDGMENT_T135_COMBINATORS),
        adjudication: "retained combinator config/spec; evaluated on the destination inlet, not published as an actor",
        semantics: "map",
    },
    Parse => {
        source: crate::registrations::parse_source(),
        factory: FactoryArm::Actor,
        scope: Deferred(&crate::registrations::JUDGMENT_T135_COMBINATORS),
        adjudication: "retained combinator config/spec; evaluated on the destination inlet, not published as an actor",
        semantics: "parse",
    },
    Tap => {
        source: crate::registrations::tap_source(),
        factory: FactoryArm::Actor,
        scope: Published(Transform),
        adjudication: "published pilot: tap catalog contract and semantics; realization choices are documented at the SpecSource",
        semantics: "tap",
    },
    Input => {
        source: crate::registrations::input_source(),
        factory: FactoryArm::Actor,
        scope: Published(Structure),
        adjudication: "published structure: input is a pass-through boundary actor with exact label config, fixed Stream(Any), and protocol-derived boundary identity; parent wires and external injections arrive at that identity",
        semantics: "input",
    },
    Output => {
        source: crate::registrations::output_source(),
        factory: FactoryArm::Actor,
        scope: Published(Structure),
        adjudication: "published structure: output is a pass-through boundary actor with exact label config, fixed Stream(Any), and protocol-derived boundary identity; it emits to parent wires from that identity",
        semantics: "output",
    },
    Replicator => {
        source: crate::registrations::replicator_source(),
        factory: FactoryArm::Actor,
        issues_schedule: true,
        scope: Published(Structure),
        adjudication: "published structure: runtime cell state is not expressible in ActorSpec; the selected minimal event inlet is documented at the SpecSource",
        semantics: "replicator",
    },
    Agent => {
        source: crate::registrations::agent_source(),
        factory: FactoryArm::Actor,
        scope: Published(BoundarySink),
        adjudication: "published boundary sink: AgentHarness/AgentInvoke is explicit; port/config and dry-run policy choices are documented at the SpecSource",
        semantics: "agent",
    },
    TokenCostMeter => {
        source: crate::registrations::token_cost_meter_source(),
        factory: FactoryArm::Actor,
        scope: Deferred(&crate::registrations::JUDGMENT_11_DEFERRED),
        adjudication: "published accumulator: reporting stays distinct from grant-budget enforcement; state declarations remain outside ActorSpec",
        semantics: "token_cost_meter",
    },
    Counter => {
        source: crate::registrations::counter_source(),
        factory: FactoryArm::Actor,
        scope: Published(Accumulator),
        adjudication: "published accumulator: Any preserves the unresolved exact count carrier instead of silently narrowing it to Number",
        semantics: "counter",
    },
    Ema => {
        source: crate::registrations::ema_source(),
        factory: FactoryArm::Actor,
        scope: Published(Accumulator),
        adjudication: "published accumulator: selected sample/result/config surface does not claim the unresolved recurrence or state schema",
        semantics: "ema",
    },
    WindowedReduce => {
        source: crate::registrations::windowed_reduce_source(),
        factory: FactoryArm::Actor,
        issues_schedule: true,
        scope: Published(Accumulator),
        adjudication: "published accumulator: acc/sample fold from a mandatory seed, no emission for an empty window, half-open [f-w, f); the periodic phase lives in the single live reservation instead of a next_due state field",
        semantics: "windowed_reduce",
    },
    Timer => {
        source: crate::registrations::timer_source(),
        factory: FactoryArm::Actor,
        issues_schedule: true,
        scope: Published(BoundarySource),
        adjudication: "published boundary source: daemon bang arms a process-local relative Schedule and each fire emits one sequenced tick before rearming",
        semantics: "timer",
    },
    ToolExecutor => {
        source: crate::registrations::tool_executor_source(),
        factory: FactoryArm::Actor,
        scope: Published(BoundarySink),
        adjudication: "published boundary sink: the current declaration conservatively exposes the configured tool effect vocabulary; per-tool refinement remains outside ConfigSchema",
        semantics: "tool_executor",
    },
    Notify => {
        source: crate::registrations::notify_source(),
        factory: FactoryArm::Actor,
        issues_schedule: true,
        scope: Published(BoundarySink),
        adjudication: "published boundary sink: UserNotify/Notify and durable retry ownership are explicit",
        semantics: "notify",
    },
    Cli => {
        source: crate::registrations::cli_source(),
        factory: FactoryArm::Actor,
        scope: Deferred(&crate::registrations::JUDGMENT_11_DEFERRED),
        adjudication: "published boundary sink: ProcessSpawn/Spawn covers per-event execution; the coprocess session effect remains an explicit gap",
        semantics: "cli",
    },
    Bang => {
        source: crate::registrations::bang_source(),
        factory: FactoryArm::Actor,
        scope: Deferred(&crate::registrations::JUDGMENT_T135_COMBINATORS),
        adjudication: "retained combinator config/spec; evaluated on the destination inlet, not published as an actor",
        semantics: "bang",
    },
    Peer => {
        source: crate::registrations::peer_source(),
        factory: FactoryArm::Actor,
        scope: Published(BoundarySink),
        adjudication: "published bidirectional boundary: provider-neutral peer ports, four capabilities, durable send/receive effects, and replay-safe custody are explicit",
        semantics: "peer",
    },
    Listener => {
        source: crate::registrations::listener_source(),
        factory: FactoryArm::Source,
        scope: Published(BoundarySource),
        adjudication: "published boundary source (provisional name): ingress element; the control vocabulary is this element's and the mount name belongs to mount registration",
        semantics: "listener",
    },
    KeyedReduce => {
        source: crate::registrations::keyed_reduce_source(),
        factory: FactoryArm::Actor,
        scope: Published(Accumulator),
        adjudication: "published [provisional name]: per-key accumulation with eviction; the fold is addition and the cardinality is the only integer projection",
        semantics: "keyed_reduce",
    },
    FixturePanic => {
        source: crate::registrations::fixture_panic_source(),
        factory: FactoryArm::Actor,
        scope: FixtureLocal(&crate::registrations::FIXTURE_PANIC_MANIFEST),
        adjudication: "fixture-local: the one deliberate failure — a hook that panics at an authored arrival",
        semantics: "fixture/fixture_panic",
    },
    Request => {
        source: crate::registrations::request_source(),
        factory: FactoryArm::Actor,
        scope: Published(BoundarySink),
        adjudication: "published boundary sink: config-described HttpFetch/Http requests project each event and return bounded response facts or explicit errors",
        semantics: "request",
    },
    File => {
        source: crate::registrations::file_source(),
        factory: FactoryArm::Actor,
        scope: Published(BoundarySink),
        adjudication: "published boundary sink: one real file, serial FileRead/FileWrite, bytes content and whole-file replacement",
        semantics: "file",
    },
    Json => {
        source: crate::registrations::json_source(),
        factory: FactoryArm::Actor,
        scope: Published(BoundarySource),
        adjudication: "published boundary source: a value box driven by initial, set and bang; the static_json spelling is not published",
        semantics: "json",
    },
    Form => {
        source: crate::registrations::form_source(),
        factory: FactoryArm::Actor,
        scope: Published(BoundarySource),
        adjudication: "published boundary source: typed draft injection; the field schema is the published type-expression carrier and Mandatory gates standing, not authoring",
        semantics: "form",
    },
    Otlp => {
        source: crate::registrations::otlp_source(),
        factory: FactoryArm::Source,
        scope: Published(BoundarySource),
        adjudication: "published boundary source: OTLP/HTTP JSON logs and metrics; engine-owned loopback mount and durable ingress",
        semantics: "otlp",
    },
}

#[must_use]
pub fn derived_error_outlet(
    actor_type: ActorType,
    side: crate::ports::Side,
    port_name: &str,
) -> Option<(crate::types::Flow, crate::ports::Arity)> {
    if side != crate::ports::Side::Outlet || port_name != "_error" {
        return None;
    }
    (matches!(actor_type, ActorType::Timer | ActorType::Assemble)
        || matches!(
            registration(actor_type).spec().effect(),
            crate::capabilities::EffectDeclaration::External { .. }
        ))
    .then_some({
        (
            crate::types::Flow::Stream(crate::types::Shape::Base(crate::types::BaseShape::String)),
            crate::ports::Arity::Many,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_current_actor_type_reaches_one_complete_registration() {
        for actor_type in ActorType::ALL {
            let row = registration(actor_type);
            assert!(!row.adjudication().is_empty());
            assert!(!row.semantics_key().is_empty());
            assert_eq!(row.spec().is_source(), row.factory() == FactoryArm::Source);
        }

        let sources = ActorType::ALL
            .into_iter()
            .filter(|actor_type| registration(*actor_type).factory() == FactoryArm::Source)
            .collect::<Vec<_>>();
        assert_eq!(
            sources,
            vec![
                ActorType::FixtureInput,
                ActorType::Listener,
                ActorType::Otlp,
            ]
        );
    }

    #[test]
    fn graph_presentation_roles_are_an_explicit_closed_designation() {
        let with_role = |role| {
            ActorType::ALL
                .into_iter()
                .filter(|actor_type| registration(*actor_type).presentation() == role)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            with_role(GraphPresentationRole::SystemBoundary(
                BoundaryDirection::Source
            )),
            vec![
                ActorType::Input,
                ActorType::Timer,
                ActorType::Listener,
                ActorType::Json,
                ActorType::Otlp,
            ]
        );
        assert_eq!(
            with_role(GraphPresentationRole::SystemBoundary(
                BoundaryDirection::Sink
            )),
            vec![ActorType::Output, ActorType::Notify, ActorType::Request,]
        );
        assert_eq!(
            with_role(GraphPresentationRole::InlineOperation),
            vec![
                ActorType::Debounce,
                ActorType::Throttle,
                ActorType::Filter,
                ActorType::Dedup,
                ActorType::Map,
                ActorType::Parse,
                ActorType::Tap,
                ActorType::Ema,
                ActorType::WindowedReduce,
                ActorType::Form,
            ]
        );

        assert_eq!(
            registration(ActorType::WindowedReduce).factory(),
            FactoryArm::Actor
        );
        assert_eq!(
            registration(ActorType::WindowedReduce).presentation(),
            GraphPresentationRole::InlineOperation
        );
        assert_eq!(
            registration(ActorType::Bang).presentation(),
            GraphPresentationRole::Instrument
        );
        assert_eq!(registration(ActorType::Input).factory(), FactoryArm::Actor);
        assert_eq!(
            registration(ActorType::Input).presentation(),
            GraphPresentationRole::SystemBoundary(BoundaryDirection::Source)
        );
        assert_eq!(
            registration(ActorType::PipelineActor).presentation(),
            GraphPresentationRole::Container(ContainerCardinality::One)
        );
        assert_eq!(
            registration(ActorType::Replicator).presentation(),
            GraphPresentationRole::Container(ContainerCardinality::KeyedMany)
        );
    }

    #[test]
    fn immutable_lookup_uses_distinct_stable_cells() {
        let first = ActorType::ALL.map(get);
        let second = ActorType::ALL.map(get);
        assert!(
            first
                .iter()
                .zip(second.iter())
                .all(|(left, right)| std::ptr::eq(*left, *right))
        );
        for (index, spec) in first.iter().enumerate() {
            assert!(
                first[..index]
                    .iter()
                    .all(|other| !std::ptr::eq(*other, *spec))
            );
        }
    }

    #[test]
    fn lookup_equals_a_fresh_derivation_from_the_same_registration_row() {
        for actor_type in ActorType::ALL {
            assert_eq!(get(actor_type), &derive_registered_for_test(actor_type));
        }
    }

    #[test]
    fn the_error_outlet_derives_for_declared_failures_and_timer_schedule_failures() {
        use crate::capabilities::EffectDeclaration;
        use crate::ports::Side;

        for actor_type in ActorType::ALL {
            let needs_error = matches!(actor_type, ActorType::Timer | ActorType::Assemble)
                || matches!(
                    registration(actor_type).spec().effect(),
                    EffectDeclaration::External { .. }
                );
            assert_eq!(
                derived_error_outlet(actor_type, Side::Outlet, "_error").is_some(),
                needs_error,
                "the derivation for {actor_type:?} disagrees with the effect declaration"
            );
            assert!(
                derived_error_outlet(actor_type, Side::Inlet, "_error").is_none(),
                "a derived outlet must not appear on the inlet side"
            );
            assert!(
                derived_error_outlet(actor_type, Side::Outlet, "error").is_none(),
                "the derivation appears only on that one reserved name"
            );
        }
        assert!(
            derived_error_outlet(ActorType::Agent, Side::Outlet, "_error").is_some(),
            "agent is an external effect element, so it gets the derivation"
        );
        assert!(
            derived_error_outlet(ActorType::Tap, Side::Outlet, "_error").is_none(),
            "a derivation on an element without effects would draw a failure path that does not exist"
        );
    }
}

