
use super::Address;
use crate::inlet_preprocess::CompiledPreprocess;
use circular_core::RevisionEpochId;
use circular_plan::{ActorDecl, EdgeId, Endpoint, NamedActorId, PortId, WirePolicy};
use circular_protocol::declaration_payload::DeclaredDelay;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Semaphore;

#[derive(Clone, PartialEq)]
pub(crate) struct InletShape {
    pub(crate) port: PortId,
    pub(crate) program: CompiledPreprocess,
    pub(crate) delay: DeclaredDelay,
    pub(crate) policy: WirePolicy,
}

#[derive(Clone, PartialEq)]
pub(crate) struct OutletShape {
    pub(crate) edge: EdgeId,
    pub(crate) to: Endpoint,
    pub(crate) wire: InletShape,
}

#[derive(Clone)]
pub(crate) struct ShareShape {
    pub(crate) revision: RevisionEpochId,
    pub(crate) declaration: ActorDecl,
    pub(crate) inlets: BTreeMap<EdgeId, InletShape>,
    pub(crate) outlets: BTreeMap<PortId, Vec<OutletShape>>,
    pub(crate) prototype: Option<Arc<CellPrototype>>,
    pub(crate) cell: bool,
    pub(crate) refused: bool,
}

impl ShareShape {
    pub(crate) fn is_entry(&self) -> bool {
        circular_actors::is_entry_actor(*self.declaration.domain().actor_type())
    }

    pub(crate) fn same_as(&self, other: &Self) -> bool {
        self.declaration == other.declaration
            && self.refused == other.refused
            && self.inlets == other.inlets
            && self.outlets == other.outlets
            && match (&self.prototype, &other.prototype) {
                (Some(before), Some(after)) => before.same_as(after),
                (None, None) => true,
                _ => false,
            }
    }
}

pub(crate) fn shapes(standing: &super::system::Standing) -> BTreeMap<NamedActorId, ShareShape> {
    let (graph, table, _) = standing.declarations.parts();
    let revision = standing.revision;
    let mut shapes: BTreeMap<NamedActorId, ShareShape> = graph
        .actors()
        .iter()
        .map(|(actor, declaration)| {
            (
                actor.clone(),
                ShareShape {
                    revision,
                    declaration: declaration.clone(),
                    inlets: BTreeMap::new(),
                    outlets: BTreeMap::new(),
                    prototype: cell_prototype(standing, actor).map(Arc::new),
                    cell: false,
                    refused: table.refused(actor).is_some(),
                },
            )
        })
        .collect();
    for edge in graph.edges() {
        let attrs = edge.attrs();
        let wire = InletShape {
            port: edge.to().port().clone(),
            program: edge.preprocess().for_edge(edge.id()).then(edge.hop()),
            delay: attrs.delay,
            policy: attrs.policy,
        };
        if let Some(from) = shapes.get_mut(edge.from().actor()) {
            from.outlets
                .entry(edge.from().port().clone())
                .or_default()
                .push(OutletShape {
                    edge: edge.id().clone(),
                    to: edge.to().clone(),
                    wire: wire.clone(),
                });
        }
        if let Some(to) = shapes.get_mut(edge.to().actor()) {
            to.inlets.insert(edge.id().clone(), wire);
        }
    }
    for crossing in graph.outbound() {
        if let Some(to) = shapes.get_mut(crossing.outer.actor()) {
            to.inlets
                .insert(crossing.edge.clone(), outbound_wire(crossing));
        }
    }
    shapes
}

#[derive(Clone)]
pub(crate) struct CellPrototype {
    pub(crate) revision: RevisionEpochId,
    pub(crate) authored_generation: u64,
    template: crate::run_graph::TemplateScope,
    admitted: circular_plan::AdmittedTemplate,
    inbound: Vec<crate::run_graph::KeyedCrossing>,
    outbound: Vec<crate::run_graph::BoundaryCrossing>,
    pub(crate) declarations: crate::tap_pilot::StandingDeclarations,
    pub(crate) registry: crate::actor_registry::RegistryProfile,
    pub(crate) grants: crate::tap_pilot::ProductGrantCatalog,
}

impl CellPrototype {
    fn same_as(&self, other: &Self) -> bool {
        self.template == other.template
            && self.admitted == other.admitted
            && self.inbound == other.inbound
            && self.outbound == other.outbound
            && self.declarations == other.declarations
            && self.registry == other.registry
            && self.grants == other.grants
    }
}

pub(crate) fn cell_prototype(
    standing: &super::system::Standing,
    container: &NamedActorId,
) -> Option<CellPrototype> {
    let (graph, declarations, registry) = standing.declarations.parts();
    let template = graph.template(container)?.clone();
    Some(CellPrototype {
        revision: standing.revision,
        authored_generation: graph.actors().get(container)?.authored_generation(),
        admitted: graph.admitted_template(container)?.clone(),
        inbound: graph.keyed_into(container).cloned().collect(),
        outbound: graph
            .outbound()
            .iter()
            .filter(|crossing| &crossing.container == container)
            .cloned()
            .collect(),
        declarations: declarations.for_actors(template.actors.keys()),
        registry,
        grants: standing.grants.for_actors(template.actors.keys()),
        template,
    })
}

fn outbound_wire(crossing: &crate::run_graph::BoundaryCrossing) -> InletShape {
    InletShape {
        port: crossing.outer.port().clone(),
        program: crossing
            .preprocess
            .for_edge(&crossing.edge)
            .then(&crossing.hop),
        delay: crossing.attrs.delay,
        policy: crossing.attrs.policy,
    }
}

pub(crate) struct CellShapes {
    pub(crate) actors: BTreeMap<NamedActorId, ShareShape>,
    pub(crate) forwards: BTreeMap<EdgeId, Vec<OutletShape>>,
}

pub(crate) fn cell_shapes(
    prototype: &CellPrototype,
    key: &circular_runtime::InstanceKey,
) -> Result<CellShapes, crate::run_graph::GraphError> {
    let revision = prototype.revision;
    let container = &prototype.template.container;
    let cell = crate::run_graph::derive_cell(&prototype.template, &prototype.admitted, key)?;
    let mut actors: BTreeMap<NamedActorId, ShareShape> = cell
        .actors
        .iter()
        .filter(|(_, declaration)| !declaration.domain().actor_type().is_container())
        .map(|(actor, declaration)| {
            (
                actor.clone(),
                ShareShape {
                    revision,
                    declaration: declaration.clone(),
                    inlets: BTreeMap::new(),
                    outlets: BTreeMap::new(),
                    prototype: None,
                    cell: true,
                    refused: prototype.declarations.refused(actor).is_some(),
                },
            )
        })
        .collect();
    for edge in &cell.edges {
        let attrs = edge.attrs();
        let wire = InletShape {
            port: edge.to().port().clone(),
            program: edge.preprocess().for_edge(edge.id()).then(edge.hop()),
            delay: attrs.delay,
            policy: attrs.policy,
        };
        if let Some(from) = actors.get_mut(edge.from().actor()) {
            from.outlets
                .entry(edge.from().port().clone())
                .or_default()
                .push(OutletShape {
                    edge: edge.id().clone(),
                    to: edge.to().clone(),
                    wire: wire.clone(),
                });
        }
        if let Some(to) = actors.get_mut(edge.to().actor()) {
            to.inlets.insert(edge.id().clone(), wire);
        }
    }
    let mut forwards: BTreeMap<EdgeId, Vec<OutletShape>> = BTreeMap::new();
    for crossing in &prototype.inbound {
        let receiver = crate::run_graph::keyed_target(&crossing.inlet, key);
        let wire = InletShape {
            port: receiver.port().clone(),
            program: crossing
                .preprocess
                .for_edge(&crossing.edge)
                .then(&crossing.hop),
            delay: crossing.attrs.delay,
            policy: crossing.attrs.policy,
        };
        if let Some(to) = actors.get_mut(receiver.actor()) {
            to.inlets.insert(crossing.edge.clone(), wire.clone());
        }
        forwards
            .entry(crate::run_graph::container_inlet_edge(crossing, container))
            .or_default()
            .push(OutletShape {
                edge: crossing.edge.clone(),
                to: receiver,
                wire,
            });
    }
    for crossing in &prototype.outbound {
        let source = crate::run_graph::cell_endpoint(&prototype.admitted, &crossing.inner, key)?;
        if let Some(from) = actors.get_mut(source.actor()) {
            from.outlets
                .entry(source.port().clone())
                .or_default()
                .push(OutletShape {
                    edge: crossing.edge.clone(),
                    to: crossing.outer.clone(),
                    wire: outbound_wire(crossing),
                });
        }
    }
    Ok(CellShapes { actors, forwards })
}

pub(crate) type Credit = Arc<Semaphore>;

#[derive(Clone)]
pub(crate) struct InletWire {
    pub(crate) shape: InletShape,
}

#[derive(Clone, Debug)]
pub(crate) struct OutWire {
    pub(crate) edge: EdgeId,
    pub(crate) to: Endpoint,
    pub(crate) address: Address,
    pub(crate) credit: Option<Credit>,
}

#[derive(Clone)]
pub(crate) struct Share {
    pub(crate) revision: RevisionEpochId,
    pub(crate) declaration: ActorDecl,
    pub(crate) inlets: BTreeMap<EdgeId, InletWire>,
    pub(crate) outlets: BTreeMap<PortId, Vec<OutWire>>,
    pub(crate) prototype: Option<Arc<CellPrototype>>,
    pub(crate) cell: bool,
}

pub(crate) fn holds_back(policy: &WirePolicy) -> bool {
    !matches!(policy.delivery, circular_plan::Delivery::BestEffort { .. })
}

pub(crate) fn capacity_of(policy: &WirePolicy, mailbox: usize) -> usize {
    policy.capacity.map_or(mailbox, |capacity| capacity.get())
}
