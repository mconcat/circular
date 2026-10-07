//! Registry-owned admission for config-bearing actor creation.
//!
//! This layer joins the sealed [`ConfigSchema`](crate::ConfigSchema) subset to
//! the same [`ActorSpec`](crate::ActorSpec) that expands runtime ports.  Clients
//! may display projected metadata and edit a draft, but only this producer can
//! mint an [`AdmittedActorCreate`].

use std::error::Error;
use std::fmt;

use circular_core::{ActorType, Value};
use circular_protocol::boundary_port::{
    BoundaryActorGeneration, BoundaryPortId, BoundaryPortIdError,
};
use circular_protocol::declaration_payload::PlanActorKey;
use circular_runtime::FoldedConfig;

use crate::{
    CreateInputAdmissionError, CreateInputDraft, CreateInputSchema, CreateInputUnavailable,
    PortExpansionError, PortSet, registration,
};

/// Registration-authored draft containing only canonical optional defaults.
pub fn registered_create_draft(
    actor_type: ActorType,
) -> Result<CreateInputDraft, RegisteredCreateAdmissionError> {
    Ok(registered_create_inputs(actor_type)?.draft())
}

/// Complete create-input contract for one registration. An incomplete
/// ConfigSchema remains unavailable with its exact producer stop line.
pub fn registered_create_inputs(
    actor_type: ActorType,
) -> Result<CreateInputSchema<'static>, RegisteredCreateAdmissionError> {
    registration(actor_type)
        .spec()
        .config()
        .create_inputs()
        .map_err(RegisteredCreateAdmissionError::Unavailable)
}

/// Admit config and derive the complete port identity set from the same
/// registration. Dynamic ports are never partially projected from a draft.
pub fn admit_registered_create(
    actor_type: ActorType,
    config: &Value,
) -> Result<AdmittedActorCreate, RegisteredCreateAdmissionError> {
    admit_registered_create_inner(actor_type, config, None)
}

/// Admit config against one exact prospective authored identity.
///
/// Ordinary registrations preserve the fence for the eventual placement.
/// Pipeline-boundary registrations additionally derive their reserved port id
/// from this key; they cannot be admitted through the identity-free entrypoint.
pub fn admit_registered_create_at(
    actor_type: ActorType,
    config: &Value,
    actor: &PlanActorKey,
    generation: BoundaryActorGeneration,
) -> Result<AdmittedActorCreate, RegisteredCreateAdmissionError> {
    admit_registered_create_inner(actor_type, config, Some((actor, generation)))
}

fn admit_registered_create_inner(
    actor_type: ActorType,
    config: &Value,
    actor: Option<(&PlanActorKey, BoundaryActorGeneration)>,
) -> Result<AdmittedActorCreate, RegisteredCreateAdmissionError> {
    let spec = registration(actor_type).spec();
    let admitted = spec
        .config()
        .create_inputs()
        .map_err(RegisteredCreateAdmissionError::Unavailable)?
        .admit(config)
        .map_err(RegisteredCreateAdmissionError::Config)?;
    crate::approval_config::admit_beyond_schema(actor_type, admitted.value())?;
    crate::capability_config::validate(actor_type, admitted.value())
        .map_err(RegisteredCreateAdmissionError::Config)?;
    let folded = FoldedConfig::minted(actor_type, admitted.into_value());
    let ports = match spec.boundary() {
        Some(boundary) => {
            let (actor, generation) =
                actor.ok_or(RegisteredCreateAdmissionError::BoundaryIdentityRequired)?;
            let id = BoundaryPortId::derive(boundary.direction(), actor, generation)
                .map_err(RegisteredCreateAdmissionError::BoundaryIdentity)?;
            spec.expand_ports_at(&folded, Some(id.into_port_id()))
                .map_err(RegisteredCreateAdmissionError::Ports)?
        }
        None => spec
            .expand_ports(&folded)
            .map_err(RegisteredCreateAdmissionError::Ports)?,
    };
    Ok(AdmittedActorCreate {
        config: folded,
        ports,
        authored_actor: actor.map(|(actor, _)| actor.clone()),
    })
}

/// Value and complete ports that passed the registration's sealed admission.
#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedActorCreate {
    config: FoldedConfig,
    ports: PortSet,
    authored_actor: Option<PlanActorKey>,
}

impl AdmittedActorCreate {
    #[must_use]
    pub const fn actor_type(&self) -> ActorType {
        self.config.actor_type()
    }

    #[must_use]
    pub const fn config(&self) -> &Value {
        self.config.value()
    }

    /// The same actor-typed value used to derive these ports. Activation still
    /// needs its own grants and resolved inlets; authoring admission does not
    /// prove that those later inputs are valid.
    #[must_use]
    pub const fn folded_config(&self) -> &FoldedConfig {
        &self.config
    }

    #[must_use]
    pub const fn ports(&self) -> &PortSet {
        &self.ports
    }

    #[must_use]
    pub const fn authored_actor(&self) -> Option<&PlanActorKey> {
        self.authored_actor.as_ref()
    }

    #[must_use]
    pub fn into_parts(self) -> (ActorType, Value, PortSet, Option<PlanActorKey>) {
        (
            self.config.actor_type(),
            self.config.into_value(),
            self.ports,
            self.authored_actor,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RegisteredCreateAdmissionError {
    Unavailable(CreateInputUnavailable),
    Config(CreateInputAdmissionError),
    Ports(PortExpansionError),
    BoundaryIdentityRequired,
    BoundaryIdentity(BoundaryPortIdError),
}

impl fmt::Display for RegisteredCreateAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(error) => error.fmt(formatter),
            Self::Config(error) => error.fmt(formatter),
            Self::Ports(error) => error.fmt(formatter),
            Self::BoundaryIdentityRequired => formatter.write_str(
                "boundary actor creation requires an exact prospective authored actor identity",
            ),
            Self::BoundaryIdentity(error) => error.fmt(formatter),
        }
    }
}

impl Error for RegisteredCreateAdmissionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConfigPath;

    fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        Value::object(entries).expect("unique test fields")
    }

    fn with_capabilities(kind: ActorType, config: Value) -> Value {
        let names: &[&str] = match kind {
            ActorType::ToolExecutor => &["FsRead", "FsWrite", "ProcessSpawn"],
            ActorType::File => &["FsRead", "FsWrite"],
            ActorType::Listener => &["FsRead"],
            ActorType::Request => &["HttpFetch"],
            ActorType::Notify => &["UserNotify"],
            _ => return config,
        };
        let fields = config.as_object().unwrap().clone().into_map();
        let parent = |path: &str| {
            std::path::Path::new(path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(|parent| parent.to_string_lossy().into_owned())
        };
        let mut roots = Vec::new();
        if let Some(path) = fields.get("path").and_then(Value::as_str) {
            roots.extend(parent(path));
        }
        if let Some(tools) = fields.get("tools").and_then(Value::as_object) {
            for (_, tool) in tools.iter() {
                if let Some(path) = tool
                    .as_object()
                    .and_then(|tool| tool.get("path"))
                    .and_then(Value::as_str)
                {
                    roots.extend(parent(path));
                }
            }
        }
        roots.sort();
        roots.dedup();
        if roots.is_empty() {
            roots.push(format!("/test-only/{}", kind.as_str()));
        }
        let mut fields = fields;
        fields.insert(
            "capabilities".into(),
            Value::object(names.iter().map(|name| {
                let mut policy = vec![("approval", Value::string("none"))];
                if matches!(*name, "FsRead" | "FsWrite") {
                    policy.push((
                        "roots",
                        Value::array(roots.iter().map(|root| Value::string(root.clone()))),
                    ));
                }
                (*name, object(policy))
            }))
            .unwrap(),
        );
        Value::object(fields).unwrap()
    }

    fn actor_key(local: &str) -> PlanActorKey {
        PlanActorKey {
            scope: Vec::new(),
            local: circular_protocol::declaration_payload::AuthoredLocal::try_new(local)
                .expect("test actor local is authored")
                .into(),
        }
    }

    #[test]
    fn map_and_filter_publish_missing_mandatory_snippet_drafts() {
        let map = registered_create_draft(ActorType::Map).expect("map contract");
        assert_eq!(map.config(), &object([]));
        assert_eq!(map.missing().len(), 1);

        let filter = registered_create_draft(ActorType::Filter).expect("filter contract");
        assert_eq!(filter.config(), &object([]));
        assert_eq!(filter.missing().len(), 1);
    }

    #[test]
    fn canonical_snippet_parser_and_port_expansion_mint_the_template() {
        let map = admit_registered_create(
            ActorType::Map,
            &object([("transform", Value::String("event".to_owned()))]),
        )
        .expect("map config and dynamic event outlet");
        assert_eq!(map.ports().inlets()[0].id().as_str(), "event");
        assert_eq!(map.ports().outlets()[0].id().as_str(), "event");

        let filter = admit_registered_create(
            ActorType::Filter,
            &object([("predicate", Value::String("true".to_owned()))]),
        )
        .expect("filter predicate");
        assert_eq!(filter.ports().inlets()[0].id().as_str(), "event");
        assert_eq!(filter.ports().outlets()[0].id().as_str(), "event");
    }

    #[test]
    fn route_relation_and_dynamic_port_naming_are_producer_admitted() {
        let route = admit_registered_create(
            ActorType::Route,
            &object([
                ("at", Value::Array(Vec::new())),
                (
                    "cases",
                    object([
                        ("accepted", Value::String("ok".to_owned())),
                        ("rejected", Value::String("no".to_owned())),
                    ]),
                ),
            ]),
        )
        .expect("route contract");
        let outputs = route
            .ports()
            .outlets()
            .iter()
            .map(|port| port.id().as_str())
            .collect::<Vec<_>>();
        assert_eq!(outputs, ["unmatched", "route_accepted", "route_rejected"]);

        let duplicate = object([
            ("at", Value::Array(Vec::new())),
            (
                "cases",
                object([
                    ("one", Value::String("same".to_owned())),
                    ("two", Value::String("same".to_owned())),
                ]),
            ),
        ]);
        assert!(matches!(
            admit_registered_create(ActorType::Route, &duplicate),
            Err(RegisteredCreateAdmissionError::Config(
                CreateInputAdmissionError::DuplicateObjectValue { .. }
            ))
        ));
    }

    #[test]
    fn incomplete_registration_never_promotes_from_visible_slots() {
        let error = registered_create_draft(ActorType::TokenCostMeter)
            .expect_err("token_cost_meter remains incomplete");
        assert!(matches!(
            error,
            RegisteredCreateAdmissionError::Unavailable(_)
        ));
    }

    #[test]
    fn boundary_create_requires_exact_identity_and_uses_stream_any() {
        let draft = registered_create_draft(ActorType::Input).expect("sealed input contract");
        assert_eq!(
            draft.missing(),
            &[circular_expr::ConfigPath::root().join_key("label")]
        );
        let config = object([("label", Value::String("Request".to_owned()))]);
        assert!(matches!(
            admit_registered_create(ActorType::Input, &config),
            Err(RegisteredCreateAdmissionError::BoundaryIdentityRequired)
        ));
        let actor = actor_key("request-input");
        let admitted = admit_registered_create_at(
            ActorType::Input,
            &config,
            &actor,
            BoundaryActorGeneration::initial(),
        )
        .expect("exact boundary admission");
        assert_eq!(admitted.authored_actor(), Some(&actor));
        assert!(admitted.ports().inlets().is_empty());
        let [outlet] = admitted.ports().outlets() else {
            panic!("input boundary has one local outlet")
        };
        assert!(outlet.id().as_str().starts_with("_bi1_"));
        assert_eq!(outlet.ty(), &crate::Flow::Stream(crate::Shape::Any));
        assert_eq!(admitted.config(), &config);

        let output = admit_registered_create_at(
            ActorType::Output,
            &config,
            &actor_key("result-output"),
            BoundaryActorGeneration::initial(),
        )
        .expect("exact output admission");
        assert_eq!(output.ports().inlets().len(), 1);
        assert!(
            output.ports().inlets()[0]
                .id()
                .as_str()
                .starts_with("_bo1_")
        );
        assert!(output.ports().outlets().is_empty());
    }

    #[test]
    fn file_create_requires_path_and_its_grants_and_derives_both_directions() {
        let draft = registered_create_draft(ActorType::File).expect("file contract");
        assert_eq!(
            draft.missing(),
            &[
                circular_expr::ConfigPath::root().join_key("capabilities"),
                circular_expr::ConfigPath::root().join_key("path"),
            ]
        );
        assert_eq!(draft.config(), &object([]));
        assert!(admit_registered_create(ActorType::File, &object([])).is_err());
        assert!(
            admit_registered_create(ActorType::File, &object([("path", Value::Int(1))])).is_err()
        );
        assert_eq!(
            admit_registered_create(
                ActorType::File,
                &object([("path", Value::String("/tmp/file".into()))])
            )
            .unwrap_err(),
            RegisteredCreateAdmissionError::Config(CreateInputAdmissionError::MissingMandatory(
                circular_expr::ConfigPath::root().join_key("capabilities")
            ))
        );
        let config = object([("path", Value::String("/tmp/file".into()))]);
        let config = with_capabilities(ActorType::File, config);
        let admitted = admit_registered_create(ActorType::File, &config).expect("file admission");
        assert_eq!(admitted.config(), &config);
        assert_eq!(
            admitted
                .ports()
                .inlets()
                .iter()
                .map(|port| port.id().as_str())
                .collect::<Vec<_>>(),
            vec!["write", "read"]
        );
        assert_eq!(
            admitted
                .ports()
                .outlets()
                .iter()
                .map(|port| port.id().as_str())
                .collect::<Vec<_>>(),
            vec!["content", "written"]
        );
        for key in ["encoding", "mode", "max_bytes", "flush_period"] {
            assert!(
                admit_registered_create(
                    ActorType::File,
                    &object([
                        ("path", Value::String("/tmp/file".into())),
                        (key, Value::Int(1))
                    ])
                )
                .is_err()
            );
        }
    }

    #[test]
    fn peer_literal_passes_create_admission_and_zero_capacity_is_rejected() {
        let config = |capacity| {
            object([
                ("adapter", Value::string("x")),
                ("realm", Value::string("r")),
                ("name", Value::string("n")),
                (
                    "inbound_policy",
                    object([("any_known_peer", Value::Bool(true))]),
                ),
                ("inbox_capacity", Value::Int(capacity)),
            ])
        };
        let literal = config(8);
        let admitted = admit_registered_create(ActorType::Peer, &literal)
            .expect("peer literal passes the existing slot contract");
        assert_eq!(admitted.config(), &literal);
        outside_space(ActorType::Peer, config(0));
    }

    #[test]
    fn listener_transcript_tail_literal_passes_create_admission() {
        let config = object([(
            "source",
            object([
                ("kind", Value::string("file_tail")),
                (
                    "value",
                    object([
                        ("glob", Value::string("~/.claude/projects/**/*.jsonl")),
                        ("poll", Value::int(500)),
                    ]),
                ),
            ]),
        )]);
        let config = with_capabilities(ActorType::Listener, config);
        let admitted = admit_registered_create(ActorType::Listener, &config)
            .expect("listener literal passes the existing nested object slots");
        assert_eq!(admitted.config(), &config);
    }

    #[test]
    fn listener_unknown_kind_passes_create_but_activation_decoder_rejects() {
        let config = object([(
            "source",
            object([
                ("kind", Value::string("other")),
                (
                    "value",
                    object([
                        ("glob", Value::string("~/.claude/projects/**/*.jsonl")),
                        ("poll", Value::int(500)),
                    ]),
                ),
            ]),
        )]);
        let config = with_capabilities(ActorType::Listener, config);
        let admitted = admit_registered_create(ActorType::Listener, &config)
            .expect("admission leaves the kind closed set to the activation decoder");
        assert_eq!(admitted.config(), &config);
        assert_eq!(
            crate::listener::ListenerConfig::from_value(admitted.config()),
            Err(crate::listener::ListenerConfigError::UnknownKind)
        );
    }

    #[test]
    fn shipped_request_empty_headers_config_passes_create_admission() {
        let config = object([
            ("method", Value::String("get".to_owned())),
            ("url", Value::String("https://example.test/x".to_owned())),
            ("headers", Value::Array(vec![])),
        ]);
        let config = with_capabilities(ActorType::Request, config);
        let admitted = admit_registered_create(ActorType::Request, &config)
            .expect("shipped request empty headers pass slot admission");
        assert_eq!(admitted.config(), &config);
    }

    #[test]
    fn shipped_parse_config_passes_create_admission() {
        let config = object([
            ("decoder", Value::String("json".to_owned())),
            ("field", Value::String("body".to_owned())),
            ("arguments", object([])),
        ]);
        let admitted = admit_registered_create(ActorType::Parse, &config)
            .expect("shipped JSON parse config passes slot admission");
        assert_eq!(admitted.config(), &config);
    }

    #[test]
    fn shipped_tool_executor_config_passes_create_admission() {
        let config = object([(
            "tools",
            object([(
                "remediate",
                object([
                    ("effect", Value::String("spawn".to_owned())),
                    ("program", Value::String("/tmp/remediate".to_owned())),
                    ("arguments", Value::Array(vec![])),
                ]),
            )]),
        )]);
        let config = with_capabilities(ActorType::ToolExecutor, config);
        let admitted = admit_registered_create(ActorType::ToolExecutor, &config)
            .expect("shipped nested spawn tool config passes slot admission");
        assert_eq!(admitted.config(), &config);
    }

    #[test]
    fn registrations_preserve_their_existing_producer_unary_facts() {
        let constraint = |actor_type, key: &'static str| {
            registration(actor_type)
                .spec()
                .config()
                .get(&crate::ConfigPath::root().join_key(key))
                .unwrap_or_else(|| panic!("{actor_type:?} omitted {key:?}"))
                .space()
                .constraint()
        };

        for (actor_type, key) in [
            (ActorType::Agent, "queue_capacity"),
            (ActorType::Peer, "inbox_capacity"),
        ] {
            let Some(crate::ConfigConstraint::IntegerCount(count)) = constraint(actor_type, key)
            else {
                panic!("{actor_type:?}.{key} must preserve its positive count fact")
            };
            assert_eq!(count.minimum(), crate::IntegerMinimum::One);
            assert_eq!(count.maximum(), None);
        }

        for (actor_type, key, expected) in [(ActorType::Cli, "mode", ["coprocess", "request"])] {
            let Some(crate::ConfigConstraint::ClosedTags(tags)) = constraint(actor_type, key)
            else {
                panic!("{actor_type:?}.{key} must preserve its closed tag set")
            };
            assert_eq!(tags.iter().collect::<Vec<_>>(), expected);
        }

        assert!(matches!(
            constraint(ActorType::TokenCostMeter, "at"),
            Some(crate::ConfigConstraint::ExactPayloadPath)
        ));
    }

    fn outside_space(actor_type: ActorType, config: Value) {
        assert!(matches!(
            admit_registered_create(actor_type, &config),
            Err(RegisteredCreateAdmissionError::Config(
                CreateInputAdmissionError::OutsideSpace { .. }
            ))
        ));
    }

    #[test]
    fn agent_create_defaults_and_existing_daemon_config_are_admitted() {
        let draft = registered_create_draft(ActorType::Agent).expect("agent create contract");
        assert_eq!(
            draft.config(),
            &object([
                ("queue_capacity", Value::Int(8)),
                ("tools", Value::Array(Vec::new())),
            ])
        );
        assert_eq!(
            draft.missing(),
            &[
                crate::ConfigPath::root().join_key("harness"),
                crate::ConfigPath::root().join_key("queue_capacity"),
                crate::ConfigPath::root().join_key("result"),
            ]
        );
        let daemon_config = object([
            ("harness", Value::String("reference".to_owned())),
            ("queue_capacity", Value::Int(2)),
            ("result", Value::String("bytes".to_owned())),
            ("tools", Value::Array(Vec::new())),
        ]);
        let admitted = admit_registered_create(ActorType::Agent, &daemon_config)
            .expect("omitted approval has the semantic none default");
        assert_eq!(admitted.config(), &daemon_config);
        assert_eq!(
            admitted
                .ports()
                .inlets()
                .iter()
                .map(|p| p.id().as_str())
                .collect::<Vec<_>>(),
            ["turn", "tool_result"]
        );
        assert_eq!(
            admitted
                .ports()
                .outlets()
                .iter()
                .map(|p| p.id().as_str())
                .collect::<Vec<_>>(),
            ["record", "tool_request", "result"]
        );
        let minimal = object([
            ("harness", Value::String("reference".to_owned())),
            ("queue_capacity", Value::Int(2)),
            ("result", Value::String("bytes".to_owned())),
        ]);
        let defaulted = admit_registered_create(ActorType::Agent, &minimal)
            .expect("tools has a producer default");
        assert_eq!(defaulted.config(), &daemon_config);
        assert_eq!(defaulted.ports(), admitted.ports());
    }

    #[test]
    fn agent_create_checks_harness_shape_and_positive_queue_capacity() {
        for (harness, capacity, rejected_key) in [
            (Value::String(String::new()), Value::Int(2), "harness"),
            (Value::Null, Value::Int(2), "harness"),
            (
                Value::String("unregistered-harness".into()),
                Value::Int(0),
                "queue_capacity",
            ),
            (
                Value::String("unregistered-harness".into()),
                Value::Int(-1),
                "queue_capacity",
            ),
            (
                Value::String("unregistered-harness".into()),
                Value::float(2.0),
                "queue_capacity",
            ),
        ] {
            assert!(matches!(
                admit_registered_create(
                    ActorType::Agent,
                    &object([("harness", harness), ("queue_capacity", capacity),])
                ),
                Err(RegisteredCreateAdmissionError::Config(
                    CreateInputAdmissionError::OutsideSpace { path, .. }
                )) if path == crate::ConfigPath::root().join_key(rejected_key)
            ));
        }
        for missing in ["harness", "queue_capacity"] {
            let config = object(
                [
                    ("harness", Value::String("unregistered-harness".into())),
                    ("queue_capacity", Value::Int(1)),
                ]
                .into_iter()
                .filter(|(key, _)| *key != missing),
            );
            assert_eq!(
                admit_registered_create(ActorType::Agent, &config),
                Err(RegisteredCreateAdmissionError::Config(
                    CreateInputAdmissionError::MissingMandatory(
                        crate::ConfigPath::root().join_key(missing)
                    )
                ))
            );
        }
    }

    /// `tools` stays an `Any` slot whose shape the factory judges; `fallback` is no
    /// longer a key of the agent registration, so a config that still carries it is refused.
    #[test]
    fn agent_create_defers_tools_validation_and_refuses_fallback() {
        let base = |extra: Vec<(&'static str, Value)>| {
            let mut entries = vec![
                ("harness", Value::String("unregistered-harness".into())),
                ("queue_capacity", Value::Int(1)),
                ("result", Value::String("json".into())),
            ];
            entries.extend(extra);
            object(entries)
        };
        for value in [
            Value::Null,
            Value::Bool(true),
            Value::Int(7),
            Value::String("deferred".into()),
            object([]),
            Value::Array(vec![
                Value::Null,
                Value::Int(3),
                Value::String("not a tool object".into()),
            ]),
        ] {
            let config = base(vec![("tools", value)]);
            assert_eq!(
                admit_registered_create(ActorType::Agent, &config)
                    .expect("deferred fields must remain unchecked")
                    .config(),
                &config
            );
        }
        assert_eq!(
            admit_registered_create(ActorType::Agent, &base(vec![("fallback", Value::Null)]))
                .map(drop),
            Err(RegisteredCreateAdmissionError::Config(
                CreateInputAdmissionError::UnknownField("fallback".to_owned())
            ))
        );
    }

    #[test]
    fn alert_delays_reject_zero_at_registered_admission() {
        for (firing, recovery) in [(0, 1), (1, 0)] {
            outside_space(
                ActorType::Alert,
                object([
                    ("predicate", Value::String("true".to_owned())),
                    ("firing_delay", Value::Int(firing)),
                    ("recovery_delay", Value::Int(recovery)),
                ]),
            );
        }
    }

    #[test]
    fn control_and_selection_contracts_execute_unary_constraints_and_snippet_admission() {
        admit_registered_create(
            ActorType::Debounce,
            &object([("quiet_window", Value::Int(0))]),
        )
        .expect("zero debounce interval is the immediate canonical boundary");
        outside_space(
            ActorType::Debounce,
            object([("quiet_window", Value::Int(-1))]),
        );
        outside_space(
            ActorType::Throttle,
            object([("minimum_interval", Value::Int(-1))]),
        );

        admit_registered_create(
            ActorType::Alert,
            &object([
                ("predicate", Value::String("true".to_owned())),
                ("firing_delay", Value::Int(1)),
                ("recovery_delay", Value::Int(1)),
            ]),
        )
        .expect("predicate and two millisecond intervals are complete");
        assert!(matches!(
            admit_registered_create(
                ActorType::Alert,
                &object([
                    ("predicate", Value::String("(".to_owned())),
                    ("firing_delay", Value::Int(1)),
                    ("recovery_delay", Value::Int(1)),
                ]),
            ),
            Err(RegisteredCreateAdmissionError::Config(
                CreateInputAdmissionError::Snippet { .. }
            ))
        ));

        let dedup = registered_create_draft(ActorType::Dedup).expect("dedup draft");
        assert_eq!(dedup.config(), &object([("at", Value::Array(Vec::new()))]));
        admit_registered_create(ActorType::Dedup, &object([("window", Value::Int(0))]))
            .expect("whole-payload default and immediate window");
        outside_space(
            ActorType::Dedup,
            object([
                ("at", Value::Array(vec![Value::Bool(true)])),
                ("window", Value::Int(1)),
            ]),
        );
    }

    #[test]
    fn timer_sealed_registration_keeps_t1_ports_and_interval_literal() {
        let admitted =
            admit_registered_create(ActorType::Timer, &object([("every", Value::Int(100))]))
                .expect("sealed timer config");
        assert_eq!(admitted.config(), &object([("every", Value::Int(100))]));
        let [bang] = admitted.ports().inlets() else {
            panic!("one authored inlet")
        };
        assert_eq!(bang.id().as_str(), "bang");
        assert_eq!(bang.ty(), &crate::Flow::Stream(crate::Shape::Any));
        assert_eq!(bang.presence(), &crate::Presence::Optional);
        assert!(bang.primary());
        let [tick] = admitted.ports().outlets() else {
            panic!("one authored outlet")
        };
        assert_eq!(tick.id().as_str(), "tick");
        assert_eq!(
            tick.ty(),
            &crate::Flow::Stream(crate::Shape::Object {
                fields: crate::FieldMap::try_new(vec![(
                    crate::Name::from_static("sequence"),
                    crate::Shape::Base(crate::BaseShape::UInt)
                )])
                .expect("one field"),
                open: false,
            })
        );
        assert!(tick.primary());
        for config in [
            object([]),
            object([("every", Value::Int(0))]),
            object([("every", Value::Int(100)), ("phase", Value::Int(1))]),
        ] {
            assert!(admit_registered_create(ActorType::Timer, &config).is_err());
        }
    }

    #[test]
    fn json_requires_initial_and_keeps_bang_optional_primary_and_any() {
        assert!(admit_registered_create(ActorType::Json, &Value::Null).is_err());
        assert!(admit_registered_create(ActorType::Json, &object([])).is_err());
        let admitted =
            admit_registered_create(ActorType::Json, &object([("initial", Value::Null)]))
                .expect("initial accepts Null as a value, distinct from a missing slot");
        assert_eq!(admitted.config(), &object([("initial", Value::Null)]));
        let inlets = admitted.ports().inlets();
        assert_eq!(inlets.len(), 2);
        assert_eq!(inlets[0].id().as_str(), "set");
        assert_eq!(inlets[0].ty(), &crate::Flow::Stream(crate::Shape::Any));
        assert_eq!(inlets[0].presence(), &crate::Presence::Required);
        assert!(!inlets[0].primary());
        assert_eq!(inlets[1].id().as_str(), "bang");
        assert_eq!(inlets[1].ty(), &crate::Flow::Stream(crate::Shape::Any));
        assert_eq!(inlets[1].presence(), &crate::Presence::Optional);
        assert!(inlets[1].primary());
        let outlets = admitted.ports().outlets();
        assert_eq!(outlets.len(), 1);
        assert_eq!(outlets[0].id().as_str(), "value");
        assert_eq!(outlets[0].ty(), &crate::Flow::Stream(crate::Shape::Any));
        assert!(outlets[0].primary());
    }

    #[test]
    fn safe_source_and_transform_contracts_preserve_values_snippets_and_fixed_ports() {
        let json = admit_registered_create(
            ActorType::Json,
            &object([(
                "initial",
                Value::Array(vec![Value::Null, Value::UInt(u64::MAX)]),
            )]),
        )
        .expect("json accepts every canonical Value as its initial payload");
        assert_eq!(
            json.ports()
                .inlets()
                .iter()
                .map(|port| port.id().as_str())
                .collect::<Vec<_>>(),
            ["set", "bang"]
        );
        assert_eq!(
            json.ports()
                .outlets()
                .iter()
                .map(|port| port.id().as_str())
                .collect::<Vec<_>>(),
            ["value"]
        );
    }
}
