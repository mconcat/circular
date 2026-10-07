
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use crate::activation_config::fold_config;
use crate::authoring_assembly::projection::AuthoredProjection;
use crate::authoring_assembly::projection::fold_projection;
use circular_actors::{
    ActorType, BoundaryPortRule, CreateInputAdmissionError, Flow, GroundFlow, GroundShape, Name,
    PortSet, RegistrationScope, Side, StaticPortQueryError, StaticPortRequest, Substitution,
    connectable, get, registration, resolve_static_port,
};
use circular_plan::{
    ActorDecl, BoundaryPortRef, DeclaredEdgeId, EdgeDecl, ExportName, NamedActorId, PortId, ScopeId,
};
use circular_protocol::boundary_port::{
    BoundaryActorGeneration, BoundaryPortId, validate_boundary_port_ids,
};
use circular_protocol::declaration_payload::PlanActorKey;
use std::collections::{BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryProfile {
    Fixture,
    Published,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncompatibleConnectionReport {
    pub edge: DeclaredEdgeId,
    pub from: String,
    pub to: String,
    pub hint: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanRegistryError {
    FixtureLocalActorInPublishedPlan {
        actor: NamedActorId,
        actor_type: ActorType,
    },
    UnknownPort {
        actor: NamedActorId,
        side: Side,
        port: PortId,
        available: Box<[PortId]>,
    },
    TypeVarConflict(Box<TypeVarConflictReport>),
    UnassignedTypeVar {
        actor: NamedActorId,
        side: Side,
        port: PortId,
    },
    IncompatibleConnection(Box<IncompatibleConnectionReport>),
    ConfigFold {
        actor_type: ActorType,
        detail: String,
        admission: Option<Box<CreateInputAdmissionError>>,
    },
    PortExpansion {
        actor: Option<NamedActorId>,
        actor_type: ActorType,
        detail: String,
    },
    /// An edge program failed at its authored step, without an actor registration.
    PreprocessInvariant {
        edge: Box<DeclaredEdgeId>,
        step: usize,
        detail: String,
    },
    ArityExceeded {
        actor: NamedActorId,
        side: Side,
        port: PortId,
    },
    InvalidRequestExport {
        export: ExportName,
        actor: NamedActorId,
        port: PortId,
        reason: RequestExportInvalidity,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestExportInvalidity {
    UnknownActor,
    NotInputBoundary,
    PortResolution(String),
    UnknownBoundaryOutlet { available: Box<[PortId]> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RequestExportIngress {
    BoundarySource {
        emitter: NamedActorId,
        outlet: PortId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypeVarConflictReport {
    pub variable: Name,
    pub existing: GroundShape,
    pub incoming: GroundShape,
    pub owner: NamedActorId,
    pub actor: NamedActorId,
    pub side: Side,
    pub port: PortId,
}

impl fmt::Display for PlanRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FixtureLocalActorInPublishedPlan { actor, actor_type } => write!(
                formatter,
                "actor {} uses fixture-only type {} and cannot be published",
                actor_text(actor),
                actor_type.as_str(),
            ),
            Self::UnknownPort {
                actor,
                side,
                port,
                available,
            } => write!(
                formatter,
                "actor {} has no {} port {port}; available ports: {}",
                actor_text(actor),
                side_text(*side),
                port_list(available),
            ),
            Self::TypeVarConflict(report) => write!(
                formatter,
                "type variable {} on actor {} is already {} and cannot also be {} from {} port {}.{}",
                report.variable,
                actor_text(&report.owner),
                shape_text(report.existing.as_shape()),
                shape_text(report.incoming.as_shape()),
                side_text(report.side),
                actor_text(&report.actor),
                report.port,
            ),
            Self::UnassignedTypeVar { actor, side, port } => write!(
                formatter,
                "cannot determine the type of {} port {}.{port}",
                side_text(*side),
                actor_text(actor),
            ),
            Self::IncompatibleConnection(report) => {
                write!(
                    formatter,
                    "cannot connect {} to {}: the output is {}, the input takes {}",
                    endpoint_text(report.edge.from()),
                    endpoint_text(report.edge.to()),
                    report.from,
                    report.to,
                )?;
                if !report.hint.is_empty() {
                    write!(formatter, "; {}", report.hint)?;
                }
                Ok(())
            }
            Self::ConfigFold { detail, .. } => formatter.write_str(detail),
            Self::PortExpansion { detail, .. } => formatter.write_str(detail),
            Self::PreprocessInvariant { edge, step, detail } => write!(
                formatter,
                "cannot connect {} to {}: preprocess step {step} failed: {detail}",
                endpoint_text(edge.from()),
                endpoint_text(edge.to()),
            ),
            Self::ArityExceeded { actor, side, port } => write!(
                formatter,
                "cannot connect more than one edge to {} port {}.{port}",
                side_text(*side),
                actor_text(actor),
            ),
            Self::InvalidRequestExport {
                export,
                actor,
                port,
                reason,
            } => {
                write!(
                    formatter,
                    "request export {} cannot target {}.{port}: ",
                    export.name(),
                    actor_text(actor),
                )?;
                match reason {
                    RequestExportInvalidity::UnknownActor => {
                        formatter.write_str("the actor does not exist")
                    }
                    RequestExportInvalidity::NotInputBoundary => {
                        formatter.write_str("the target is not an input boundary")
                    }
                    RequestExportInvalidity::PortResolution(detail) => {
                        write!(formatter, "the port could not be resolved: {detail}")
                    }
                    RequestExportInvalidity::UnknownBoundaryOutlet { available } => write!(
                        formatter,
                        "the boundary has no such outlet; available outlets: {}",
                        port_list(available),
                    ),
                }
            }
        }
    }
}

fn actor_text(actor: &NamedActorId) -> String {
    actor.to_string()
}

fn endpoint_text(endpoint: &circular_plan::Endpoint) -> String {
    format!("{}.{}", endpoint.actor(), endpoint.port())
}

fn side_text(side: Side) -> &'static str {
    match side {
        Side::Inlet => "input",
        Side::Outlet => "output",
    }
}

fn port_list(ports: &[PortId]) -> String {
    if ports.is_empty() {
        "none".to_owned()
    } else {
        ports
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn shape_text(shape: &circular_actors::Shape) -> String {
    circular_core::spelling::ShapeText(shape).to_string()
}

fn flow_text(flow: &GroundFlow) -> String {
    match flow.as_flow() {
        Flow::Stream(item) => format!("stream<{}>", shape_text(item)),
        Flow::Signal { item, rate } => {
            let rate = match rate {
                circular_actors::RateExpr::Period(period) => {
                    format!("every {} ticks", period.get().get())
                }
                circular_actors::RateExpr::RateVar(name) => format!("rate {name}"),
            };
            format!("signal<{}, {rate}>", shape_text(item))
        }
    }
}

impl Error for PlanRegistryError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegisteredPortError {
    Query {
        actor_type: ActorType,
        source: StaticPortQueryError,
    },
    MissingDefault {
        actor_type: ActorType,
        side: Side,
    },
}

impl fmt::Display for RegisteredPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Query { actor_type, source } => write!(
                formatter,
                "registered default port lookup failed for actor type {actor_type}: {source}"
            ),
            Self::MissingDefault { actor_type, side } => write!(
                formatter,
                "actor type {actor_type} registers no default {} port",
                side_text(*side)
            ),
        }
    }
}

impl Error for RegisteredPortError {}

pub(crate) fn default_plan_port(
    actor_type: ActorType,
    side: Side,
) -> Result<PortId, RegisteredPortError> {
    let port = resolve_static_port(get(actor_type), StaticPortRequest::Default { side })
        .map_err(|source| RegisteredPortError::Query { actor_type, source })?
        .ok_or(RegisteredPortError::MissingDefault { actor_type, side })?;
    Ok(port.id().clone())
}

type ContainerPorts = BTreeMap<(Side, PortId), EndpointFlow>;

#[derive(Clone, Debug)]
struct EndpointFlow {
    flow: circular_actors::Flow,
    arity: circular_actors::Arity,
    undeclared_any: bool,
}

/// Immutable registration/shape output owned by one admitted plan revision.
/// Lookups do not retain a AuthoredProjection or invoke flatten, config folding, or CEL parsing.
#[derive(Clone, Debug, Default)]
pub struct ResolvedRevisionPorts {
    actors: std::sync::Arc<BTreeMap<NamedActorId, Result<ResolvedActorPorts, PlanRegistryError>>>,
    requests: std::sync::Arc<BTreeMap<(ScopeId, ExportName), RequestExportIngress>>,
    carried_shapes:
        std::sync::Arc<BTreeMap<(NamedActorId, Side, PortId), Result<GroundShape, String>>>,
}

#[derive(Clone, Debug)]
struct ResolvedActorPorts {
    shapes: BTreeMap<(Side, PortId), Option<GroundShape>>,
    available: BTreeMap<Side, Box<[PortId]>>,
}

impl ResolvedRevisionPorts {
    pub fn resolve(plan: &AuthoredProjection) -> Result<Self, PlanRegistryError> {
        let mut ports = Self::default();
        resolve_plan_shape(plan, RegistryProfile::Published, None, Some(&mut ports))?;
        ports.carried_shapes = std::sync::Arc::new(runtime_shapes::derive(plan)?);
        Ok(ports)
    }

    pub fn carried_shape(
        &self,
        actor: &NamedActorId,
        port: &PortId,
        input: bool,
    ) -> Option<&Result<GroundShape, String>> {
        self.carried_shapes.get(&(
            actor.clone(),
            if input { Side::Inlet } else { Side::Outlet },
            port.clone(),
        ))
    }

    pub fn contains_actor(&self, actor: &NamedActorId) -> bool {
        self.actors.contains_key(actor)
    }

    pub fn request(&self, scope: &ScopeId, name: &ExportName) -> Option<&RequestExportIngress> {
        self.requests.get(&(scope.clone(), name.clone()))
    }

    pub fn shape(
        &self,
        actor: &NamedActorId,
        port: &PortId,
        input: bool,
    ) -> Result<Option<GroundShape>, PlanRegistryError> {
        let Some(resolved) = self.actors.get(actor) else {
            return Ok(None);
        };
        let resolved = resolved.as_ref().map_err(Clone::clone)?;
        let side = if input { Side::Inlet } else { Side::Outlet };
        resolved
            .shapes
            .get(&(side, port.clone()))
            .cloned()
            .ok_or_else(|| PlanRegistryError::UnknownPort {
                actor: actor.clone(),
                side,
                port: port.clone(),
                available: resolved.available[&side].clone(),
            })
    }
}

struct ResolvedLayer {
    ports: ContainerPorts,
    shape: Option<GroundShape>,
}

pub(crate) fn resolve_plan(
    plan: &AuthoredProjection,
    profile: RegistryProfile,
) -> Result<(), PlanRegistryError> {
    resolve_plan_shape(plan, profile, None, None).map(|_| ())
}

pub(crate) fn resolve_plan_inlets(
    plan: &AuthoredProjection,
    profile: RegistryProfile,
) -> Result<BTreeMap<NamedActorId, circular_actors::ResolvedInletShapes>, PlanRegistryError> {
    let mut ports = ResolvedRevisionPorts::default();
    resolve_plan_shape(plan, profile, None, Some(&mut ports))?;
    Ok(ports
        .actors
        .iter()
        .filter_map(|(actor, resolved)| {
            let shapes = circular_actors::ResolvedInletShapes::new(
                resolved
                    .as_ref()
                    .ok()?
                    .shapes
                    .iter()
                    .filter(|((side, _), _)| *side == Side::Inlet)
                    .filter_map(|((_, port), shape)| Some((port.clone(), shape.clone()?))),
            );
            (!shapes.is_empty()).then(|| (actor.clone(), shapes))
        })
        .collect())
}

/// Resolve a retained port with the same actor-local substitution as plan admission.
pub fn recorded_port_shape(
    plan: &AuthoredProjection,
    actor: &NamedActorId,
    port: &PortId,
    input: bool,
) -> Result<Option<GroundShape>, PlanRegistryError> {
    resolve_plan_shape(
        plan,
        RegistryProfile::Published,
        Some((actor, port, if input { Side::Inlet } else { Side::Outlet })),
        None,
    )
}

fn resolve_plan_shape(
    plan: &AuthoredProjection,
    profile: RegistryProfile,
    query: Option<(&NamedActorId, &PortId, Side)>,
    mut output: Option<&mut ResolvedRevisionPorts>,
) -> Result<Option<GroundShape>, PlanRegistryError> {
    let refused = plan.refused();
    let resolved = fold_projection::<Result<ResolvedLayer, PlanRegistryError>>(plan, |layer| {
        for (actor, declaration) in layer.actors() {
            let actor_type = *declaration.domain().actor_type();
            if profile == RegistryProfile::Published
                && matches!(
                    registration(actor_type).scope(),
                    RegistrationScope::FixtureLocal(_)
                )
            {
                return Err(PlanRegistryError::FixtureLocalActorInPublishedPlan {
                    actor: actor.clone(),
                    actor_type,
                });
            }
        }

        let mut containers: BTreeMap<circular_plan::Name, ContainerPorts> = BTreeMap::new();
        let actors = layer.actors();
        let edges = layer.edges();
        let boundary = layer.graph().declaration().boundary().clone();
        let mut semantic_boundaries = Vec::new();
        for (actor, declaration) in actors {
            let actor_type = *declaration.domain().actor_type();
            if let Some(rule) = get(actor_type).boundary() {
                let key = circular_runtime::product_identity::wire_named_actor(actor);
                semantic_boundaries.push((
                    rule.direction(),
                    key,
                    BoundaryActorGeneration::new(declaration.authored_generation()),
                    actor.clone(),
                    actor_type,
                ));
            }
        }
        validate_boundary_port_ids(
            semantic_boundaries
                .iter()
                .map(|(direction, key, generation, _, _)| (*direction, key, *generation)),
        )
        .map_err(|error| PlanRegistryError::PortExpansion {
            actor: None,
            actor_type: ActorType::PipelineActor,
            detail: format!("container boundary interface identity: {error}"),
        })?;
        let mut shape = None;
        for (segment, child) in layer.into_scopes() {
            let child = child?;
            if child.shape.is_some() {
                shape = child.shape;
            }
            containers.insert(segment.name().clone(), child.ports);
        }
        let edges = standing_edges(edges, actors, &containers, refused);
        let edges = edges.as_ref();
        if let Some((actor, port, side)) = query.filter(|(actor, _, _)| actors.contains_key(*actor))
        {
            let environment = substitute_scope(edges, actors, &containers)?;
            let flow = resolve_endpoint(actors, &containers, actor, side, port)?.flow;
            shape = environment.ground(actor, &flow).map(|flow| {
                GroundShape::try_new(flow.as_flow().item().clone()).expect("ground item")
            });
        }

        if let Some(output) = output.as_deref_mut() {
            let environment = substitute_scope(edges, actors, &containers)?;
            for (actor, declaration) in actors {
                let actor_type = *declaration.domain().actor_type();
                let resolved = resolve_ports(
                    Some(actor),
                    actor_type,
                    declaration.domain().config(),
                    BoundaryActorGeneration::new(declaration.authored_generation()),
                )
                .map(|ports| {
                    let mut flows = BTreeMap::new();
                    for port in ports.inlets() {
                        flows.insert((Side::Inlet, port.id().clone()), port.ty().clone());
                    }
                    for port in ports.outlets() {
                        flows.insert((Side::Outlet, port.id().clone()), port.ty().clone());
                    }
                    let spec = get(actor_type);
                    let faces = match spec.boundary() {
                        Some(rule) => Some(match rule.side() {
                            Side::Outlet => (Side::Outlet, Side::Inlet),
                            Side::Inlet => (Side::Inlet, Side::Outlet),
                        }),
                        None => spec.is_source().then_some((Side::Outlet, Side::Inlet)),
                    };
                    if let Some((public, runtime)) = faces {
                        let mirrored = flows
                            .iter()
                            .filter(|((side, _), _)| *side == public)
                            .map(|((_, port), flow)| ((runtime, port.clone()), flow.clone()))
                            .collect::<Vec<_>>();
                        flows.extend(mirrored);
                    }
                    if let Some((flow, _)) =
                        circular_actors::derived_error_outlet(actor_type, Side::Outlet, "_error")
                    {
                        flows
                            .entry((Side::Outlet, PortId::try_new("_error".to_owned()).unwrap()))
                            .or_insert(flow);
                    }
                    if let Some(boundary) = containers.get(actor.name()) {
                        for (key, port) in boundary {
                            flows.insert(key.clone(), port.flow.clone());
                        }
                    }
                    ResolvedActorPorts {
                        shapes: flows
                            .into_iter()
                            .map(|(key, flow)| {
                                (
                                    key,
                                    environment.ground(actor, &flow).map(|flow| {
                                        GroundShape::try_new(flow.as_flow().item().clone())
                                            .expect("ground item")
                                    }),
                                )
                            })
                            .collect(),
                        available: [Side::Inlet, Side::Outlet]
                            .into_iter()
                            .map(|side| (side, circular_actors::available_ports(&ports, side)))
                            .collect(),
                    }
                });
                std::sync::Arc::get_mut(&mut output.actors)
                    .expect("port output is built before sharing")
                    .insert(actor.clone(), resolved);
            }
        }

        validate_edges(edges, actors, &containers)?;

        let mut ports = ContainerPorts::new();
        for (side, bindings) in [
            (Side::Inlet, boundary.inlets()),
            (Side::Outlet, boundary.outlets()),
        ] {
            for (outer, inner) in bindings {
                let inner_side =
                    crate::run_graph::boundary_inner_side(actors.get(inner.actor()), side);
                let resolved =
                    resolve_endpoint(actors, &containers, inner.actor(), inner_side, inner.port())?;
                ports.insert((side, outer.clone()), resolved);
            }
        }
        Ok(ResolvedLayer { ports, shape })
    })?;
    validate_request_exports(plan, output)?;
    Ok(resolved.shape)
}

fn standing_edges<'a>(
    edges: &'a BTreeMap<DeclaredEdgeId, EdgeDecl>,
    actors: &BTreeMap<NamedActorId, ActorDecl>,
    containers: &BTreeMap<circular_plan::Name, ContainerPorts>,
    refused: &BTreeMap<NamedActorId, crate::authoring_assembly::projection::RefusedDeclaration>,
) -> std::borrow::Cow<'a, BTreeMap<DeclaredEdgeId, EdgeDecl>> {
    let unresolved = |endpoint: &circular_plan::Endpoint, side| {
        refused.contains_key(endpoint.actor())
            && resolve_endpoint(actors, containers, endpoint.actor(), side, endpoint.port())
                .is_err()
    };
    if !actors.keys().any(|actor| refused.contains_key(actor)) {
        return std::borrow::Cow::Borrowed(edges);
    }
    std::borrow::Cow::Owned(
        edges
            .iter()
            .filter(|(_, edge)| {
                !unresolved(edge.from(), Side::Outlet) && !unresolved(edge.to(), Side::Inlet)
            })
            .map(|(id, edge)| (id.clone(), edge.clone()))
            .collect(),
    )
}

/// Validate a daemon-authored product plan through the canonical published
/// registry without activating a runtime.
///
/// Authoring admission uses this read-only seam to prove edited dynamic ports,
/// Flow substitution, arity, boundary bindings, and request exports against
/// the complete candidate graph before it issues a mutation receipt.
pub fn validate_published_plan(plan: &AuthoredProjection) -> Result<(), PlanRegistryError> {
    resolve_plan(plan, RegistryProfile::Published)
}

fn validate_request_exports(
    plan: &AuthoredProjection,
    mut output: Option<&mut ResolvedRevisionPorts>,
) -> Result<(), PlanRegistryError> {
    let graph = std::cell::OnceCell::new();
    fold_projection::<Result<(), PlanRegistryError>>(plan, |layer| {
        let scope = layer.scope().clone();
        for (export_name, export) in layer.exports() {
            let Some(reference) = export.request_boundary_ref() else {
                continue;
            };
            let graph = graph.get_or_init(|| {
                crate::run_graph::flatten(plan)
                    .map_err(|error| RequestExportInvalidity::PortResolution(error.to_string()))
            });
            let ingress = graph
                .as_ref()
                .map_err(Clone::clone)
                .and_then(|graph| request_export_ingress_resolved(plan, graph, &reference))
                .map_err(|reason| PlanRegistryError::InvalidRequestExport {
                    export: export_name.clone(),
                    actor: reference.actor().clone(),
                    port: reference.port().clone(),
                    reason,
                })?;
            if let Some(output) = output.as_deref_mut() {
                std::sync::Arc::get_mut(&mut output.requests)
                    .expect("request output is built before sharing")
                    .insert((scope.clone(), export_name.clone()), ingress);
            }
        }
        for child in layer.into_scopes().into_values() {
            child?;
        }
        Ok(())
    })
}

fn request_export_ingress_resolved(
    plan: &AuthoredProjection,
    graph: &crate::run_graph::RunGraph,
    reference: &BoundaryPortRef,
) -> Result<RequestExportIngress, RequestExportInvalidity> {
    let actor = reference.actor();
    let port = reference.port();
    let boundaries = AuthoredBoundaries::of(plan);
    if let Some(inner) = boundaries.reference(actor, port, Side::Inlet) {
        let declaration = graph
            .actors()
            .get(inner.actor())
            .ok_or(RequestExportInvalidity::UnknownActor)?;
        let actor_type = *declaration.domain().actor_type();
        let spec = get(actor_type);
        let Some(boundary) = spec.boundary() else {
            return Err(RequestExportInvalidity::NotInputBoundary);
        };
        if boundary.direction() != circular_protocol::boundary_port::BoundaryPortDirection::Inlet
            || boundary.side() != Side::Outlet
        {
            return Err(RequestExportInvalidity::NotInputBoundary);
        }
        let ports = resolve_ports(
            Some(inner.actor()),
            actor_type,
            declaration.domain().config(),
            BoundaryActorGeneration::new(declaration.authored_generation()),
        )
        .map_err(|error| RequestExportInvalidity::PortResolution(error.to_string()))?;
        let available = ports
            .outlets()
            .iter()
            .map(|candidate| candidate.id().clone())
            .collect::<Vec<_>>();
        return available
            .iter()
            .any(|candidate| candidate == inner.port())
            .then(|| RequestExportIngress::BoundarySource {
                emitter: inner.actor().clone(),
                outlet: inner.port().clone(),
            })
            .ok_or_else(|| RequestExportInvalidity::UnknownBoundaryOutlet {
                available: available.into_boxed_slice(),
            });
    }
    let declaration = graph
        .actors()
        .get(actor)
        .ok_or(RequestExportInvalidity::UnknownActor)?;
    let actor_type = *declaration.domain().actor_type();
    let spec = get(actor_type);
    let ports = resolve_ports(
        Some(actor),
        actor_type,
        declaration.domain().config(),
        BoundaryActorGeneration::new(declaration.authored_generation()),
    )
    .map_err(|error| RequestExportInvalidity::PortResolution(error.to_string()))?;

    if let Some(boundary) = spec.boundary() {
        if boundary.direction() != circular_protocol::boundary_port::BoundaryPortDirection::Inlet
            || boundary.side() != Side::Outlet
        {
            return Err(RequestExportInvalidity::NotInputBoundary);
        }
        let available = ports
            .outlets()
            .iter()
            .map(|candidate| candidate.id().clone())
            .collect::<Vec<_>>();
        return available
            .iter()
            .any(|candidate| candidate == port)
            .then(|| RequestExportIngress::BoundarySource {
                emitter: actor.clone(),
                outlet: port.clone(),
            })
            .ok_or_else(|| RequestExportInvalidity::UnknownBoundaryOutlet {
                available: available.into_boxed_slice(),
            });
    }

    Err(RequestExportInvalidity::NotInputBoundary)
}

#[derive(Default)]
struct AuthoredBoundaries {
    own: circular_plan::ScopeBoundary,
    declarations: BTreeMap<NamedActorId, ActorDecl>,
    refs: BTreeMap<(NamedActorId, Side, PortId), circular_plan::Endpoint>,
    by_source: BTreeMap<circular_plan::Endpoint, Vec<circular_plan::Endpoint>>,
    by_target: BTreeMap<circular_plan::Endpoint, Vec<circular_plan::Endpoint>>,
}

impl AuthoredBoundaries {
    fn of(plan: &AuthoredProjection) -> Self {
        fold_projection::<Self>(plan, |layer| {
            let own = layer.graph().declaration().boundary().clone();
            let containers = layer.actors().keys().cloned().collect::<Vec<_>>();
            let mut folded = Self {
                own,
                declarations: layer.actors().clone(),
                ..Self::default()
            };
            for edge in layer.edges().values() {
                folded
                    .by_source
                    .entry(edge.from().clone())
                    .or_default()
                    .push(edge.to().clone());
                folded
                    .by_target
                    .entry(edge.to().clone())
                    .or_default()
                    .push(edge.from().clone());
            }
            for (segment, child) in layer.into_scopes() {
                folded.declarations.extend(child.declarations);
                folded.refs.extend(child.refs);
                for (from, mut targets) in child.by_source {
                    folded
                        .by_source
                        .entry(from)
                        .or_default()
                        .append(&mut targets);
                }
                for (to, mut sources) in child.by_target {
                    folded.by_target.entry(to).or_default().append(&mut sources);
                }
                let Some(container) = containers
                    .iter()
                    .find(|actor| actor.name() == segment.name())
                    .cloned()
                else {
                    continue;
                };
                folded
                    .refs
                    .extend(child.own.inlets().iter().map(|(outer, inner)| {
                        (
                            (container.clone(), Side::Inlet, outer.clone()),
                            inner.clone(),
                        )
                    }));
                folded
                    .refs
                    .extend(child.own.outlets().iter().map(|(outer, inner)| {
                        (
                            (container.clone(), Side::Outlet, outer.clone()),
                            inner.clone(),
                        )
                    }));
            }
            folded
        })
    }

    fn reference(
        &self,
        container: &NamedActorId,
        port: &PortId,
        side: Side,
    ) -> Option<circular_plan::Endpoint> {
        self.refs
            .get(&(container.clone(), side, port.clone()))
            .cloned()
    }

    fn sources(&self, target: &circular_plan::Endpoint) -> &[circular_plan::Endpoint] {
        self.by_target.get(target).map_or(&[], Vec::as_slice)
    }

    fn targets(&self, source: &circular_plan::Endpoint) -> &[circular_plan::Endpoint] {
        self.by_source.get(source).map_or(&[], Vec::as_slice)
    }
}

pub fn observation_mount_actor(
    plan: &AuthoredProjection,
    actor: &NamedActorId,
    port: &PortId,
) -> Result<NamedActorId, circular_protocol::boundary_port::BoundaryActivationRejection> {
    use circular_protocol::boundary_port::BoundaryActivationRejection;

    crate::run_graph::flatten(plan)
        .map_err(|_| BoundaryActivationRejection::BoundaryLeafUnresolved)?;
    let boundaries = AuthoredBoundaries::of(plan);
    let declarations = &boundaries.declarations;
    let mut output = if let Some(inner) = boundaries.reference(actor, port, Side::Outlet) {
        inner
    } else {
        let Some(declaration) = declarations.get(actor) else {
            return Err(BoundaryActivationRejection::BoundaryLeafUnresolved);
        };
        if declaration.domain().actor_type() != &ActorType::Output {
            return Ok(actor.clone());
        }
        circular_plan::Endpoint::new(actor.clone(), port.clone())
    };
    for _ in 0..=declarations.len() {
        let Some(declaration) = declarations.get(output.actor()) else {
            return Err(BoundaryActivationRejection::BoundaryLeafUnresolved);
        };
        if !declaration.domain().actor_type().is_container() {
            if declaration.domain().actor_type() == &ActorType::Output {
                let ports = resolve_ports(
                    Some(output.actor()),
                    ActorType::Output,
                    declaration.domain().config(),
                    BoundaryActorGeneration::new(declaration.authored_generation()),
                )
                .map_err(|_| BoundaryActivationRejection::BoundaryLeafUnresolved)?;
                if !ports
                    .inlets()
                    .iter()
                    .any(|candidate| candidate.id() == output.port())
                {
                    return Err(BoundaryActivationRejection::BoundaryLeafUnresolved);
                }
            }
            return Ok(output.actor().clone());
        }
        output = boundaries
            .reference(output.actor(), output.port(), Side::Outlet)
            .ok_or(BoundaryActivationRejection::BoundaryLeafUnresolved)?;
    }
    Err(BoundaryActivationRejection::BoundaryLeafUnresolved)
}

#[must_use]
pub fn boundary_direction(
    actor_type: ActorType,
) -> Option<circular_protocol::boundary_port::BoundaryPortDirection> {
    get(actor_type).boundary().map(BoundaryPortRule::direction)
}

pub fn request_mount_actors(
    plan: &AuthoredProjection,
    actor: &NamedActorId,
    port: &PortId,
) -> Result<Vec<NamedActorId>, circular_protocol::boundary_port::BoundaryActivationRejection> {
    use circular_protocol::boundary_port::BoundaryActivationRejection;

    let graph = crate::run_graph::flatten(plan)
        .map_err(|_| BoundaryActivationRejection::BoundaryLeafUnresolved)?;
    let boundaries = AuthoredBoundaries::of(plan);
    let input = if let Some(inner) = boundaries.reference(actor, port, Side::Inlet) {
        inner
    } else {
        let Some(declaration) = graph.actors().get(actor) else {
            return Err(BoundaryActivationRejection::BoundaryLeafUnresolved);
        };
        let actor_type = *declaration.domain().actor_type();
        if boundary_direction(actor_type)
            != Some(circular_protocol::boundary_port::BoundaryPortDirection::Inlet)
        {
            return Err(BoundaryActivationRejection::BoundaryLeafUnresolved);
        }
        let ports = resolve_ports(
            Some(actor),
            actor_type,
            declaration.domain().config(),
            BoundaryActorGeneration::new(declaration.authored_generation()),
        )
        .map_err(|_| BoundaryActivationRejection::BoundaryLeafUnresolved)?;
        if !ports
            .outlets()
            .iter()
            .any(|candidate| candidate.id() == port)
        {
            return Err(BoundaryActivationRejection::BoundaryLeafUnresolved);
        }
        circular_plan::Endpoint::new(actor.clone(), port.clone())
    };

    Ok(vec![input.actor().clone()])
}

#[derive(Debug, Default)]
struct ScopeSubstitution(BTreeMap<NamedActorId, Substitution>);

impl ScopeSubstitution {
    fn at(&self, actor: &NamedActorId) -> Option<&Substitution> {
        self.0.get(actor)
    }

    fn entry(&mut self, actor: &NamedActorId) -> &mut Substitution {
        self.0.entry(actor.clone()).or_default()
    }

    fn ground(&self, actor: &NamedActorId, flow: &Flow) -> Option<GroundFlow> {
        match self.at(actor) {
            Some(environment) => environment.ground(flow),
            None => Substitution::new().ground(flow),
        }
    }
}

fn shape_contains_any(shape: &circular_actors::Shape) -> bool {
    let mut pending = vec![shape];
    while let Some(shape) = pending.pop() {
        match shape {
            circular_actors::Shape::Any => return true,
            circular_actors::Shape::Array(item) => pending.push(item),
            circular_actors::Shape::Object { fields, .. } => {
                pending.extend(fields.as_slice().iter().map(|(_, item)| item))
            }
            circular_actors::Shape::Base(_) | circular_actors::Shape::Var(_) => {}
        }
    }
    false
}

struct EdgePreprocessTypes {
    program: crate::inlet_preprocess::CompiledPreprocess,
    ports: Vec<(Flow, Flow)>,
}

impl EdgePreprocessTypes {
    fn resolve(edge: &EdgeDecl) -> Result<Self, PlanRegistryError> {
        let id =
            circular_plan::EdgeId::declared(edge.from().clone(), edge.to().clone(), edge.ordinal());
        let program =
            crate::inlet_preprocess::CompiledPreprocess::compile(&id, edge.attrs().preprocess())
                .map_err(|error| match error {
                    crate::run_graph::GraphError::PreprocessInvariant { step, detail, .. } => {
                        PlanRegistryError::PreprocessInvariant {
                            edge: Box::new(DeclaredEdgeId::derive(
                                edge.from().clone(),
                                edge.to().clone(),
                                edge.ordinal(),
                            )),
                            step,
                            detail,
                        }
                    }
                    _ => unreachable!("preprocess compilation returns the indexed invariant error"),
                })?;
        let ports = program.steps().iter().map(|step| step.flows()).collect();
        Ok(Self { program, ports })
    }

    fn source_constraint<'a>(&'a self, destination: &'a Flow) -> Option<&'a Flow> {
        use crate::inlet_preprocess::CompiledStep;
        for (step, (input, _)) in self.program.steps().iter().zip(&self.ports) {
            match step.as_ref() {
                CompiledStep::Filter(_) => {}
                CompiledStep::Map(snippet) => {
                    let produced =
                        snippet.output_shape(&circular_actors::map_config::shape_env(input.item()));
                    if circular_actors::types::from_unnamed_shape(&produced) != *input.item() {
                        return None;
                    }
                }
                CompiledStep::Bang | CompiledStep::Parse(..) | CompiledStep::Flatten(..) => {
                    return Some(input);
                }
            }
        }
        Some(destination)
    }

    fn symbolic_output(&self, source: &Flow) -> Option<GroundFlow> {
        use crate::inlet_preprocess::CompiledStep;
        let mut current = source.clone();
        let mut unresolved_any = false;
        for (step, (_, output)) in self.program.steps().iter().zip(&self.ports) {
            current = match step.as_ref() {
                CompiledStep::Map(snippet) => {
                    let unknown_input = unresolved_any || current.variables().next().is_some();
                    let shape = circular_actors::types::from_unnamed_shape(
                        &snippet
                            .output_shape(&circular_actors::map_config::shape_env(current.item())),
                    );
                    unresolved_any = unknown_input
                        && snippet.references_input(circular_actors::map_config::EVENT_BINDING)
                        && shape_contains_any(&shape);
                    match output {
                        Flow::Stream(_) => Flow::Stream(shape),
                        Flow::Signal { rate, .. } => Flow::Signal {
                            item: shape,
                            rate: rate.clone(),
                        },
                    }
                }
                CompiledStep::Filter(_) => current,
                CompiledStep::Bang | CompiledStep::Parse(..) | CompiledStep::Flatten(..) => {
                    unresolved_any = false;
                    output.clone()
                }
            };
        }
        if unresolved_any {
            return None;
        }
        GroundFlow::try_new(current).ok()
    }

    fn output(
        &self,
        edge: &DeclaredEdgeId,
        source: &GroundFlow,
        dynamic_entry: bool,
    ) -> Result<GroundFlow, PlanRegistryError> {
        use crate::inlet_preprocess::CompiledStep;
        let mut current = source.clone();
        for (index, (step, (input, output))) in
            self.program.steps().iter().zip(&self.ports).enumerate()
        {
            let mut environment = Substitution::new();
            let item =
                GroundShape::try_new(current.as_flow().item().clone()).expect("ground flow item");
            for variable in input.variables() {
                environment
                    .insert(variable.clone(), item.clone())
                    .expect("one input shape per stage");
            }
            let required = environment
                .ground(input)
                .expect("retained unary input has no rate variable");
            if !(index == 0 && dynamic_entry || connectable(&current, &required)) {
                return Err(PlanRegistryError::IncompatibleConnection(Box::new(
                    IncompatibleConnectionReport {
                        edge: edge.clone(),
                        from: flow_text(&current),
                        to: flow_text(&required),
                        hint: format!("preprocess step {index} input"),
                    },
                )));
            }
            if let CompiledStep::Map(snippet) | CompiledStep::Filter(snippet) = step.as_ref()
                && let Some(split) = snippet.kind_split(&circular_actors::map_config::shape_env(
                    current.as_flow().item(),
                ))
            {
                return Err(PlanRegistryError::PreprocessInvariant {
                    edge: Box::new(edge.clone()),
                    step: index,
                    detail: format!("it cannot produce a value: {split}"),
                });
            }
            current = match step.as_ref() {
                CompiledStep::Map(snippet) => {
                    let shape = circular_actors::map_config::output_shape_of(
                        snippet,
                        current.as_flow().item(),
                    )
                    .ok_or_else(|| PlanRegistryError::PreprocessInvariant {
                        edge: Box::new(edge.clone()),
                        step: index,
                        detail: "its snippet output shape is unresolved".to_owned(),
                    })?;
                    let flow = match output {
                        Flow::Stream(_) => Flow::Stream(shape.as_shape().clone()),
                        Flow::Signal { rate, .. } => Flow::Signal {
                            item: shape.as_shape().clone(),
                            rate: rate.clone(),
                        },
                    };
                    GroundFlow::try_new(flow).expect("wire constructor and ground snippet result")
                }
                CompiledStep::Filter(_) => current,
                CompiledStep::Bang | CompiledStep::Parse(..) | CompiledStep::Flatten(..) => {
                    environment
                        .ground(output)
                        .expect("retained fixed output is ground")
                }
            };
        }
        Ok(current)
    }
}

fn substitute_scope(
    edges: &BTreeMap<DeclaredEdgeId, EdgeDecl>,
    actors: &BTreeMap<NamedActorId, ActorDecl>,
    containers: &BTreeMap<circular_plan::Name, ContainerPorts>,
) -> Result<ScopeSubstitution, PlanRegistryError> {
    let preprocess = edges
        .iter()
        .map(|(id, edge)| Ok((id, EdgePreprocessTypes::resolve(edge)?)))
        .collect::<Result<BTreeMap<_, _>, PlanRegistryError>>()?;
    let mut environment = ScopeSubstitution::default();
    let mut queued: BTreeSet<&DeclaredEdgeId> = edges.keys().collect();
    let mut queue: VecDeque<&DeclaredEdgeId> = queued.iter().copied().collect();

    let mut seed_snippet_outputs = false;
    loop {
        let Some(edge_id) = queue.pop_front() else {
            if seed_snippet_outputs {
                break;
            }
            seed_snippet_outputs = true;
            queued.extend(edges.keys());
            queue.extend(queued.iter().copied());
            continue;
        };
        queued.remove(edge_id);
        let edge = &edges[edge_id];
        let from = resolve_endpoint(
            actors,
            containers,
            edge.from().actor(),
            Side::Outlet,
            edge.from().port(),
        )?;
        let to = resolve_endpoint(
            actors,
            containers,
            edge.to().actor(),
            Side::Inlet,
            edge.to().port(),
        )?;

        let program = &preprocess[edge_id];
        let mut emitted = environment
            .ground(edge.from().actor(), &from.flow)
            .map(|ground| program.output(edge_id, &ground, from.undeclared_any))
            .transpose()?;
        if emitted.is_none() && seed_snippet_outputs {
            let source = environment.at(edge.from().actor()).map_or_else(
                || from.flow.clone(),
                |substitution| from.flow.substitute(substitution),
            );
            emitted = program.symbolic_output(&source);
        }
        let reverse = program.source_constraint(&to.flow);
        let mut progressed = false;
        for (ground_side, variable_side) in [
            (
                (
                    &from.flow,
                    edge.from().actor(),
                    Side::Outlet,
                    edge.from().port(),
                ),
                (&to.flow, edge.to().actor()),
            ),
            (
                (&to.flow, edge.to().actor(), Side::Inlet, edge.to().port()),
                (&from.flow, edge.from().actor()),
            ),
        ] {
            let (_ground_flow, ground_actor, ground_side_name, ground_port) = ground_side;
            let (variable_flow, variable_actor) = variable_side;
            let ground = if ground_side_name == Side::Outlet {
                emitted.clone()
            } else {
                reverse.and_then(|flow| environment.ground(ground_actor, flow))
            };
            let Some(ground) = ground else {
                continue;
            };
            let Some(shape) = GroundShape::try_new(ground.as_flow().item().clone()).ok() else {
                continue;
            };
            let unresolved = match environment.at(variable_actor) {
                Some(current) => variable_flow.substitute(current),
                None => variable_flow.clone(),
            };
            for variable in unresolved.variables().cloned().collect::<Vec<Name>>() {
                match environment
                    .entry(variable_actor)
                    .insert(variable.clone(), shape.clone())
                {
                    Ok(assigned) => progressed |= assigned.is_fresh(),
                    Err(conflict) => {
                        return Err(PlanRegistryError::TypeVarConflict(Box::new(
                            TypeVarConflictReport {
                                variable,
                                existing: conflict.existing().clone(),
                                incoming: conflict.incoming().clone(),
                                owner: variable_actor.clone(),
                                actor: ground_actor.clone(),
                                side: ground_side_name,
                                port: ground_port.clone(),
                            },
                        )));
                    }
                }
            }
        }

        if progressed {
            for other in edges.keys() {
                if queued.insert(other) {
                    queue.push_back(other);
                }
            }
        }
    }

    Ok(environment)
}

fn validate_edges(
    edges: &BTreeMap<DeclaredEdgeId, circular_plan::EdgeDecl>,
    actors: &BTreeMap<NamedActorId, ActorDecl>,
    containers: &BTreeMap<circular_plan::Name, ContainerPorts>,
) -> Result<(), PlanRegistryError> {
    let environment = substitute_scope(edges, actors, containers)?;

    let mut incidences =
        BTreeMap::<(NamedActorId, Side, PortId), (usize, circular_actors::Arity)>::new();

    for (edge_id, edge) in edges {
        let from = resolve_endpoint(
            actors,
            containers,
            edge.from().actor(),
            Side::Outlet,
            edge.from().port(),
        )?;
        let to = resolve_endpoint(
            actors,
            containers,
            edge.to().actor(),
            Side::Inlet,
            edge.to().port(),
        )?;
        if !from.undeclared_any {
            let from_ty = environment
                .ground(edge.from().actor(), &from.flow)
                .ok_or_else(|| PlanRegistryError::UnassignedTypeVar {
                    actor: edge.from().actor().clone(),
                    side: Side::Outlet,
                    port: edge.from().port().clone(),
                })?;
            let from_ty = EdgePreprocessTypes::resolve(edge)?.output(edge_id, &from_ty, false)?;
            let to_ty = environment
                .ground(edge.to().actor(), &to.flow)
                .ok_or_else(|| PlanRegistryError::UnassignedTypeVar {
                    actor: edge.to().actor().clone(),
                    side: Side::Inlet,
                    port: edge.to().port().clone(),
                })?;
            if !connectable(&from_ty, &to_ty) {
                let from = flow_text(&from_ty);
                let to = flow_text(&to_ty);
                let hint = any_hint(&from, &to);
                return Err(PlanRegistryError::IncompatibleConnection(Box::new(
                    IncompatibleConnectionReport {
                        edge: edge_id.clone(),
                        from,
                        to,
                        hint,
                    },
                )));
            }
        }
        for (actor, side, port, arity) in [
            (
                edge.from().actor().clone(),
                Side::Outlet,
                edge.from().port().clone(),
                from.arity,
            ),
            (
                edge.to().actor().clone(),
                Side::Inlet,
                edge.to().port().clone(),
                to.arity,
            ),
        ] {
            let (count, stored_arity) = incidences
                .entry((actor.clone(), side, port.clone()))
                .or_insert((0, arity));
            *count += 1;
            debug_assert_eq!(*stored_arity, arity);
            if arity == circular_actors::Arity::One && *count > 1 {
                return Err(PlanRegistryError::ArityExceeded { actor, side, port });
            }
        }
    }
    Ok(())
}

fn resolve_ports(
    actor: Option<&NamedActorId>,
    actor_type: ActorType,
    config: &circular_plan::Config,
    generation: BoundaryActorGeneration,
) -> Result<PortSet, PlanRegistryError> {
    let spec = get(actor_type);
    if spec.boundary().is_none()
        && spec.ports().dynamic().is_empty()
        && spec.ports().choices().is_empty()
    {
        return Ok(spec.ports().fixed().clone());
    }
    let value = fold_actor_config(actor, actor_type, config)?;
    let boundary_id = spec
        .boundary()
        .map(|boundary| {
            let actor = actor.ok_or_else(|| PlanRegistryError::PortExpansion {
                actor: None,
                actor_type,
                detail:
                    "registered boundary port resolution requires the exact plan actor identity"
                        .to_owned(),
            })?;
            let key = circular_runtime::product_identity::wire_named_actor(actor);
            BoundaryPortId::derive(boundary.direction(), &key, generation)
                .map(BoundaryPortId::into_port_id)
                .map_err(|error| PlanRegistryError::PortExpansion {
                    actor: Some(actor.clone()),
                    actor_type,
                    detail: error.to_string(),
                })
        })
        .transpose()?;
    spec.expand_ports_at(&value, boundary_id)
        .map_err(|error| PlanRegistryError::PortExpansion {
            actor: actor.cloned(),
            actor_type,
            detail: circular_actors::config::config_rejection(
                actor.map_or_else(|| actor_type.to_string(), ToString::to_string),
                "config",
                Some(value.value()),
                error,
            ),
        })
}

fn fold_actor_config(
    actor: Option<&NamedActorId>,
    actor_type: ActorType,
    config: &circular_plan::Config,
) -> Result<circular_runtime::FoldedConfig, PlanRegistryError> {
    fold_config(actor_type, config).map_err(|error| PlanRegistryError::ConfigFold {
        actor_type,
        detail: crate::activation_config::config_rejection(
            actor.map_or_else(|| actor_type.to_string(), ToString::to_string),
            "config",
            config,
            error,
        ),
        admission: None,
    })
}

/// One complete registry-resolved port identity and its authored Flow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclaredPort {
    id: PortId,
    flow: Flow,
}

impl DeclaredPort {
    #[must_use]
    pub const fn id(&self) -> &PortId {
        &self.id
    }

    #[must_use]
    pub const fn flow(&self) -> &Flow {
        &self.flow
    }
}

/// The complete declared port set for one authored actor declaration.
///
/// Dynamic rules and the derived `_error` outlet are resolved by the same
/// registration path used for plan admission. No consumer has to infer a Flow
/// from an id or re-run the derivation rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclaredPorts {
    inlets: Box<[DeclaredPort]>,
    outlets: Box<[DeclaredPort]>,
}

impl DeclaredPorts {
    #[must_use]
    pub fn inlets(&self) -> &[DeclaredPort] {
        &self.inlets
    }

    #[must_use]
    pub fn outlets(&self) -> &[DeclaredPort] {
        &self.outlets
    }
}

pub fn resolve_declared_ports(
    actor_type: ActorType,
    config: &circular_plan::Config,
) -> Result<DeclaredPorts, PlanRegistryError> {
    resolve_declared_ports_inner(None, actor_type, config, BoundaryActorGeneration::initial())
}

/// Resolve declared ports for an exact authored actor identity. Boundary
/// registrations require this entrypoint; ordinary registrations produce the
/// same facts as [`resolve_declared_ports`].
pub fn resolve_declared_ports_at(
    actor: &PlanActorKey,
    actor_type: ActorType,
    config: &circular_plan::Config,
    generation: BoundaryActorGeneration,
) -> Result<DeclaredPorts, PlanRegistryError> {
    let plan_actor = named_actor_from_authored_key(actor)?;
    resolve_declared_ports_inner(Some(&plan_actor), actor_type, config, generation)
}

/// Resolve the registry-owned ports for an actor that already lives in a
/// physical [`AuthoredProjection`].  Runtime boundary routing uses this entrypoint so the
/// exact same `(actor identity, actor type, config)` facts that admitted the
/// plan also authorize the boundary source endpoint; it must not derive an
/// id from a label or accept an arbitrary reserved-looking port name.
pub(crate) fn resolve_declared_ports_for_plan_actor(
    actor: &NamedActorId,
    actor_type: ActorType,
    config: &circular_plan::Config,
    generation: BoundaryActorGeneration,
) -> Result<DeclaredPorts, PlanRegistryError> {
    resolve_declared_ports_inner(Some(actor), actor_type, config, generation)
}

fn resolve_declared_ports_inner(
    actor: Option<&NamedActorId>,
    actor_type: ActorType,
    config: &circular_plan::Config,
    generation: BoundaryActorGeneration,
) -> Result<DeclaredPorts, PlanRegistryError> {
    let ports = resolve_ports(actor, actor_type, config, generation)?;
    let inlets = ports
        .inlets()
        .iter()
        .map(|port| DeclaredPort {
            id: port.id().clone(),
            flow: port.ty().clone(),
        })
        .collect::<Vec<_>>();
    let mut outlets = ports
        .outlets()
        .iter()
        .map(|port| DeclaredPort {
            id: port.id().clone(),
            flow: port.ty().clone(),
        })
        .collect::<Vec<_>>();
    if let Some((flow, _arity)) =
        circular_actors::derived_error_outlet(actor_type, Side::Outlet, "_error")
    {
        let id = PortId::try_new("_error".to_owned()).expect("registered derived port is valid");
        if !outlets.iter().any(|port| port.id == id) {
            outlets.push(DeclaredPort { id, flow });
        }
    }
    Ok(DeclaredPorts {
        inlets: inlets.into_boxed_slice(),
        outlets: outlets.into_boxed_slice(),
    })
}

fn named_actor_from_authored_key(actor: &PlanActorKey) -> Result<NamedActorId, PlanRegistryError> {
    circular_runtime::product_identity::named_actor_from_wire(actor).map_err(|error| {
        PlanRegistryError::PortExpansion {
            actor: None,
            actor_type: ActorType::Input,
            detail: format!("authored boundary scope: {error}"),
        }
    })
}

fn any_hint(from: &str, to: &str) -> String {
    if from.contains("Any") && !to.contains("Any") {
        "the producing side is `Any` — either the element published it that way, or the \
         branches of a condition in the authored snippet produced different shapes and \
         were joined. If it is the latter, the branches must agree down to the field \
         kinds: matching only the names folds them again"
            .to_owned()
    } else {
        String::new()
    }
}

fn resolve_endpoint(
    actors: &BTreeMap<NamedActorId, ActorDecl>,
    containers: &BTreeMap<circular_plan::Name, ContainerPorts>,
    actor: &NamedActorId,
    side: Side,
    port: &PortId,
) -> Result<EndpointFlow, PlanRegistryError> {
    if let Some(ports) = containers.get(actor.name())
        && let Some(resolved) = ports.get(&(side, port.clone()))
    {
        return Ok(resolved.clone());
    }
    let declaration = actors.get(actor).expect(
        "AuthoredProjectionBuilder validates that every edge endpoint is an actor in this layer",
    );
    let actor_type = *declaration.domain().actor_type();
    let config = declaration.domain().config();
    let ports = resolve_ports(
        Some(actor),
        actor_type,
        config,
        BoundaryActorGeneration::new(declaration.authored_generation()),
    )
    .map_err(|error| match error {
        PlanRegistryError::PortExpansion {
            actor: None,
            actor_type,
            detail,
        } => PlanRegistryError::PortExpansion {
            actor: Some(actor.clone()),
            actor_type,
            detail,
        },
        other => other,
    })?;
    let found = match side {
        Side::Inlet => ports
            .inlets()
            .iter()
            .find(|spec| spec.id() == port)
            .map(|spec| (spec.ty().clone(), spec.arity())),
        Side::Outlet => ports
            .outlets()
            .iter()
            .find(|spec| spec.id() == port)
            .map(|spec| (spec.ty().clone(), spec.arity())),
    };
    let spec = get(actor_type);
    let undeclared_any = match (&found, spec.boundary()) {
        (Some(_), Some(rule)) if rule.side() == side => {
            let config = fold_actor_config(Some(actor), actor_type, config)?;
            spec.boundary_undeclared_any(&config)
        }
        _ => false,
    };
    let (flow, arity) = found
        .or_else(|| circular_actors::derived_error_outlet(actor_type, side, port.as_str()))
        .ok_or_else(|| PlanRegistryError::UnknownPort {
            actor: actor.clone(),
            side,
            port: port.clone(),
            available: circular_actors::available_ports(&ports, side),
        })?;
    Ok(EndpointFlow {
        flow,
        arity,
        undeclared_any,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring_assembly::projection::AuthoredProjectionBuilder;
    use circular_plan::{
        ActorDomain, ActorFlags, Config, Delivery, EdgeAttrs, Endpoint, Name,
        NonContainerActorDecl, PipelineActorDecl, WirePolicy,
    };

    fn plan_name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    fn declaration(actor_type: ActorType) -> NonContainerActorDecl {
        NonContainerActorDecl::try_new(
            ActorDomain::new(actor_type, Config::default()),
            ActorFlags::default(),
        )
        .expect("test registration is not a container")
    }

    #[test]
    fn registered_fixture_plan_resolves_but_published_profile_rejects_it() {
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .add_actor(plan_name("map"), declaration(ActorType::FixtureMap))
            .unwrap();
        let plan = builder.finish().unwrap();
        resolve_plan(&plan, RegistryProfile::Fixture).unwrap();
        assert!(matches!(
            resolve_plan(&plan, RegistryProfile::Published),
            Err(PlanRegistryError::FixtureLocalActorInPublishedPlan { .. })
        ));
    }

    #[test]
    fn a_container_port_absent_from_the_boundary_is_still_unknown() {
        let mut builder = AuthoredProjectionBuilder::new();
        let upstream = builder
            .add_actor(plan_name("upstream"), declaration(ActorType::FixtureMap))
            .expect("root actor");
        builder
            .enter_scope(plan_name("cell"), PipelineActorDecl::default())
            .expect("child scope");
        builder.exit_scope().expect("to the root");

        let container =
            circular_plan::NamedActorId::new(circular_plan::ScopeId::root(), plan_name("cell"));
        let policy = WirePolicy::new(Delivery::Lossless, None);
        builder
            .add_edge(
                Endpoint::new(upstream, PortId::try_new("out").unwrap()),
                Endpoint::new(container, PortId::try_new("nowhere").unwrap()),
                0,
                EdgeAttrs::new(circular_core::Ticks::ZERO, policy),
            )
            .expect("edge");
        let plan = builder.finish().expect("container without a boundary");

        assert!(matches!(
            resolve_plan(&plan, RegistryProfile::Fixture),
            Err(PlanRegistryError::UnknownPort { .. })
        ));
    }

    #[test]
    fn canonical_pipeline_container_is_admitted_by_the_published_profile() {
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .enter_scope(plan_name("child"), PipelineActorDecl::default())
            .expect("canonical child scope");
        builder.exit_scope().expect("return to root");
        let plan = builder.finish().expect("nested plan");

        resolve_plan(&plan, RegistryProfile::Published)
            .expect("the canonical pipeline_actor is a published registration");
    }

    #[test]
    fn plan_builder_accepts_an_unknown_port_but_registry_admission_rejects_it() {
        let mut builder = AuthoredProjectionBuilder::new();
        let from = builder
            .add_actor(plan_name("from"), declaration(ActorType::EditableCounter))
            .unwrap();
        let to = builder
            .add_actor(plan_name("to"), declaration(ActorType::EditableAux))
            .unwrap();
        builder
            .add_edge(
                Endpoint::new(from, PortId::try_new("missing").unwrap()),
                Endpoint::new(to, PortId::try_new("in").unwrap()),
                0,
                EdgeAttrs::new(
                    circular_core::Ticks::ZERO,
                    WirePolicy::new(Delivery::Lossless, None),
                ),
            )
            .expect(
                "AuthoredProjectionBuilder validates identity syntax, not the registered port set",
            );
        let plan = builder.finish().unwrap();
        assert!(matches!(
            resolve_plan(&plan, RegistryProfile::Fixture),
            Err(PlanRegistryError::UnknownPort {
                side: Side::Outlet,
                ..
            })
        ));
    }
}

#[cfg(test)]
mod type_var_tests {
    use super::*;
    use crate::authoring_assembly::projection::AuthoredProjectionBuilder;
    use circular_actors::{BaseShape, Shape};
    use circular_plan::{
        ActorDomain, ActorFlags, Config, Delivery, EdgeAttrs, Endpoint, Name,
        NonContainerActorDecl, PipelineActorDecl, PositiveCapacity, WirePolicy,
    };

    fn declaration(actor_type: ActorType) -> NonContainerActorDecl {
        NonContainerActorDecl::try_new(
            ActorDomain::new(actor_type, Config::default()),
            ActorFlags::default(),
        )
        .expect("a registered element is not a container")
    }

    fn policy() -> WirePolicy {
        WirePolicy::new(Delivery::Lossless, Some(PositiveCapacity::new(1).unwrap()))
    }

    fn port(actor_type: ActorType, side: Side) -> PortId {
        default_plan_port(actor_type, side).unwrap()
    }

    fn null_boundary(builder: &mut AuthoredProjectionBuilder) -> NamedActorId {
        let boundary = builder
            .enter_scope(Name::from_normalized("b"), PipelineActorDecl::default())
            .unwrap();
        builder.exit_scope().unwrap();
        boundary
    }

    fn null_outlet() -> PortId {
        PortId::try_new("null").unwrap()
    }

    fn null_boundary_ports() -> BTreeMap<Name, ContainerPorts> {
        BTreeMap::from([(
            Name::from_normalized("b"),
            BTreeMap::from([(
                (Side::Outlet, null_outlet()),
                EndpointFlow {
                    flow: Flow::Stream(Shape::Base(BaseShape::Null)),
                    arity: circular_actors::Arity::Many,
                    undeclared_any: false,
                },
            )]),
        )])
    }

    fn validate_null_boundary_fixture(plan: &AuthoredProjection) -> Result<(), PlanRegistryError> {
        validate_edges(
            plan.graph().edges(),
            plan.graph().actors(),
            &null_boundary_ports(),
        )
    }

    #[test]
    fn a_ground_upstream_assigns_the_downstream_type_variable() {
        let mut builder = AuthoredProjectionBuilder::new();
        let boundary = null_boundary(&mut builder);
        let tap = builder
            .add_actor(Name::from_normalized("t"), declaration(ActorType::Tap))
            .unwrap();
        builder
            .add_edge(
                Endpoint::new(boundary, null_outlet()),
                Endpoint::new(tap, port(ActorType::Tap, Side::Inlet)),
                0,
                EdgeAttrs::new(circular_core::Ticks::ZERO, policy()),
            )
            .unwrap();
        let plan = builder.finish().unwrap();
        assert_eq!(validate_null_boundary_fixture(&plan), Ok(()));
    }

    #[test]
    fn a_variable_only_component_is_rejected_rather_than_guessed() {
        let mut builder = AuthoredProjectionBuilder::new();
        let first = builder
            .add_actor(Name::from_normalized("t1"), declaration(ActorType::Tap))
            .unwrap();
        let second = builder
            .add_actor(Name::from_normalized("t2"), declaration(ActorType::Tap))
            .unwrap();
        builder
            .add_edge(
                Endpoint::new(first, port(ActorType::Tap, Side::Outlet)),
                Endpoint::new(second.clone(), port(ActorType::Tap, Side::Inlet)),
                0,
                EdgeAttrs::new(circular_core::Ticks::ZERO, policy()),
            )
            .unwrap();
        let plan = builder.finish().unwrap();
        assert!(matches!(
            resolve_plan(&plan, RegistryProfile::Published),
            Err(PlanRegistryError::UnassignedTypeVar { .. })
        ));
        let _ = second;
        let _ = Shape::Base(BaseShape::Null);
    }
}

#[cfg(test)]
mod dynamic_port_tests {
    use super::*;
    use crate::authoring_assembly::projection::AuthoredProjectionBuilder;
    use circular_core::{Boundary, Ceilings, Value, encode};
    use circular_plan::{
        ActorDomain, ActorFlags, Config, ConfigValue, Delivery, EdgeAttrs, Endpoint, Export,
        ExportName, Mount, Name, NonContainerActorDecl, OperationDecl, PositiveCapacity, Role,
        WirePolicy,
    };

    fn wire_actor(
        scope: Vec<circular_protocol::declaration_payload::ScopeSegment>,
        local: &str,
    ) -> PlanActorKey {
        PlanActorKey {
            scope,
            local: circular_protocol::declaration_payload::ActorLocal::parse(local),
        }
    }

    fn scalar(value: &Value) -> ConfigValue {
        ConfigValue::Scalar {
            tag: Name::from_normalized(crate::activation_config::CANONICAL_VALUE_TAG),
            bytes: encode(value, Ceilings::for_boundary(Boundary::Config))
                .expect("the test value fits the canonical encoding")
                .into_boxed_slice(),
        }
    }

    fn string_boundary_config() -> Config {
        Config::try_new(vec![(
            Name::from_normalized("label"),
            scalar(&Value::String("Request".to_owned())),
        )])
        .expect("boundary config name is unique")
    }

    fn declaration(actor_type: ActorType, config: Config) -> NonContainerActorDecl {
        NonContainerActorDecl::try_new(ActorDomain::new(actor_type, config), ActorFlags::default())
            .expect("a registered element is not a container")
    }

    fn policy() -> WirePolicy {
        WirePolicy::new(Delivery::Lossless, Some(PositiveCapacity::new(1).unwrap()))
    }

    #[test]
    fn an_outlet_outside_the_declared_cases_is_rejected() {
        let cases = ConfigValue::Record(
            circular_plan::ConfigRecord::try_new(vec![(
                Name::from_normalized("even"),
                scalar(&Value::int(0)),
            )])
            .expect("one key"),
        );
        let config = Config::try_new(vec![
            (Name::from_normalized("at"), ConfigValue::List(Box::new([]))),
            (Name::from_normalized("cases"), cases),
        ])
        .expect("only two keys");

        let mut builder = AuthoredProjectionBuilder::new();
        let route = builder
            .add_actor(
                Name::from_normalized("r"),
                declaration(ActorType::Route, config),
            )
            .unwrap();
        let tap = builder
            .add_actor(
                Name::from_normalized("t"),
                declaration(ActorType::Tap, Config::default()),
            )
            .unwrap();
        builder
            .add_edge(
                Endpoint::new(route, PortId::try_new("route_odd").unwrap()),
                Endpoint::new(tap, default_plan_port(ActorType::Tap, Side::Inlet).unwrap()),
                0,
                EdgeAttrs::new(circular_core::Ticks::ZERO, policy()),
            )
            .unwrap();

        let plan = builder.finish().unwrap();
        assert!(matches!(
            resolve_plan(&plan, RegistryProfile::Published),
            Err(PlanRegistryError::UnknownPort { .. })
        ));
    }

    #[test]
    fn a_config_driven_route_stands_in_a_published_plan() {
        let cases = ConfigValue::Record(
            circular_plan::ConfigRecord::try_new(vec![
                (Name::from_normalized("even"), scalar(&Value::int(0))),
                (Name::from_normalized("odd"), scalar(&Value::int(1))),
            ])
            .expect("the two keys differ"),
        );
        let config =
            Config::try_new(vec![(Name::from_normalized("cases"), cases)]).expect("only one cases");

        let mut builder = AuthoredProjectionBuilder::new();
        let bang = builder
            .add_actor(
                Name::from_normalized("b"),
                declaration(ActorType::Counter, Config::default()),
            )
            .unwrap();
        let route = builder
            .add_actor(
                Name::from_normalized("r"),
                declaration(ActorType::Route, config),
            )
            .unwrap();
        builder
            .add_edge(
                Endpoint::new(
                    bang,
                    default_plan_port(ActorType::Counter, Side::Outlet).unwrap(),
                ),
                Endpoint::new(route, PortId::try_new("event").unwrap()),
                0,
                EdgeAttrs::new(circular_core::Ticks::ZERO, policy()),
            )
            .unwrap();
        let plan = builder.finish().unwrap();

        assert_eq!(
            resolve_plan(&plan, RegistryProfile::Published),
            Ok(()),
            "passes only when dynamic port expansion and type variable substitution hold together"
        );
    }
}

#[cfg(test)]
pub(crate) fn snippet_config(key: &str, text: &str) -> circular_plan::Config {
    let bytes = circular_core::encode(
        &circular_actors::ProductValue::String(text.to_owned()),
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Config),
    )
    .expect("canonical encoding");
    circular_plan::Config::try_new(vec![(
        circular_plan::Name::from_normalized(key),
        circular_plan::ConfigValue::Scalar {
            tag: circular_plan::Name::from_normalized(
                crate::activation_config::CANONICAL_VALUE_TAG,
            ),
            bytes: bytes.into_boxed_slice(),
        },
    )])
    .expect("one config entry")
}

#[path = "actor_registry_runtime_shapes.rs"]
mod runtime_shapes;
