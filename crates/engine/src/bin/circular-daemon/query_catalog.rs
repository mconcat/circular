//! Canonical descriptor projection for every query the product daemon serves.
//!
//! This is discovery over the existing Query/QueryResult partition, not a new
//! protocol verb.  Descriptors publish enough argument, lifecycle, paging, and
//! current-availability facts for a generic client surface to render honest
//! controls without copying this daemon's query-name switch.

use crate::daemon::query::{QueryHandler, RecordQueryHandler, handlers};
use circular_core::Value;
use circular_protocol::approval_payload::RUNTIME_APPROVALS_QUERY;
use circular_protocol::declaration_payload::{PreprocessKind, QueryPage, Terminal};
use circular_protocol::timeline::{TIMELINE_AT_QUERY, TIMELINE_BINS_QUERY};

use crate::daemon::actor_access::AUTHORING_ACTOR_ACCESS_QUERY;
use crate::daemon::actor_catalog::{
    ACTOR_CATALOG_QUERY, ACTOR_CONFIGURATION_ADMISSION_QUERY, ACTOR_CREATE_ADMISSION_QUERY,
    ACTOR_CREATE_INPUTS_QUERY, AUTHORING_ACTOR_PORTS_QUERY,
};
use crate::daemon::daemon_health::DAEMON_HEALTH_QUERY;
pub(crate) const OBSERVATION_SCAN_QUERY: &str = "observation-scan";
pub(crate) const ACTOR_EVENTS_QUERY: &str = "actor.events";
pub(crate) const QUERY_CATALOG_QUERY: &str = "query.catalog";
pub(crate) const AGENT_HARNESSES_QUERY: &str = "agent.harnesses";
pub(crate) const AGENT_HARNESS_CANDIDATES_QUERY: &str = "agent.harness-candidates";
pub(crate) const AUTHORING_SNAPSHOT_QUERY: &str = "authoring-snapshot";
pub(crate) const PIPELINES_QUERY: &str = "pipelines";
pub(crate) const TIMELINE_QUERY: &str = "timeline";
pub(crate) const PRESENTATION_QUERY: &str = "structure.presentation";
pub(crate) const ARRIVAL_SCAN_QUERY: &str = "arrival.scan";
pub(crate) const DEAD_LETTERS_QUERY: &str = "dead.letters";
pub(crate) const TRANSITION_QUERY: &str = "instance.transitions";
pub(crate) const ROLLUP_QUERY: &str = "display.rollup";

/// Stable daemon query identity for discovery and diagnostics. The same
/// registration row owns its descriptor and executable handler.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueryId {
    AgentHarnesses,
    Catalog,
    AuthoringSnapshot,
    AuthoringActorAccess,
    AuthoringActorPorts,
    ActorCatalog,
    ActorConfigurationAdmission,
    ActorCreateAdmission,
    ActorCreateInputs,
    Pipelines,
    Timeline,
    RuntimeApprovals,
    Presentation,
    ArrivalScan,
    DeadLetters,
    Transitions,
    Rollup,
    ObservationScan,
    ActorEvents,
    Records,
    DaemonHealth,
    TimelineBins,
    TimelineAt,
    AgentHarnessCandidates,
}

#[derive(Clone, Copy)]
enum ArgumentSpec {
    Null,
    Text {
        label: &'static str,
        nonempty: bool,
    },
    ScopeObject,
    RecordsObject,
    Fields(&'static [ArgumentField]),
}

#[derive(Clone, Copy)]
struct ArgumentField {
    name: &'static str,
    label: &'static str,
    kind: FieldKind,
    required: bool,
}

#[derive(Clone, Copy)]
enum FieldKind {
    UInt,
    ActorIdentity,
}

const fn field(
    name: &'static str,
    label: &'static str,
    kind: FieldKind,
    required: bool,
) -> ArgumentField {
    ArgumentField {
        name,
        label,
        kind,
        required,
    }
}

const TIMELINE_BINS_FIELDS: &[ArgumentField] = &[
    field("from_ms", "From (ms)", FieldKind::UInt, true),
    field("to_ms", "To (ms)", FieldKind::UInt, true),
    field("bins", "Bins", FieldKind::UInt, true),
    field("actor", "Actor", FieldKind::ActorIdentity, false),
];

const TIMELINE_AT_FIELDS: &[ArgumentField] = &[field("at_ms", "At (ms)", FieldKind::UInt, true)];

#[derive(Clone, Copy)]
enum LifecycleRequirement {
    None,
    StandingPipeline,
}

#[derive(Clone, Copy, Debug)]
enum PagingSpec {
    None,
    OptionalRetainedCursor,
    OptionalImmutableCursor,
}

#[derive(Clone, Copy)]
enum AvailabilityDependency {
    None,
    StandingPipeline,
    SpecializedActorConfigurator,
}

#[derive(Clone, Copy)]
struct DescriptorSpec {
    id: QueryId,
    name: &'static str,
    label: &'static str,
    argument: ArgumentSpec,
    lifecycle_requirement: LifecycleRequirement,
    paging: PagingSpec,
    availability: AvailabilityDependency,
    handler: QueryHandler,
    accepts_since: bool,
}

const DESCRIPTORS: &[DescriptorSpec] = &[
    DescriptorSpec {
        id: QueryId::ActorCatalog,
        handler: QueryHandler::Immediate(handlers::actor_catalog),
        accepts_since: false,
        name: ACTOR_CATALOG_QUERY,
        label: "Actor Catalog",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::ActorConfigurationAdmission,
        handler: QueryHandler::Immediate(handlers::actor_configuration_admission),
        accepts_since: false,
        name: ACTOR_CONFIGURATION_ADMISSION_QUERY,
        label: "Actor Configuration Admission",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::SpecializedActorConfigurator,
    },
    DescriptorSpec {
        id: QueryId::ActorCreateAdmission,
        handler: QueryHandler::Immediate(handlers::actor_create_admission),
        accepts_since: false,
        name: ACTOR_CREATE_ADMISSION_QUERY,
        label: "Actor Create Admission",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::SpecializedActorConfigurator,
    },
    DescriptorSpec {
        id: QueryId::ActorCreateInputs,
        handler: QueryHandler::Immediate(handlers::actor_create_inputs),
        accepts_since: false,
        name: ACTOR_CREATE_INPUTS_QUERY,
        label: "Actor Create Inputs",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::ActorEvents,
        handler: QueryHandler::Projection(handlers::actor_events),
        accepts_since: true,
        name: ACTOR_EVENTS_QUERY,
        label: "Actor Events",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::OptionalImmutableCursor,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::AgentHarnessCandidates,
        handler: QueryHandler::Immediate(handlers::agent_harness_candidates),
        accepts_since: false,
        name: AGENT_HARNESS_CANDIDATES_QUERY,
        label: "Agent Harness Candidates",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::AgentHarnesses,
        handler: QueryHandler::Immediate(handlers::agent_harnesses),
        accepts_since: false,
        name: AGENT_HARNESSES_QUERY,
        label: "Agent Harnesses",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::ArrivalScan,
        handler: QueryHandler::Projection(handlers::arrival_scan),
        accepts_since: true,
        name: ARRIVAL_SCAN_QUERY,
        label: "Arrival Scan",
        argument: ArgumentSpec::Text {
            label: "Mount",
            nonempty: true,
        },
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::OptionalImmutableCursor,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::AuthoringSnapshot,
        handler: QueryHandler::Retained(handlers::authoring_snapshot),
        accepts_since: false,
        name: AUTHORING_SNAPSHOT_QUERY,
        label: "Authoring Snapshot",
        argument: ArgumentSpec::ScopeObject,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::OptionalRetainedCursor,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::AuthoringActorAccess,
        handler: QueryHandler::Immediate(handlers::authoring_actor_access),
        accepts_since: false,
        name: AUTHORING_ACTOR_ACCESS_QUERY,
        label: "Authored Actor Access",
        argument: ArgumentSpec::ScopeObject,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::AuthoringActorPorts,
        handler: QueryHandler::Immediate(handlers::authoring_actor_ports),
        accepts_since: false,
        name: AUTHORING_ACTOR_PORTS_QUERY,
        label: "Authored Actor Ports",
        argument: ArgumentSpec::ScopeObject,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::DaemonHealth,
        handler: QueryHandler::Immediate(handlers::daemon_health),
        accepts_since: false,
        name: DAEMON_HEALTH_QUERY,
        label: "Daemon Health",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::DeadLetters,
        handler: QueryHandler::Projection(handlers::dead_letters),
        accepts_since: false,
        name: DEAD_LETTERS_QUERY,
        label: "Dead Letters",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::OptionalImmutableCursor,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::Rollup,
        handler: QueryHandler::Immediate(handlers::rollup),
        accepts_since: true,
        name: ROLLUP_QUERY,
        label: "Display Rollup",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::StandingPipeline,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::StandingPipeline,
    },
    DescriptorSpec {
        id: QueryId::Transitions,
        handler: QueryHandler::Projection(handlers::transitions),
        accepts_since: false,
        name: TRANSITION_QUERY,
        label: "Instance Transitions",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::OptionalImmutableCursor,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::ObservationScan,
        handler: QueryHandler::Records(RecordQueryHandler {
            admit: handlers::admit_observation_scan,
            capture: handlers::read_observations,
        }),
        accepts_since: false,
        name: OBSERVATION_SCAN_QUERY,
        label: "Restart Records",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::OptionalImmutableCursor,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::Pipelines,
        handler: QueryHandler::Immediate(handlers::pipelines),
        accepts_since: false,
        name: PIPELINES_QUERY,
        label: "Standing Pipelines",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::StandingPipeline,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::StandingPipeline,
    },
    DescriptorSpec {
        id: QueryId::Catalog,
        handler: QueryHandler::Immediate(handlers::catalog),
        accepts_since: false,
        name: QUERY_CATALOG_QUERY,
        label: "Query Catalog",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::Records,
        handler: QueryHandler::Records(RecordQueryHandler {
            admit: handlers::admit_records,
            capture: handlers::read_records,
        }),
        accepts_since: false,
        name: crate::daemon::subscription_catalog::RECORDS_TARGET,
        label: "Records",
        argument: ArgumentSpec::RecordsObject,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::OptionalImmutableCursor,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::RuntimeApprovals,
        handler: QueryHandler::Immediate(handlers::runtime_approvals),
        accepts_since: false,
        name: RUNTIME_APPROVALS_QUERY,
        label: "Runtime Approvals",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::Presentation,
        handler: QueryHandler::Immediate(handlers::presentation),
        accepts_since: false,
        name: PRESENTATION_QUERY,
        label: "Structure Presentation",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::StandingPipeline,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::StandingPipeline,
    },
    DescriptorSpec {
        id: QueryId::Timeline,
        handler: QueryHandler::Projection(handlers::timeline),
        accepts_since: true,
        name: TIMELINE_QUERY,
        label: "Timeline",
        argument: ArgumentSpec::Null,
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::OptionalImmutableCursor,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::TimelineAt,
        handler: QueryHandler::Immediate(handlers::timeline_at),
        accepts_since: false,
        name: TIMELINE_AT_QUERY,
        label: "Timeline At",
        argument: ArgumentSpec::Fields(TIMELINE_AT_FIELDS),
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
    DescriptorSpec {
        id: QueryId::TimelineBins,
        handler: QueryHandler::Immediate(handlers::timeline_bins),
        accepts_since: false,
        name: TIMELINE_BINS_QUERY,
        label: "Timeline Bins",
        argument: ArgumentSpec::Fields(TIMELINE_BINS_FIELDS),
        lifecycle_requirement: LifecycleRequirement::None,
        paging: PagingSpec::None,
        availability: AvailabilityDependency::None,
    },
];

#[derive(Clone, Copy)]
pub(crate) struct QueryCatalogState {
    pub(crate) pipeline_available: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct QueryRegistration(&'static DescriptorSpec);

impl QueryRegistration {
    pub(crate) fn id(self) -> QueryId {
        self.0.id
    }
    pub(crate) fn handler(self) -> QueryHandler {
        self.0.handler
    }
    pub(crate) fn accepts_since(self) -> bool {
        self.0.accepts_since
    }
    pub(crate) fn uses_immutable_pager(self) -> bool {
        matches!(
            self.0.handler,
            QueryHandler::Projection(_) | QueryHandler::Records(_)
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn wire_rows() -> impl Iterator<Item = (String, &'static str, String)> {
    DESCRIPTORS.iter().map(|descriptor| {
        (
            format!("{:?}", descriptor.id),
            descriptor.name,
            format!("{:?}", descriptor.paging),
        )
    })
}

pub(crate) fn registration(name: &str) -> Option<QueryRegistration> {
    DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.name == name)
        .map(QueryRegistration)
}

pub(crate) fn resolve(name: &str) -> Option<QueryId> {
    registration(name).map(QueryRegistration::id)
}

fn descriptor(id: QueryId) -> &'static DescriptorSpec {
    DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.id == id)
        .expect("one descriptor per registration identity")
}

pub(crate) fn uses_immutable_pager(id: QueryId) -> bool {
    matches!(descriptor(id).paging, PagingSpec::OptionalImmutableCursor)
}

pub(crate) fn is_immutable_record_reader(id: QueryId) -> bool {
    let descriptor = descriptor(id);
    matches!(descriptor.paging, PagingSpec::OptionalImmutableCursor)
        && descriptor.handler.record_reader().is_some()
}

pub(crate) fn catalog_page(state: QueryCatalogState) -> Result<QueryPage, String> {
    validate_descriptor_order(DESCRIPTORS)?;
    let items = DESCRIPTORS
        .iter()
        .map(|descriptor| descriptor_value(*descriptor, state))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor: Value::object([
            ("preprocess", preprocess_value()?),
            (
                "queries",
                Value::Array(
                    DESCRIPTORS
                        .iter()
                        .map(|descriptor| Value::String(descriptor.name.to_owned()))
                        .collect(),
                ),
            ),
        ])
        .map_err(|error| format!("query catalog anchor: {error:?}"))?,
        items,
        terminal: Terminal::Complete,
    })
}

fn preprocess_value() -> Result<Value, String> {
    PreprocessKind::ALL
        .into_iter()
        .map(|kind| {
            let mut slots = circular_actors::preprocess_schema::config_slots(kind);
            slots.sort_by(|(left, _), (right, _)| left.cmp(right));
            let slots = slots
                .iter()
                .map(|(path, slot)| {
                    crate::daemon::actor_catalog::create_input_slot_value(path, slot)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Value::object([
                ("kind", Value::String(kind.as_str().to_owned())),
                ("slots", Value::Array(slots)),
            ])
            .map_err(|error| format!("preprocess kind {:?}: {error:?}", kind.as_str()))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

fn validate_descriptor_order(descriptors: &[DescriptorSpec]) -> Result<(), String> {
    for pair in descriptors.windows(2) {
        if pair[0].name >= pair[1].name {
            return Err(format!(
                "query descriptors are not in unique canonical order at {:?}, {:?}",
                pair[0].name, pair[1].name
            ));
        }
    }
    Ok(())
}

fn descriptor_value(descriptor: DescriptorSpec, state: QueryCatalogState) -> Result<Value, String> {
    Value::object([
        ("argument", argument_value(descriptor.argument)?),
        (
            "availability",
            availability_value(descriptor.availability, state)?,
        ),
        ("label", Value::String(descriptor.label.to_owned())),
        ("name", Value::String(descriptor.name.to_owned())),
        ("paging", paging_value(descriptor.paging)?),
        (
            "lifecycle_requirement",
            Value::String(
                match descriptor.lifecycle_requirement {
                    LifecycleRequirement::None => "none",
                    LifecycleRequirement::StandingPipeline => "standing_pipeline",
                }
                .to_owned(),
            ),
        ),
    ])
    .map_err(|error| format!("query descriptor {:?}: {error:?}", descriptor.name))
}

fn argument_value(argument: ArgumentSpec) -> Result<Value, String> {
    match argument {
        ArgumentSpec::Null => Value::object([("kind", Value::String("null".to_owned()))]),
        ArgumentSpec::Text { label, nonempty } => Value::object([
            ("kind", Value::String("text".to_owned())),
            ("label", Value::String(label.to_owned())),
            ("nonempty", Value::Bool(nonempty)),
        ]),
        ArgumentSpec::ScopeObject | ArgumentSpec::RecordsObject => {
            let field = Value::object([
                ("kind", Value::String("scope_identity".to_owned())),
                ("label", Value::String("Scope".to_owned())),
                ("name", Value::String("scope".to_owned())),
                ("required", Value::Bool(true)),
            ])
            .map_err(|error| format!("scope argument field: {error:?}"))?;
            let mut fields = vec![field];
            if matches!(argument, ArgumentSpec::RecordsObject) {
                fields.push(
                    Value::object([
                        ("kind", Value::string("cursor")),
                        ("label", Value::string("Since")),
                        ("name", Value::string("since")),
                        ("required", Value::Bool(false)),
                    ])
                    .map_err(|error| format!("cursor argument field: {error:?}"))?,
                );
            }
            Value::object([
                ("additional_fields", Value::Bool(false)),
                ("fields", Value::Array(fields)),
                ("kind", Value::String("object".to_owned())),
            ])
        }
        ArgumentSpec::Fields(fields) => {
            let fields = fields
                .iter()
                .map(|field| {
                    Value::object([
                        ("kind", Value::string(field_kind(field.kind))),
                        ("label", Value::string(field.label)),
                        ("name", Value::string(field.name)),
                        ("required", Value::Bool(field.required)),
                    ])
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("query argument field: {error:?}"))?;
            Value::object([
                ("additional_fields", Value::Bool(false)),
                ("fields", Value::Array(fields)),
                ("kind", Value::String("object".to_owned())),
            ])
        }
    }
    .map_err(|error| format!("query argument contract: {error:?}"))
}

const fn field_kind(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::UInt => "uint",
        FieldKind::ActorIdentity => "actor_identity",
    }
}

fn paging_value(paging: PagingSpec) -> Result<Value, String> {
    match paging {
        PagingSpec::None => Value::object([("kind", Value::String("none".to_owned()))]),
        PagingSpec::OptionalRetainedCursor | PagingSpec::OptionalImmutableCursor => {
            Value::object([
                (
                    "cursor",
                    Value::string(if matches!(paging, PagingSpec::OptionalImmutableCursor) {
                        "opaque_record"
                    } else {
                        "non_negative_int"
                    }),
                ),
                ("kind", Value::String("optional_retained_cursor".to_owned())),
                ("limit", Value::String("positive_int".to_owned())),
            ])
        }
    }
    .map_err(|error| format!("query paging contract: {error:?}"))
}

fn availability_value(
    dependency: AvailabilityDependency,
    state: QueryCatalogState,
) -> Result<Value, String> {
    let unavailable = match dependency {
        AvailabilityDependency::None => None,
        AvailabilityDependency::StandingPipeline if !state.pipeline_available => {
            Some(("no_standing_pipeline", "No pipeline is standing."))
        }
        AvailabilityDependency::SpecializedActorConfigurator => Some((
            "specialized_actor_configurator_required",
            "This query accepts a registry-owned opaque config carrier and is available only through the actor configurator.",
        )),
        AvailabilityDependency::StandingPipeline => None,
    };
    match unavailable {
        None => Value::object([("kind", Value::String("available".to_owned()))]),
        Some((code, reason)) => Value::object([
            ("code", Value::String(code.to_owned())),
            ("kind", Value::String("unavailable".to_owned())),
            ("reason", Value::String(reason.to_owned())),
        ]),
    }
    .map_err(|error| format!("query availability: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(value: &Value) -> &circular_core::ObjectValue {
        let Value::Object(fields) = value else {
            panic!("descriptor is an object")
        };
        fields
    }

    fn anchor_fields(page: &QueryPage) -> &circular_core::ObjectValue {
        let Value::Object(fields) = &page.anchor else {
            panic!("catalog anchor is an object")
        };
        fields
    }

    fn full_page() -> QueryPage {
        catalog_page(QueryCatalogState {
            pipeline_available: true,
        })
        .expect("catalog")
    }

    #[test]
    fn the_preprocess_section_is_the_protocol_enumeration_itself() {
        let page = full_page();
        let Some(Value::Array(kinds)) = anchor_fields(&page).get("preprocess") else {
            panic!("catalog anchor carries a preprocess section")
        };
        let published = kinds
            .iter()
            .map(|kind| {
                let Value::Object(fields) = kind else {
                    panic!("preprocess kind is an object")
                };
                let Some(Value::String(name)) = fields.get("kind") else {
                    panic!("preprocess kind carries its wire spelling")
                };
                name.clone()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            published,
            PreprocessKind::ALL
                .iter()
                .map(|kind| kind.as_str().to_owned())
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn every_published_slot_comes_from_the_acceptor_that_owns_it() {
        let page = full_page();
        let Some(Value::Array(kinds)) = anchor_fields(&page).get("preprocess") else {
            panic!("catalog anchor carries a preprocess section")
        };
        for (value, kind) in kinds.iter().zip(PreprocessKind::ALL) {
            let Value::Object(fields) = value else {
                panic!("preprocess kind is an object")
            };
            let Some(Value::Array(slots)) = fields.get("slots") else {
                panic!("preprocess kind carries its slots")
            };
            let mut owned = circular_actors::preprocess_schema::config_slots(kind);
            owned.sort_by(|(left, _), (right, _)| left.cmp(right));
            assert_eq!(slots.len(), owned.len(), "{kind:?}");
            for (published, (path, slot)) in slots.iter().zip(&owned) {
                assert_eq!(
                    published,
                    &crate::daemon::actor_catalog::create_input_slot_value(path, slot)
                        .expect("slot"),
                    "{kind:?}"
                );
            }
        }
    }

    #[test]
    fn the_query_name_list_still_anchors_the_descriptor_items() {
        let page = full_page();
        let Some(Value::Array(names)) = anchor_fields(&page).get("queries") else {
            panic!("catalog anchor carries the query name list")
        };
        assert_eq!(names.len(), page.items.len());
        assert_eq!(anchor_fields(&page).len(), 2);
        let catalog = page
            .items
            .iter()
            .find(|item| fields(item).get("name") == Some(&Value::String("query.catalog".into())))
            .expect("the catalog describes itself");
        assert_eq!(
            fields(catalog)
                .get("paging")
                .and_then(|paging| paging.as_object())
                .and_then(|paging| paging.get("kind")),
            Some(&Value::String("none".into()))
        );
    }

    #[test]
    fn projection_descriptors_publish_the_existing_retained_cursor_contract() {
        let page = catalog_page(QueryCatalogState {
            pipeline_available: true,
        })
        .unwrap();
        let expected = Value::object([
            ("cursor", Value::String("opaque_record".into())),
            ("kind", Value::String("optional_retained_cursor".into())),
            ("limit", Value::String("positive_int".into())),
        ])
        .unwrap();
        for name in [
            "arrival.scan",
            "actor.events",
            "dead.letters",
            "instance.transitions",
            "observation-scan",
            "records",
        ] {
            let descriptor = page
                .items
                .iter()
                .find(|item| fields(item).get("name") == Some(&Value::String(name.into())))
                .unwrap();
            assert_eq!(fields(descriptor).get("paging"), Some(&expected));
        }
    }

    #[test]
    fn create_admission_is_discoverable_but_never_claims_a_generic_editor() {
        let page = catalog_page(QueryCatalogState {
            pipeline_available: true,
        })
        .expect("catalog");
        let descriptor = page
            .items
            .iter()
            .find(|item| {
                fields(item).get("name")
                    == Some(&Value::String(ACTOR_CREATE_ADMISSION_QUERY.to_owned()))
            })
            .expect("create admission descriptor");
        assert_eq!(
            fields(descriptor)
                .get("availability")
                .and_then(|value| match value {
                    Value::Object(object) => object.get("code"),
                    _ => None,
                }),
            Some(&Value::String(
                "specialized_actor_configurator_required".to_owned()
            ))
        );
        assert_eq!(
            fields(descriptor)
                .get("argument")
                .and_then(|value| match value {
                    Value::Object(object) => object.get("kind"),
                    _ => None,
                }),
            Some(&Value::String("null".to_owned())),
            "the generic surface gets no invented editable config fields"
        );
    }

    #[test]
    fn duplicate_or_unsorted_registry_rows_are_rejected() {
        let row = DESCRIPTORS[0];
        assert!(validate_descriptor_order(&[row, row]).is_err());
        assert!(validate_descriptor_order(&[DESCRIPTORS[1], DESCRIPTORS[0]]).is_err());
    }
}
