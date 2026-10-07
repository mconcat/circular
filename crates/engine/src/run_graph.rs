
use crate::authoring_assembly::projection::AuthoredProjection;
use crate::authoring_assembly::projection::fold_projection;
use crate::inlet_preprocess::CompiledPreprocess;
use circular_actors::Side;
use circular_plan::{
    ActorDecl, AdmittedTemplate, EdgeAttrs, EdgeId, Endpoint, InstanceKey, Name, NamedActorId,
    PortId, ScopeRole,
};
use std::collections::BTreeMap;

#[derive(Debug)]
pub enum GraphError {
    PreprocessInvariant {
        edge: Box<EdgeId>,
        step: usize,
        detail: String,
    },
    UnknownTemplate(NamedActorId),
    CellDerivation(circular_plan::CellDerivationError),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PreprocessInvariant { edge, step, detail } => write!(
                formatter,
                "admitted preprocess invariant: {edge:?} step {step}: {detail}"
            ),
            Self::UnknownTemplate(container) => write!(
                formatter,
                "minted a template absent from this graph: {container:?}"
            ),
            Self::CellDerivation(error) => write!(formatter, "cell derivation: {error:?}"),
        }
    }
}

impl std::error::Error for GraphError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunEdge {
    id: EdgeId,
    /// Template declaration identity, retained when `id` gains a cell key.
    authored: EdgeId,
    from: Endpoint,
    to: Endpoint,
    attrs: EdgeAttrs,
    preprocess: CompiledPreprocess,
    hop: CompiledPreprocess,
}

impl RunEdge {
    pub(crate) fn preprocess(&self) -> &CompiledPreprocess {
        &self.preprocess
    }
    pub(crate) fn hop(&self) -> &CompiledPreprocess {
        &self.hop
    }
    #[must_use]
    pub(crate) const fn id(&self) -> &EdgeId {
        &self.id
    }

    #[must_use]
    pub(crate) const fn from(&self) -> &Endpoint {
        &self.from
    }

    #[must_use]
    pub(crate) const fn to(&self) -> &Endpoint {
        &self.to
    }

    #[must_use]
    pub(crate) fn attrs(&self) -> EdgeAttrs {
        self.attrs.clone()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BoundaryCrossing {
    pub(crate) edge: EdgeId,
    pub(crate) authored: EdgeId,
    pub(crate) container: NamedActorId,
    pub(crate) inner: Endpoint,
    pub(crate) outer: Endpoint,
    pub(crate) attrs: EdgeAttrs,
    pub(crate) preprocess: CompiledPreprocess,
    pub(crate) hop: CompiledPreprocess,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemplateBoundEdge {
    pub(crate) edge: EdgeId,
    pub(crate) endpoint: Endpoint,
    pub(crate) side: Side,
    pub(crate) receiver: Option<Endpoint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemplateScope {
    pub(crate) container: NamedActorId,
    pub(crate) actors: BTreeMap<NamedActorId, ActorDecl>,
    pub(crate) edges: Vec<RunEdge>,
}

impl TemplateScope {}

#[derive(Clone, Debug, Default)]
pub struct RunGraph {
    actors: BTreeMap<NamedActorId, ActorDecl>,
    edges: Vec<RunEdge>,
    declared_sources: BTreeMap<EdgeId, Endpoint>,
    templates: Vec<TemplateScope>,
    scopes: circular_plan::ScopeRoleTable,
    admitted: BTreeMap<NamedActorId, AdmittedTemplate>,
    template_bound: Vec<TemplateBoundEdge>,
    inbound: Vec<BoundaryCrossing>,
    outbound: Vec<BoundaryCrossing>,
    keyed: Vec<KeyedCrossing>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct KeyedCrossing {
    pub(crate) edge: EdgeId,
    pub(crate) authored: EdgeId,
    pub(crate) outer: Endpoint,
    pub(crate) inlet: circular_runtime::KeyedInlet,
    pub(crate) key_at: circular_actors::PayloadPath,
    pub(crate) attrs: EdgeAttrs,
    pub(crate) preprocess: CompiledPreprocess,
    pub(crate) hop: CompiledPreprocess,
}

pub(crate) fn keyed_target(inlet: &circular_runtime::KeyedInlet, key: &InstanceKey) -> Endpoint {
    Endpoint::new(inlet.cell_receiver(key), inlet.receiver().port().clone())
}

impl RunGraph {
    #[must_use]
    pub(crate) const fn actors(&self) -> &BTreeMap<NamedActorId, ActorDecl> {
        &self.actors
    }

    #[must_use]
    pub(crate) fn edges(&self) -> &[RunEdge] {
        &self.edges
    }

    #[must_use]
    pub(crate) fn templates(&self) -> &[TemplateScope] {
        &self.templates
    }

    #[must_use]
    pub(crate) fn scopes(&self) -> &circular_plan::ScopeRoleTable {
        &self.scopes
    }

    pub(crate) fn admit_runtime_scope(
        &self,
        address: &circular_plan::ScopeId,
    ) -> Result<circular_plan::AdmittedScope, circular_plan::ScopeAdmissionError> {
        circular_plan::admit_runtime_scope(&self.scopes, address)
    }

    pub(crate) fn admit_template(
        &self,
        container: &circular_plan::ScopeId,
        template: &circular_plan::Name,
    ) -> Result<AdmittedTemplate, circular_plan::ScopeAdmissionError> {
        circular_plan::admit_template(&self.scopes, container, template)
    }

    #[must_use]
    pub(crate) fn template(&self, container: &NamedActorId) -> Option<&TemplateScope> {
        self.templates
            .iter()
            .find(|template| &template.container == container)
    }

    pub(crate) fn admitted_template(&self, container: &NamedActorId) -> Option<&AdmittedTemplate> {
        self.admitted.get(container)
    }

    #[must_use]
    pub(crate) fn template_bound(&self) -> &[TemplateBoundEdge] {
        &self.template_bound
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn keyed(&self) -> &[KeyedCrossing] {
        &self.keyed
    }

    #[must_use]
    pub(crate) fn outbound(&self) -> &[BoundaryCrossing] {
        &self.outbound
    }

    pub(crate) fn keyed_into<'graph>(
        &'graph self,
        container: &'graph NamedActorId,
    ) -> impl Iterator<Item = &'graph KeyedCrossing> {
        self.keyed.iter().filter(move |crossing| {
            let template = crossing.inlet.template();
            template.template() == container.name() && template.container() == container.scope()
        })
    }

    pub(crate) fn outgoing<'graph>(
        &'graph self,
        source: &'graph Endpoint,
    ) -> impl Iterator<Item = &'graph RunEdge> {
        self.edges.iter().filter(move |edge| &edge.from == source)
    }
}

pub(crate) fn container_inlet_edge(crossing: &KeyedCrossing, container: &NamedActorId) -> EdgeId {
    realized_edge(
        &crossing.authored,
        &crossing.outer,
        &Endpoint::new(
            container.clone(),
            PortId::try_new("event").expect("port name"),
        ),
    )
}

pub(crate) const fn ordinal_of(edge: &EdgeId) -> u16 {
    match edge {
        EdgeId::Declared { ordinal, .. } => *ordinal,
        EdgeId::Outcome { .. } => 0,
    }
}

impl RunGraph {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundaryLeaf {
    endpoint: Endpoint,
    hop: CompiledPreprocess,
}

impl BoundaryLeaf {
    fn at(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            hop: CompiledPreprocess::default(),
        }
    }
}

#[derive(Debug, Default)]
struct FoldedScope {
    role: ScopeRole,
    ports: BTreeMap<(Side, PortId), Vec<BoundaryLeaf>>,
    flat: RunGraph,
}

pub(crate) fn boundary_inner_side(inner: Option<&ActorDecl>, side: Side) -> Side {
    let (structural, facing) = match side {
        Side::Inlet => (circular_plan::ActorType::Input, Side::Outlet),
        Side::Outlet => (circular_plan::ActorType::Output, Side::Inlet),
    };
    if inner.is_some_and(|declaration| declaration.domain().actor_type() == &structural) {
        return facing;
    }
    side
}

pub(crate) fn container_boundary_inlet(
    container: &ActorDecl,
    child_boundary_inlets: usize,
    derived: &PortId,
) -> PortId {
    if child_boundary_inlets != 1 {
        return derived.clone();
    }
    let registration = circular_actors::get(*container.domain().actor_type());
    let [registered] = registration.ports().fixed().inlets() else {
        return derived.clone();
    };
    registered.id().clone()
}

pub(crate) fn container_boundary_outlet(
    container: &ActorDecl,
    child_boundary_outlets: usize,
    derived: &PortId,
) -> PortId {
    if child_boundary_outlets != 1 {
        return derived.clone();
    }
    let registration = circular_actors::get(*container.domain().actor_type());
    let [registered] = registration.ports().fixed().outlets() else {
        return derived.clone();
    };
    registered.id().clone()
}

fn boundary_inlet_leaves(
    actors: &BTreeMap<NamedActorId, ActorDecl>,
    containers: &BTreeMap<Name, FoldedScope>,
    inner: &Endpoint,
) -> Vec<BoundaryLeaf> {
    if boundary_inner_side(actors.get(inner.actor()), Side::Inlet) == Side::Inlet {
        return match resolve(containers, inner, Side::Inlet) {
            Resolved::Concrete(endpoints) => endpoints,
            Resolved::Template(_) => Vec::new(),
        };
    }
    vec![BoundaryLeaf::at(inner.clone())]
}

fn boundary_outlet_leaves(
    actors: &BTreeMap<NamedActorId, ActorDecl>,
    containers: &BTreeMap<Name, FoldedScope>,
    inner: &Endpoint,
) -> Vec<BoundaryLeaf> {
    if boundary_inner_side(actors.get(inner.actor()), Side::Outlet) == Side::Inlet {
        return vec![BoundaryLeaf::at(inner.clone())];
    }
    match resolve(containers, inner, Side::Outlet) {
        Resolved::Concrete(endpoints) => endpoints,
        Resolved::Template(_) => Vec::new(),
    }
}

#[cfg(test)]
thread_local! { pub(crate) static FLATTEN_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub fn fold_revision(plan: &AuthoredProjection) -> Result<RunGraph, GraphError> {
    flatten(plan)
}

pub(crate) fn flatten(plan: &AuthoredProjection) -> Result<RunGraph, GraphError> {
    #[cfg(test)]
    FLATTEN_COUNT.with(|count| count.set(count.get() + 1));
    let mut flat = fold_projection::<Result<FoldedScope, GraphError>>(plan, |layer| {
        let mut flat = RunGraph::default();
        let mut containers: BTreeMap<Name, FoldedScope> = BTreeMap::new();

        let declaration = layer.graph().declaration();
        let layer_scope = layer.scope().clone();
        let layer_role = declaration.role();
        let actors = layer.actors();
        let edges = layer.edges();
        for (segment, child) in layer.into_scopes() {
            let child = child?;
            flat.scopes.absorb(&child.flat.scopes);
            containers.insert(segment.name().clone(), child);
        }
        flat.scopes.declare(layer_scope, layer_role);

        for (actor, actor_declaration) in actors {
            let Some(child) = containers.get_mut(actor.name()) else {
                flat.actors.insert(actor.clone(), actor_declaration.clone());
                continue;
            };
            if child.role.is_template() {
                flat.actors.insert(actor.clone(), actor_declaration.clone());
                let inlets: Vec<_> = child
                    .ports
                    .iter()
                    .filter(|((side, _), _)| *side == Side::Inlet)
                    .map(|((_, port), endpoints)| (port.clone(), endpoints.clone()))
                    .collect();
                if let [(derived, inlet)] = inlets.as_slice() {
                    let outer = container_boundary_inlet(actor_declaration, inlets.len(), derived);
                    child.ports.insert((Side::Inlet, outer), inlet.clone());
                }
                let outlets: Vec<_> = child
                    .ports
                    .iter()
                    .filter(|((side, _), _)| *side == Side::Outlet)
                    .map(|((_, port), endpoints)| (port.clone(), endpoints.clone()))
                    .collect();
                if let [(derived, outlet)] = outlets.as_slice() {
                    let outer =
                        container_boundary_outlet(actor_declaration, outlets.len(), derived);
                    if outer != *derived {
                        child.ports.insert((Side::Outlet, outer), outlet.clone());
                    }
                }
                flat.templates.push(TemplateScope {
                    container: actor.clone(),
                    actors: child.flat.actors.clone(),
                    edges: child.flat.edges.clone(),
                });
                flat.templates.extend(child.flat.templates.iter().cloned());
                continue;
            }
            flat.actors.extend(
                child
                    .flat
                    .actors
                    .iter()
                    .map(|inner| (inner.0.clone(), inner.1.clone())),
            );
            flat.edges.extend(child.flat.edges.iter().cloned());
            flat.templates.extend(child.flat.templates.iter().cloned());
            flat.template_bound
                .extend(child.flat.template_bound.iter().cloned());
            flat.inbound.extend(child.flat.inbound.iter().cloned());
            flat.outbound.extend(child.flat.outbound.iter().cloned());
        }

        for edge in edges.values() {
            let id = edge.id();
            let preprocess = CompiledPreprocess::compile(&id, edge.attrs().preprocess())?;
            let from = resolve(&containers, edge.from(), Side::Outlet);
            let to = resolve(&containers, edge.to(), Side::Inlet);
            if actors.get(edge.to().actor()).is_some_and(|declaration| {
                *declaration.domain().actor_type() == circular_plan::ActorType::Replicator
            }) && let Resolved::Concrete(sources) = &from
            {
                for source in sources {
                    let to =
                        Endpoint::new(edge.to().actor().clone(), PortId::try_new("event").unwrap());
                    flat.edges.push(RunEdge {
                        id: realized_edge(&id, &source.endpoint, &to),
                        authored: id.clone(),
                        from: source.endpoint.clone(),
                        to,
                        attrs: edge.attrs(),
                        preprocess: preprocess.clone(),
                        hop: CompiledPreprocess::default(),
                    });
                }
            }
            match (from, to) {
                (Resolved::Concrete(from), Resolved::Concrete(to)) => {
                    for source in &from {
                        for target in &to {
                            flat.edges.push(RunEdge {
                                id: realized_edge(&id, &source.endpoint, &target.endpoint),
                                authored: id.clone(),
                                from: source.endpoint.clone(),
                                to: target.endpoint.clone(),
                                attrs: edge.attrs(),
                                preprocess: preprocess.clone(),
                                hop: target.hop.clone(),
                            });
                        }
                    }
                }
                (Resolved::Concrete(from), Resolved::Template(inner))
                    if !from.is_empty() && !inner.is_empty() =>
                {
                    for outer in from {
                        for target in &inner {
                            flat.inbound.push(BoundaryCrossing {
                                edge: realized_edge(&id, &outer.endpoint, &target.endpoint),
                                authored: id.clone(),
                                container: edge.to().actor().clone(),
                                inner: target.endpoint.clone(),
                                outer: outer.endpoint.clone(),
                                attrs: edge.attrs(),
                                preprocess: preprocess.clone(),
                                hop: target.hop.clone(),
                            });
                        }
                    }
                }
                (Resolved::Template(inner), Resolved::Concrete(to))
                    if !inner.is_empty() && !to.is_empty() =>
                {
                    for source in inner {
                        for outer in &to {
                            flat.outbound.push(BoundaryCrossing {
                                edge: realized_edge(&id, edge.from(), &outer.endpoint),
                                authored: id.clone(),
                                container: edge.from().actor().clone(),
                                inner: source.endpoint.clone(),
                                outer: outer.endpoint.clone(),
                                attrs: edge.attrs(),
                                preprocess: preprocess.clone(),
                                hop: outer.hop.clone(),
                            });
                        }
                    }
                }
                (from, to) => {
                    let (endpoint, side, receiver) = match (from, to) {
                        (Resolved::Template(receiver), _) => (
                            edge.from().clone(),
                            Side::Outlet,
                            receiver.into_iter().next().map(|leaf| leaf.endpoint),
                        ),
                        (_, Resolved::Template(receiver)) => (
                            edge.to().clone(),
                            Side::Inlet,
                            receiver.into_iter().next().map(|leaf| leaf.endpoint),
                        ),
                        _ => unreachable!("concrete leaves were handled above"),
                    };
                    flat.template_bound.push(TemplateBoundEdge {
                        edge: id,
                        endpoint,
                        side,
                        receiver,
                    });
                }
            }
        }

        let mut ports = BTreeMap::new();
        for (outer, inner) in declaration.boundary().inlets() {
            ports.insert(
                (Side::Inlet, outer.clone()),
                boundary_inlet_leaves(actors, &containers, inner),
            );
        }
        for (outer, inner) in declaration.boundary().outlets() {
            ports.insert(
                (Side::Outlet, outer.clone()),
                boundary_outlet_leaves(actors, &containers, inner),
            );
        }

        Ok(FoldedScope {
            role: declaration.role(),
            ports,
            flat,
        })
    })?
    .flat;

    let inbound = std::mem::take(&mut flat.inbound);
    for crossing in inbound {
        match bind_keyed(
            &flat.scopes,
            &crossing,
            flat.actors.get(&crossing.container),
        ) {
            Some(keyed) => flat.keyed.push(keyed),
            None => flat.template_bound.push(TemplateBoundEdge {
                edge: crossing.authored,
                endpoint: Endpoint::new(crossing.container, crossing.inner.port().clone()),
                side: Side::Inlet,
                receiver: Some(crossing.inner),
            }),
        }
    }

    for template in flat
        .templates
        .iter()
        .filter(|template| flat.actors.contains_key(&template.container))
    {
        let container = &template.container;
        let admitted = flat
            .admit_template(container.scope(), container.name())
            .map_err(|_| GraphError::UnknownTemplate(container.clone()))?;
        flat.admitted.insert(container.clone(), admitted);
    }

    let outbound = std::mem::take(&mut flat.outbound);
    for crossing in outbound {
        match flat.admitted.get(&crossing.container) {
            Some(_) => {
                flat.outbound.push(crossing);
            }
            None => flat.template_bound.push(TemplateBoundEdge {
                edge: crossing.authored,
                endpoint: Endpoint::new(crossing.container, crossing.inner.port().clone()),
                side: Side::Outlet,
                receiver: Some(crossing.inner),
            }),
        }
    }

    flat.declared_sources = plan
        .graph()
        .edges()
        .iter()
        .map(|(declared, decl)| (declared.as_edge_id(), decl.from().clone()))
        .collect();
    Ok(flat)
}

fn realized_edge(authored: &EdgeId, from: &Endpoint, to: &Endpoint) -> EdgeId {
    EdgeId::declared(from.clone(), to.clone(), ordinal_of(authored))
}

fn bind_keyed(
    scopes: &circular_plan::ScopeRoleTable,
    crossing: &BoundaryCrossing,
    container: Option<&ActorDecl>,
) -> Option<KeyedCrossing> {
    let admitted = circular_plan::admit_template(
        scopes,
        crossing.container.scope(),
        crossing.container.name(),
    )
    .ok()?;
    let container = container
        .filter(|actor| *actor.domain().actor_type() == circular_plan::ActorType::Replicator)?;
    let folded = crate::activation_config::fold_config(
        circular_plan::ActorType::Replicator,
        container.domain().config(),
    )
    .ok()?;
    let key_at = circular_actors::replicator_actor::ReplicatorRouting::from_value(
        folded.for_type(circular_plan::ActorType::Replicator).ok()?,
    )
    .ok()?
    .at()
    .clone();
    Some(KeyedCrossing {
        edge: crossing.edge.clone(),
        authored: crossing.authored.clone(),
        outer: crossing.outer.clone(),
        inlet: circular_runtime::KeyedInlet::new(admitted, crossing.inner.clone()).ok()?,
        key_at,
        attrs: crossing.attrs.clone(),
        preprocess: crossing.preprocess.clone(),
        hop: crossing.hop.clone(),
    })
}

enum Resolved {
    Concrete(Vec<BoundaryLeaf>),
    Template(Vec<BoundaryLeaf>),
}

fn resolve(containers: &BTreeMap<Name, FoldedScope>, endpoint: &Endpoint, side: Side) -> Resolved {
    let Some(child) = containers.get(endpoint.actor().name()) else {
        return Resolved::Concrete(vec![BoundaryLeaf::at(endpoint.clone())]);
    };
    let inner = child
        .ports
        .get(&(side, endpoint.port().clone()))
        .cloned()
        .unwrap_or_default();
    if child.role.is_template() {
        return Resolved::Template(inner);
    }
    if inner.is_empty() {
        Resolved::Template(Vec::new())
    } else {
        Resolved::Concrete(inner)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct CellGraph {
    pub(crate) actors: BTreeMap<NamedActorId, ActorDecl>,
    pub(crate) edges: Vec<RunEdge>,
}

pub(crate) fn derive_cell(
    template: &TemplateScope,
    admitted: &AdmittedTemplate,
    key: &InstanceKey,
) -> Result<CellGraph, GraphError> {
    let mut actors = BTreeMap::new();
    for (actor, declaration) in &template.actors {
        actors.insert(
            admitted
                .cell_actor(actor, key)
                .map_err(GraphError::CellDerivation)?,
            declaration.clone(),
        );
    }

    let mut edges = Vec::with_capacity(template.edges.len());
    for edge in &template.edges {
        let from = cell_endpoint(admitted, edge.from(), key)?;
        let to = cell_endpoint(admitted, edge.to(), key)?;
        let EdgeId::Declared { ordinal, .. } = edge.id() else {
            continue;
        };
        edges.push(RunEdge {
            id: EdgeId::declared(from.clone(), to.clone(), *ordinal),
            authored: edge.authored.clone(),
            from,
            to,
            attrs: edge.attrs(),
            preprocess: edge.preprocess.clone(),
            hop: edge.hop.clone(),
        });
    }

    Ok(CellGraph { actors, edges })
}

pub(crate) fn cell_endpoint(
    admitted: &AdmittedTemplate,
    endpoint: &Endpoint,
    key: &InstanceKey,
) -> Result<Endpoint, GraphError> {
    let actor = admitted
        .cell_actor(endpoint.actor(), key)
        .map_err(GraphError::CellDerivation)?;
    Ok(Endpoint::new(actor, endpoint.port().clone()))
}

