//! Read-only projections of the daemon-owned actor registration table.
//!
//! No frontend crate links `circular-actors`.  The daemon publishes only facts
//! its current registry can prove: fixed ports and zero-config
//! create templates.  Dynamic ports and the incomplete config-schema frame stay
//! explicitly unavailable instead of being approximated.

use circular_actors::{
    ActorType, AdmittedActorCreate, BoundaryDirection, ClosedTags, ConfigConstraint, ConfigSlot,
    ContainerCardinality, CreateInputAdmissionError, CreateInputRelation, GraphPresentationRole,
    IntegerMinimum, IntervalDomain, RateExpr, RegisteredCreateAdmissionError, RegistrationScope,
    Required, Side, TextDomain, admit_registered_create, admit_registered_create_at,
    derived_error_outlet, registered_create_draft, registered_create_inputs, registration,
};
use circular_core::spelling::Quoted;
use circular_core::{Boundary, Ceilings, Value, encode};
use circular_protocol::Partition;
use circular_protocol::authoring_snapshot::{current_revision_value, scope_identity_value};
use circular_protocol::boundary_port::{decode_boundary_actor_key, encode_boundary_actor_key};
use circular_protocol::declaration_payload::decode_scope_identity;
use circular_protocol::declaration_payload::{QueryPage, Rejected, Terminal};
use circular_protocol::port_type::{
    PortFlow, PortRate, PortShape, encode_port_flow, encode_port_shape,
};
use circular_protocol::rejection_code::RejectionReason;

use engine::authoring_assembly::fold::{
    AuthoringActorPortFact, AuthoringPortFact, AuthoringPortFlowFact,
};
use engine::authoring_assembly::ledger::AuthoringState;
use engine::authoring_assembly::verb::ContentVerb;

pub(crate) const ACTOR_CATALOG_QUERY: &str = "actor.catalog";
pub(crate) const ACTOR_CONFIGURATION_ADMISSION_QUERY: &str = "actor.configure-admission";
pub(crate) const ACTOR_CREATE_ADMISSION_QUERY: &str = "actor.create-admission";
pub(crate) const ACTOR_CREATE_INPUTS_QUERY: &str = "actor.create-inputs";
pub(crate) const AUTHORING_ACTOR_PORTS_QUERY: &str = "authoring.actor-ports";

const CONFIG_SCHEMA_STOP_LINE: &str = "registry ConfigSchema is an incomplete frame: no complete payload admission/default projection is published";
const DYNAMIC_PORT_STOP_LINE: &str = "config-dependent ports require the authored config fold through admission; the static catalog publishes registered fixed ports only";

/// Admit one completed draft through the canonical registry. An empty schema's
/// draft is its canonical `Null`, admitted by the same registry call.
///
/// The accepted page is deliberately single-item and request-anchored. A
/// semantic admission refusal uses QueryResult's typed rejection envelope;
/// transport and physical-contract failures remain separate at the client.
pub(crate) fn create_admission_page(
    args: Value,
    authoring: &AuthoringState,
) -> Result<QueryPage, Rejected> {
    let Value::Object(object) = args else {
        return Err(malformed(
            "actor.create-admission args are not an object",
            None,
        ));
    };
    let mut fields = object.into_map();
    let actor_type = match fields.remove("actor_type") {
        Some(Value::String(actor_type)) if !actor_type.is_empty() => actor_type,
        Some(_) => {
            return Err(malformed(
                "actor.create-admission actor_type is not a nonempty String",
                Some(Value::String("actor_type".to_owned())),
            ));
        }
        None => {
            return Err(malformed(
                "actor.create-admission has no actor_type",
                Some(Value::String("actor_type".to_owned())),
            ));
        }
    };
    let config = match fields.remove("config") {
        Some(config) => config,
        None => {
            return Err(malformed(
                "actor.create-admission has no config",
                Some(Value::String("config".to_owned())),
            ));
        }
    };
    let authored_actor = match fields.remove("authored_actor") {
        Some(Value::Null) | None => None,
        Some(value) => Some(decode_boundary_actor_key(value).map_err(|error| {
            malformed(
                format!("actor.create-admission authored_actor: {error}"),
                Some(Value::String("authored_actor".to_owned())),
            )
        })?),
    };
    if let Some((unknown, _)) = fields.into_iter().next() {
        return Err(malformed(
            format!(
                "actor.create-admission carries unknown field {}",
                Quoted(&unknown)
            ),
            Some(Value::String(unknown)),
        ));
    }

    let Some(kind) = ActorType::from_str(&actor_type) else {
        return Err(unresolved(
            format!("no registered actor type {}", Quoted(&actor_type)),
            Some("Refresh the daemon-owned actor catalog."),
            Some(Value::String(actor_type)),
        ));
    };
    let registration = registration(kind);
    if !matches!(registration.scope(), RegistrationScope::Published) {
        return Err(unresolved(
            format!(
                "actor type {} is not published in the product catalog",
                Quoted(&actor_type)
            ),
            Some("Choose a published actor catalog row."),
            Some(Value::String(actor_type)),
        ));
    }
    if engine::authoring_assembly::fold::binds_template(kind, &config) {
        return template_create_page(kind, config, authored_actor, authoring);
    }
    if let Some(reason) = structural_create_requirement(kind) {
        return Err(unresolved(
            reason,
            Some("Declare the container with its paired authored scope."),
            Some(Value::String(actor_type)),
        ));
    }

    let admitted = match authored_actor.as_ref() {
        Some(actor) => admit_registered_create_at(
            kind,
            &config,
            actor,
            authoring.current().actor_generation(actor),
        ),
        None => admit_registered_create(kind, &config),
    }
    .map_err(|error| {
        admission_rejection(
            error,
            &authored_actor
                .as_ref()
                .map_or_else(|| "<not yet named>".to_owned(), ToString::to_string),
            &config,
        )
    })?;
    admitted_create_page(admitted)
}

fn template_create_page(
    kind: ActorType,
    config: Value,
    authored_actor: Option<circular_protocol::declaration_payload::PlanActorKey>,
    authoring: &AuthoringState,
) -> Result<QueryPage, Rejected> {
    use circular_protocol::declaration_payload::{ActorDeclaration, ActorFlags};
    let actor = authored_actor.as_ref().ok_or_else(|| {
        unresolved(
            "template container admission requires authored_actor",
            None,
            None,
        )
    })?;
    let mut candidate = authoring.current().clone();
    candidate
        .apply(&ContentVerb::UpsertActor {
            actor: actor.clone(),
            declaration: ActorDeclaration {
                actor_type: kind.as_str().into(),
                config: config.clone(),
                flags: ActorFlags {
                    bypass: false,
                    mute: false,
                    pause: false,
                },
            },
        })
        .map_err(|error| config_rejected(&actor.to_string(), &config, error))?;
    let plan = candidate
        .assemble()
        .map_err(|error| config_rejected(&actor.to_string(), &config, error))?;
    let mut container_scope = actor.scope.clone();
    container_scope.push(circular_protocol::declaration_payload::ScopeSegment::Child(
        actor.local.as_str().to_owned(),
    ));
    if let Some(refused) = plan
        .refused()
        .values()
        .find(|refused| refused.key() == actor || refused.key().scope.starts_with(&container_scope))
    {
        return Err(config_rejected(
            &actor.to_string(),
            &config,
            refused.rejection().clone(),
        ));
    }
    engine::validate_published_plan(&plan)
        .map_err(|error| config_rejected(&actor.to_string(), &config, error))?;
    let ports = candidate
        .actor_ports(&actor.scope)
        .map_err(|error| config_rejected(&actor.to_string(), &config, error))?
        .into_iter()
        .find(|fact| fact.actor == *actor)
        .ok_or_else(|| unresolved("template interface is unavailable", None, None))?;
    let in_ports = ports
        .in_ports
        .iter()
        .map(|port| port_value(&port.id, ports.in_ports.len() == 1))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| unresolved(error, None, None))?;
    let out_ports = ports
        .out_ports
        .iter()
        .map(|port| port_value(&port.id, ports.out_ports.len() == 1))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| unresolved(error, None, None))?;
    let item = Value::object([
        (
            "authored_actor",
            encode_boundary_actor_key(actor)
                .map_err(|error| unresolved(error.to_string(), None, None))?,
        ),
        ("config", config),
        ("in_ports", Value::Array(in_ports)),
        ("actor_type", Value::string(kind.as_str())),
        ("out_ports", Value::Array(out_ports)),
    ])
    .map_err(|error| unresolved(format!("template admission: {error}"), None, None))?;
    Ok(QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor: Value::string(kind.as_str()),
        items: vec![item],
        terminal: Terminal::Complete,
    })
}

/// Admit an edited configuration for one exact existing authored actor and
/// prove the complete current graph still assembles with the resulting ports.
///
/// This query is read-only. The accepted item is a configure-specific receipt,
/// not a create template; mutation still requires a separately CAS-fenced
/// `UpsertActor` epoch.
pub(crate) fn configuration_admission_page(
    args: Value,
    authoring: &AuthoringState,
) -> Result<QueryPage, Rejected> {
    let Value::Object(object) = args else {
        return Err(malformed(
            "actor.configure-admission args are not an object",
            None,
        ));
    };
    let mut fields = object.into_map();
    let config = fields.remove("config").ok_or_else(|| {
        malformed(
            "actor.configure-admission has no config",
            Some(Value::String("config".to_owned())),
        )
    })?;
    let expected_revision = fields.remove("expected_revision").ok_or_else(|| {
        malformed(
            "actor.configure-admission has no expected_revision",
            Some(Value::String("expected_revision".to_owned())),
        )
    })?;
    let actor = fields
        .remove("actor")
        .ok_or_else(|| {
            malformed(
                "actor.configure-admission has no actor",
                Some(Value::String("actor".to_owned())),
            )
        })
        .and_then(|value| {
            decode_boundary_actor_key(value).map_err(|error| {
                malformed(
                    format!("actor.configure-admission actor: {error}"),
                    Some(Value::String("actor".to_owned())),
                )
            })
        })?;
    let scope = fields
        .remove("scope")
        .ok_or_else(|| {
            malformed(
                "actor.configure-admission has no scope",
                Some(Value::String("scope".to_owned())),
            )
        })
        .and_then(|value| {
            decode_scope_identity(value).map_err(|error| {
                malformed(
                    format!("actor.configure-admission scope: {error}"),
                    Some(Value::String("scope".to_owned())),
                )
            })
        })?;
    if let Some((unknown, _)) = fields.into_iter().next() {
        return Err(malformed(
            format!(
                "actor.configure-admission carries unknown field {}",
                Quoted(&unknown)
            ),
            Some(Value::String(unknown)),
        ));
    }
    if !actor.scope.starts_with(&scope) {
        return Err(unresolved(
            "actor.configure-admission target lies outside the requested authoring scope",
            Some("Refresh the current authoring cut and selected actor."),
            encode_boundary_actor_key(&actor).ok(),
        ));
    }
    let snapshot = authoring.snapshot(scope.clone()).map_err(|rejection| {
        unresolved(
            rejection.to_string(),
            Some("Wait for a committed authoring snapshot before editing configuration."),
            Some(Value::String("scope".to_owned())),
        )
    })?;
    let current_revision = current_revision_value(snapshot.authoring_revision.as_deref());
    if expected_revision != current_revision {
        return Err(unresolved(
            "actor.configure-admission authoring revision is stale",
            Some("Refresh the authored graph before saving this configuration."),
            Some(Value::String("expected_revision".to_owned())),
        ));
    }

    let (current_key, current_declaration) = authoring
        .current()
        .tables()
        .actors()
        .iter()
        .find(|(key, _)| key == &actor)
        .ok_or_else(|| {
            unresolved(
                format!("authored actor `{actor}` does not exist at this cut"),
                Some("Refresh the current selection."),
                encode_boundary_actor_key(&actor).ok(),
            )
        })?;
    let Some(kind) = ActorType::from_str(&current_declaration.actor_type) else {
        return Err(unresolved(
            format!(
                "authored actor `{actor}` has unknown type {}",
                Quoted(&current_declaration.actor_type)
            ),
            Some("Refresh the daemon-owned actor registry."),
            encode_boundary_actor_key(&actor).ok(),
        ));
    };
    let registration = registration(kind);
    if !matches!(registration.scope(), RegistrationScope::Published) {
        return Err(unresolved(
            format!(
                "authored actor type {} is not published for product configuration",
                Quoted(&current_declaration.actor_type)
            ),
            None,
            encode_boundary_actor_key(&actor).ok(),
        ));
    }
    if registration.spec().config().is_empty() {
        return Err(unresolved(
            format!(
                "authored actor type {} has no configurable fields",
                Quoted(&current_declaration.actor_type)
            ),
            Some("This actor has no registry-owned Configuration form."),
            encode_boundary_actor_key(&actor).ok(),
        ));
    }

    let admitted = admit_registered_create_at(
        kind,
        &config,
        current_key,
        authoring.current().actor_generation(current_key),
    )
    .map_err(|error| admission_rejection(error, &current_key.to_string(), &config))?;
    let (admitted_kind, admitted_config, admitted_ports, admitted_actor) = admitted.into_parts();
    if admitted_kind != kind || admitted_actor.as_ref() != Some(current_key) {
        return Err(unresolved(
            "registry configuration admission changed the authored actor identity or type",
            Some("Refresh the daemon-owned registry before retrying."),
            encode_boundary_actor_key(&actor).ok(),
        ));
    }

    let mut candidate = authoring.current().clone();
    candidate
        .apply(&ContentVerb::UpsertActor {
            actor: current_key.clone(),
            declaration: circular_protocol::declaration_payload::ActorDeclaration {
                actor_type: current_declaration.actor_type.clone(),
                config: admitted_config.clone(),
                flags: current_declaration.flags,
            },
        })
        .map_err(|error| {
            config_rejected(
                &actor.to_string(),
                &config,
                format!("configured authored graph is invalid: {error}"),
            )
        })?;
    let plan = candidate.assemble().map_err(|error| {
        config_rejected(
            &actor.to_string(),
            &config,
            format!("configured authored graph is invalid: {error}"),
        )
    })?;
    engine::validate_published_plan(&plan).map_err(|error| {
        config_rejected(
            &actor.to_string(),
            &config,
            format!("configured authored graph is invalid: {error}"),
        )
    })?;

    admitted_configuration_page(
        current_key,
        kind,
        admitted_config,
        admitted_ports,
        current_revision,
    )
}

fn admitted_configuration_page(
    actor: &circular_protocol::declaration_payload::PlanActorKey,
    actor_type: ActorType,
    config: Value,
    ports: circular_actors::PortSet,
    admitted_revision: Value,
) -> Result<QueryPage, Rejected> {
    let in_ports = ports
        .inlets()
        .iter()
        .map(|port| port_value(port.id().as_str(), port.primary()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|message| unresolved(message, None, None))?;
    let out_ports = admitted_out_ports(actor_type, &ports)?;
    let actor_value = encode_boundary_actor_key(actor)
        .map_err(|error| unresolved(format!("configured actor identity: {error}"), None, None))?;
    let item = Value::object([
        ("admitted_revision", admitted_revision),
        ("config", config),
        ("in_ports", Value::Array(in_ports)),
        ("actor", actor_value.clone()),
        ("actor_type", Value::String(actor_type.as_str().to_owned())),
        ("out_ports", Value::Array(out_ports)),
    ])
    .map_err(|error| unresolved(format!("actor configuration item: {error}"), None, None))?;
    Ok(QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor: actor_value,
        items: vec![item],
        terminal: Terminal::Complete,
    })
}

fn admitted_create_page(admitted: AdmittedActorCreate) -> Result<QueryPage, Rejected> {
    let (actor_type, config, ports, authored_actor) = admitted.into_parts();
    let in_ports = ports
        .inlets()
        .iter()
        .map(|port| port_value(port.id().as_str(), port.primary()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|message| unresolved(message, None, None))?;
    let out_ports = admitted_out_ports(actor_type, &ports)?;
    let actor_type = actor_type.as_str().to_owned();
    let item = Value::object([
        (
            "authored_actor",
            authored_actor
                .as_ref()
                .map(encode_boundary_actor_key)
                .transpose()
                .map_err(|error| unresolved(format!("actor create identity: {error}"), None, None))?
                .unwrap_or(Value::Null),
        ),
        ("config", config),
        ("in_ports", Value::Array(in_ports)),
        ("actor_type", Value::String(actor_type.clone())),
        ("out_ports", Value::Array(out_ports)),
    ])
    .map_err(|error| unresolved(format!("actor create admission item: {error}"), None, None))?;
    Ok(QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor: Value::String(actor_type),
        items: vec![item],
        terminal: Terminal::Complete,
    })
}

fn admitted_out_ports(
    actor_type: ActorType,
    ports: &circular_actors::PortSet,
) -> Result<Vec<Value>, Rejected> {
    let mut outlets = ports
        .outlets()
        .iter()
        .map(|port| port_value(port.id().as_str(), port.primary()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|message| unresolved(message, None, None))?;
    if derived_error_outlet(actor_type, Side::Outlet, "_error").is_some() {
        outlets
            .push(port_value("_error", false).map_err(|message| unresolved(message, None, None))?);
    }
    Ok(outlets)
}

fn config_rejected(actor: &str, config: &Value, reason: impl std::fmt::Display) -> Rejected {
    RejectionReason::Malformed
        .reject(
            Partition::Query,
            circular_actors::config::config_rejection(actor, "config", Some(config), reason),
        )
        .hint("Fix the configuration and submit it again.".to_owned())
}

fn admission_rejection(
    error: RegisteredCreateAdmissionError,
    actor: &str,
    config: &Value,
) -> Rejected {
    match error {
        RegisteredCreateAdmissionError::Config(error) => {
            let at = config_admission_at(&error);
            Rejected {
                at,
                ..RejectionReason::Malformed
                    .reject(Partition::Query, error.rejection_message(actor, config))
                    .hint("Fix the configuration at the reported location and submit it again.")
            }
        }
        RegisteredCreateAdmissionError::Unavailable(error) => unresolved(
            error.to_string(),
            Some("Use only a registration whose actor.create-inputs state is Authoring."),
            None,
        ),
        RegisteredCreateAdmissionError::Ports(error) => RejectionReason::Malformed
            .reject(
                Partition::Query,
                circular_actors::config::config_rejection(actor, "config", Some(config), error),
            )
            .hint("Fix the configuration and submit it again.".to_owned()),
        RegisteredCreateAdmissionError::BoundaryIdentityRequired => unresolved(
            "boundary actor creation requires an exact prospective authored actor identity",
            Some(
                "Submit an exact prospective authored actor identity from the current authoring cut.",
            ),
            Some(Value::String("authored_actor".to_owned())),
        ),
        RegisteredCreateAdmissionError::BoundaryIdentity(error) => unresolved(
            error.to_string(),
            Some("Refresh the authoring cut and derive a new prospective actor identity."),
            Some(Value::String("authored_actor".to_owned())),
        ),
    }
}

pub(crate) fn config_admission_at(error: &CreateInputAdmissionError) -> Option<Value> {
    match error {
        CreateInputAdmissionError::UnknownField(field) => Some(Value::String(field.clone())),
        CreateInputAdmissionError::MissingMandatory(path)
        | CreateInputAdmissionError::OutsideSpace { path, .. }
        | CreateInputAdmissionError::UnresolvedConstraint { path, .. }
        | CreateInputAdmissionError::Snippet { path, .. }
        | CreateInputAdmissionError::DuplicateObjectValue { path, .. } => {
            config_path_value(path).ok()
        }
        CreateInputAdmissionError::EmptySchemaRequiresNull
        | CreateInputAdmissionError::RootNotObject => None,
    }
}

fn malformed(message: impl Into<String>, at: Option<Value>) -> Rejected {
    Rejected {
        hint: None,
        at: at,
        ..RejectionReason::Malformed.reject(Partition::Query, message.into())
    }
}

fn unresolved(message: impl Into<String>, hint: Option<&str>, at: Option<Value>) -> Rejected {
    Rejected {
        hint: hint.map(str::to_owned),
        at: at,
        ..RejectionReason::Unresolved.reject(Partition::Query, message.into())
    }
}

pub(crate) fn catalog_anchor() -> Value {
    Value::Array(
        ActorType::ALL
            .into_iter()
            .filter(|actor_type| {
                matches!(
                    registration(*actor_type).scope(),
                    RegistrationScope::Published
                )
            })
            .map(|actor_type| Value::String(actor_type.as_str().to_owned()))
            .collect(),
    )
}

pub(crate) fn catalog_items() -> Result<Vec<Value>, String> {
    ActorType::ALL
        .into_iter()
        .filter(|actor_type| {
            matches!(
                registration(*actor_type).scope(),
                RegistrationScope::Published
            )
        })
        .map(catalog_item)
        .collect()
}

pub(crate) fn create_input_anchor() -> Value {
    catalog_anchor()
}

/// Lossless create-input facts for every published registration. Empty config
/// is a distinct `NotRequired` arm; only registration-sealed config schemas
/// reach the authored-input arm. Visible but incomplete ConfigSchema rows keep
/// their exact producer reason.
pub(crate) fn create_input_items() -> Result<Vec<Value>, String> {
    ActorType::ALL
        .into_iter()
        .filter(|actor_type| {
            matches!(
                registration(*actor_type).scope(),
                RegistrationScope::Published
            )
        })
        .map(create_input_item)
        .collect()
}

fn create_input_item(actor_type: ActorType) -> Result<Value, String> {
    let spec = registration(actor_type).spec();
    let state = if spec.config().is_empty() {
        Value::array([Value::Int(1)])
    } else {
        match registered_create_inputs(actor_type) {
            Ok(schema) => {
                let draft = registered_create_draft(actor_type)
                    .expect("the same sealed schema mints its draft");
                Value::array([
                    Value::Int(2),
                    create_input_schema_value(actor_type, schema)?,
                    draft.config().clone(),
                    Value::Array(
                        draft
                            .missing()
                            .iter()
                            .map(config_path_value)
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                ])
            }
            Err(error) => {
                Value::array([Value::Int(3), Value::String(error.to_string())])
            }
        }
    };
    Value::object([
        ("actor_type", Value::String(actor_type.as_str().to_owned())),
        ("state", state),
    ])
    .map_err(|error| format!("actor create-input item: {error}"))
}

fn create_input_schema_value(
    actor_type: ActorType,
    schema: circular_actors::CreateInputSchema<'_>,
) -> Result<Value, String> {
    let mut slots = schema
        .slots()
        .map(|(path, slot)| Ok((path.clone(), create_input_slot_value(path, slot)?)))
        .collect::<Result<std::collections::BTreeMap<_, _>, String>>()?;
    let policy_inputs = circular_actors::capability_config::create_inputs(actor_type);
    if !policy_inputs.is_empty() {
        let path =
            circular_actors::ConfigPath::root().join_key(circular_actors::capability_config::FIELD);
        let slot = slots
            .get_mut(&path)
            .ok_or("declared capabilities have no config slot")?;
        let policies = policy_inputs
            .into_iter()
            .map(|(name, inputs)| {
                let inputs = inputs
                    .iter()
                    .map(|(path, slot)| create_input_slot_value(path, slot))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((name, Value::Array(inputs)))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut fields = slot
            .as_object()
            .expect("projected slot is an object")
            .clone()
            .into_map();
        fields.insert(
            "policies".into(),
            Value::object(policies).map_err(|e| format!("policy inputs: {e}"))?,
        );
        *slot = Value::object(fields).map_err(|e| format!("capability slot: {e}"))?;
    }
    let relations = schema
        .relations()
        .iter()
        .map(|relation| match relation {
            CreateInputRelation::UniqueObjectValues { at } => {
                Ok(Value::array([Value::Int(1), config_path_value(at)?]))
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    Value::object([
        ("relations", Value::Array(relations)),
        ("slots", Value::Array(slots.into_values().collect())),
    ])
    .map_err(|error| format!("actor create-input schema: {error}"))
}

pub(crate) fn create_input_slot_value(
    path: &circular_actors::ConfigPath,
    slot: &ConfigSlot,
) -> Result<Value, String> {
    let requirement = match slot.required() {
        Required::Mandatory => Value::array([Value::Int(1)]),
        Required::Optional { default } => Value::array([Value::Int(2), default.clone()]),
        Required::Omittable { absent } => Value::array([
            Value::Int(3),
            encode_port_flow(&port_flow(absent))
                .map_err(|error| format!("create-input omittable Flow: {error}"))?,
        ]),
    };
    let snippet = match slot.snippet() {
        None => Value::Null,
        Some(snippet) => Value::object([
            (
                "inlets",
                Value::Array(
                    snippet
                        .inlets()
                        .iter()
                        .map(|inlet| Value::String(inlet.as_str().to_owned()))
                        .collect(),
                ),
            ),
            ("mode", Value::String(snippet.mode().as_str().to_owned())),
        ])
        .map_err(|error| format!("actor create-input snippet: {error}"))?,
    };
    Value::object([
        ("constraint", constraint_value(slot.space().constraint())?),
        (
            "label",
            slot.label.clone().map_or(Value::Null, Value::String),
        ),
        (
            "description",
            slot.description.clone().map_or(Value::Null, Value::String),
        ),
        (
            "group",
            slot.group.clone().map_or(Value::Null, Value::String),
        ),
        ("path", config_path_value(path)?),
        ("requirement", requirement),
        ("shape", shape_value(slot.space().shape())?),
        ("snippet", snippet),
    ])
    .map_err(|error| format!("actor create-input slot: {error}"))
}

fn config_path_value(path: &circular_actors::ConfigPath) -> Result<Value, String> {
    let segments = path
        .segments()
        .iter()
        .map(|segment| {
            if let Some(key) = segment.as_key() {
                Ok(Value::array([Value::Int(1), Value::String(key.to_owned())]))
            } else if let Some(index) = segment.as_index() {
                let index = i64::try_from(index)
                    .map_err(|_| "config path index exceeds the Value::Int wire".to_owned())?;
                Ok(Value::array([Value::Int(2), Value::Int(index)]))
            } else {
                unreachable!("config path segment sum is closed")
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Value::Array(segments))
}

fn shape_value(shape: &circular_actors::Shape) -> Result<Value, String> {
    let shape = PortShape::from_core(&circular_actors::types::to_unnamed_shape(shape));
    encode_port_shape(&shape).map_err(|error| format!("port shape projection: {error}"))
}

fn constraint_value(constraint: Option<&ConfigConstraint>) -> Result<Value, String> {
    let Some(constraint) = constraint else {
        return Ok(Value::Null);
    };
    let interval_domain = |domain: &IntervalDomain| {
        Value::String(
            match domain {
                IntervalDomain::Milliseconds => "milliseconds",
                IntervalDomain::NonZeroMilliseconds => "nonzero_milliseconds",
            }
            .to_owned(),
        )
    };
    Ok(match constraint {
        ConfigConstraint::Interval(domain) => {
            Value::array([Value::Int(1), interval_domain(domain)])
        }
        ConfigConstraint::IntervalList(domain) => {
            Value::array([Value::Int(10), interval_domain(domain)])
        }
        ConfigConstraint::IntegerCount(count) => Value::array([
            Value::Int(2),
            Value::Int(match count.minimum() {
                IntegerMinimum::Zero => 0,
                IntegerMinimum::One => 1,
            }),
            count
                .maximum()
                .map_or(Value::Null, |maximum| Value::Int(maximum.get())),
        ]),
        ConfigConstraint::FiniteNumber => Value::array([Value::Int(3)]),
        ConfigConstraint::ClosedUnitInterval => Value::array([Value::Int(4)]),
        ConfigConstraint::ClosedTags(tags) => closed_tags_value(tags),
        ConfigConstraint::CanonicalText(domain) => text_domain_value(*domain)?,
        ConfigConstraint::CanonicalTypeExpr => Value::array([Value::Int(7)]),
        ConfigConstraint::ExactPayloadPath => Value::array([Value::Int(8)]),
        ConfigConstraint::CanonicalBaseStream => Value::array([Value::Int(9)]),
    })
}

fn closed_tags_value(tags: &ClosedTags) -> Value {
    Value::array([
        Value::Int(5),
        Value::Array(
            tags.iter()
                .map(|member| Value::String(member.to_owned()))
                .collect(),
        ),
    ])
}

fn text_domain_value(domain: TextDomain) -> Result<Value, String> {
    let declared: Vec<&'static str> = match domain {
        TextDomain::AgentHarnessName => engine::cli_adapter_names().collect(),
        TextDomain::PeerAdapterName => {
            engine::peer_adapter::selectable_peer_adapter_names().collect()
        }
        TextDomain::Label => return Ok(text_domain_name("label")),
        TextDomain::Name => return Ok(text_domain_name("name")),
        TextDomain::ToolName => return Ok(text_domain_name("tool_name")),
        TextDomain::ModelProviderName => return Ok(text_domain_name("model_provider_name")),
        TextDomain::ModelName => return Ok(text_domain_name("model_name")),
    };
    let tags = ClosedTags::try_from_members(declared)
        .map_err(|error| format!("declared adapter names for {}: {error}", domain.noun()))?;
    Ok(closed_tags_value(&tags))
}

fn text_domain_name(name: &str) -> Value {
    Value::array([Value::Int(6), Value::String(name.to_owned())])
}

pub(crate) fn authoring_port_items(
    facts: Vec<AuthoringActorPortFact>,
) -> Result<Vec<Value>, String> {
    let items = facts
        .into_iter()
        .map(|fact| {
            let actor = Value::object([
                ("local", Value::String(fact.actor.local.as_str().to_owned())),
                ("scope", scope_identity_value(&fact.actor.scope)),
            ])
            .map_err(|error| format!("authoring actor port identity: {error}"))?;
            Value::object([
                ("in_ports", authoring_ports_value(fact.in_ports)?),
                ("actor", Value::array([Value::Int(1), actor])),
                ("out_ports", authoring_ports_value(fact.out_ports)?),
            ])
            .map_err(|error| format!("authoring actor port item: {error}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut encoded_items = items
        .into_iter()
        .map(|item| {
            let encoded = encode(&item, Ceilings::for_boundary(Boundary::Wire))
                .map_err(|error| format!("projected port item does not encode: {error}"))?;
            Ok((encoded, item))
        })
        .collect::<Result<Vec<_>, String>>()?;
    encoded_items.sort_by(|(left, _), (right, _)| left.cmp(right));
    Ok(encoded_items.into_iter().map(|(_, item)| item).collect())
}

fn authoring_ports_value(ports: Vec<AuthoringPortFact>) -> Result<Value, String> {
    let ports = ports
        .into_iter()
        .map(|port| {
            let AuthoringPortFlowFact::Known(flow) = port.flow;
            let flow = Value::array([
                Value::Int(1),
                encode_port_flow(&port_flow(&flow))
                    .map_err(|error| format!("authoring port Flow: {error}"))?,
            ]);
            let label = port.label.map_or(Value::Null, Value::String);
            Value::object([
                ("flow", flow),
                ("id", Value::String(port.id)),
                ("label", label),
            ])
            .map_err(|error| format!("authoring port fact: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Array(ports))
}

fn port_flow(flow: &circular_actors::Flow) -> PortFlow {
    let shape = |shape: &circular_actors::Shape| {
        PortShape::from_core(&circular_actors::types::to_unnamed_shape(shape))
    };
    match flow {
        circular_actors::Flow::Stream(item) => PortFlow::Stream(shape(item)),
        circular_actors::Flow::Signal { item, rate } => PortFlow::Signal {
            item: shape(item),
            rate: match rate {
                RateExpr::Period(period) => PortRate::Period(*period),
                RateExpr::RateVar(name) => PortRate::Variable(name.as_str().to_owned()),
            },
        },
    }
}

fn structural_create_requirement(actor_type: ActorType) -> Option<&'static str> {
    (actor_type == ActorType::PipelineActor)
        .then_some("pipeline_actor creation requires a paired authored scope declaration")
}

fn catalog_item(actor_type: ActorType) -> Result<Value, String> {
    let row = registration(actor_type);
    let spec = row.spec();
    let config_typed_boundary = spec
        .boundary()
        .is_some_and(|boundary| !matches!(boundary.ty(), circular_actors::TypeRule::Fixed(_)));
    let static_ports = spec.ports().dynamic().is_empty() && !config_typed_boundary;
    let config_empty = spec.config().is_empty();
    let structural = structural_create_requirement(actor_type);
    let creatable = static_ports && config_empty && structural.is_none();
    let unavailable_reason = if creatable {
        Value::Null
    } else {
        let reason = if let Some(reason) = structural {
            reason.to_owned()
        } else if config_empty {
            DYNAMIC_PORT_STOP_LINE.to_owned()
        } else {
            CONFIG_SCHEMA_STOP_LINE.to_owned()
        };
        Value::String(reason)
    };
    let config_schema = if config_empty {
        Value::Int(1)
    } else {
        Value::array([
            Value::Int(2),
            Value::String(CONFIG_SCHEMA_STOP_LINE.to_owned()),
        ])
    };
    let ports_unavailable_reason = if static_ports {
        Value::Null
    } else {
        Value::String(DYNAMIC_PORT_STOP_LINE.to_owned())
    };
    let fixed = spec.ports().fixed();
    let in_ports = fixed
        .inlets()
        .iter()
        .map(|port| port_value(port.id().as_str(), port.primary()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut out_ports = fixed
        .outlets()
        .iter()
        .map(|port| port_value(port.id().as_str(), port.primary()))
        .collect::<Result<Vec<_>, _>>()?;
    if derived_error_outlet(actor_type, Side::Outlet, "_error").is_some() {
        out_ports.push(port_value("_error", false)?);
    }
    Value::object([
        ("actor_type", Value::String(actor_type.as_str().to_owned())),
        (
            "label",
            Value::String(spec.display().label().as_str().to_owned()),
        ),
        (
            "description",
            Value::String(spec.display().description().as_str().to_owned()),
        ),
        (
            "presentation_role",
            presentation_role_value(row.presentation()),
        ),
        ("source", Value::Bool(spec.is_source())),
        (
            "view_config",
            spec.view_config().cloned().unwrap_or(Value::Null),
        ),
        ("config_schema", config_schema),
        ("creatable", Value::Bool(creatable)),
        ("template_config", Value::Null),
        ("in_ports", Value::Array(in_ports)),
        ("out_ports", Value::Array(out_ports)),
        ("ports_unavailable_reason", ports_unavailable_reason),
        ("unavailable_reason", unavailable_reason),
    ])
    .map_err(|error| format!("actor catalog item: {error}"))
}

/// Closed value sum for the canonical graph-presentation family.
///
/// 1 = Instrument, 2 = InlineOperation, 3 = SystemBoundary(direction),
/// 4 = Container(cardinality).  The app decoder rejects unknown tags, arity,
/// directions, and cardinalities rather than silently treating them as a body.
fn presentation_role_value(role: GraphPresentationRole) -> Value {
    match role {
        GraphPresentationRole::Instrument => Value::array([Value::Int(1)]),
        GraphPresentationRole::InlineOperation => Value::array([Value::Int(2)]),
        GraphPresentationRole::SystemBoundary(direction) => Value::array([
            Value::Int(3),
            Value::String(
                match direction {
                    BoundaryDirection::Source => "source",
                    BoundaryDirection::Sink => "sink",
                }
                .to_owned(),
            ),
        ]),
        GraphPresentationRole::Container(cardinality) => Value::array([
            Value::Int(4),
            Value::String(
                match cardinality {
                    ContainerCardinality::One => "one",
                    ContainerCardinality::KeyedMany => "keyed_many",
                }
                .to_owned(),
            ),
        ]),
    }
}

fn port_value(id: &str, primary: bool) -> Result<Value, String> {
    Value::object([
        ("id", Value::String(id.to_owned())),
        ("primary", Value::Bool(primary)),
    ])
    .map_err(|error| format!("actor catalog port: {error}"))
}

