use super::*;
use circular_plan::Endpoint;

type Position = (NamedActorId, Side, PortId);
#[derive(Default)]
struct ShapeGraph {
    flows: BTreeMap<Position, Flow>,
    incoming: BTreeMap<Position, Vec<(Position, DeclaredEdgeId, EdgePreprocessTypes)>>,
    aliases: BTreeMap<Position, Position>,
}
struct Layer {
    graph: ShapeGraph,
    inlets: Vec<(PortId, Position)>,
    outlets: Vec<(PortId, Position)>,
}
fn position(endpoint: &Endpoint, side: Side) -> Position {
    (endpoint.actor().clone(), side, endpoint.port().clone())
}

pub(super) fn derive(
    plan: &AuthoredProjection,
) -> Result<BTreeMap<Position, Result<GroundShape, String>>, PlanRegistryError> {
    let layer = fold_projection(plan, |layer| -> Result<Layer, PlanRegistryError> {
        let mut graph = ShapeGraph::default();
        for (actor, declaration) in layer.actors() {
            let Ok(ports) = resolve_ports(
                Some(actor),
                *declaration.domain().actor_type(),
                declaration.domain().config(),
                BoundaryActorGeneration::new(declaration.authored_generation()),
            ) else {
                continue;
            };
            for port in ports.inlets() {
                graph.flows.insert(
                    (actor.clone(), Side::Inlet, port.id().clone()),
                    port.ty().clone(),
                );
            }
            for port in ports.outlets() {
                graph.flows.insert(
                    (actor.clone(), Side::Outlet, port.id().clone()),
                    port.ty().clone(),
                );
            }
            if get(*declaration.domain().actor_type())
                .boundary()
                .is_some_and(|rule| rule.side() == Side::Inlet)
            {
                for port in ports.inlets() {
                    graph.aliases.insert(
                        (actor.clone(), Side::Outlet, port.id().clone()),
                        (actor.clone(), Side::Inlet, port.id().clone()),
                    );
                }
            }
        }
        for (id, edge) in layer.edges() {
            graph
                .incoming
                .entry(position(edge.to(), Side::Inlet))
                .or_default()
                .push((
                    position(edge.from(), Side::Outlet),
                    id.clone(),
                    EdgePreprocessTypes::resolve(edge)?,
                ));
        }
        let boundary = layer.graph().declaration().boundary();
        let bound = |bindings: &BTreeMap<PortId, Endpoint>, side| {
            bindings
                .iter()
                .map(|(port, inner)| {
                    let inner_side = crate::run_graph::boundary_inner_side(
                        layer.actors().get(inner.actor()),
                        side,
                    );
                    (port.clone(), position(inner, inner_side))
                })
                .collect::<Vec<_>>()
        };
        let inlets = bound(boundary.inlets(), Side::Inlet);
        let outlets = bound(boundary.outlets(), Side::Outlet);
        let actors = layer.actors();
        for (segment, child) in layer.into_scopes() {
            let child = child?;
            let Some((container, declaration)) = actors
                .iter()
                .find(|(actor, _)| actor.name() == segment.name())
            else {
                continue;
            };
            let declared_inlets = child.inlets.len();
            for (port, inner) in child.inlets {
                let outer =
                    crate::run_graph::container_boundary_inlet(declaration, declared_inlets, &port);
                if outer != port
                    && let Some(edges) =
                        graph
                            .incoming
                            .remove(&(container.clone(), Side::Inlet, port.clone()))
                {
                    graph
                        .incoming
                        .entry((container.clone(), Side::Inlet, outer.clone()))
                        .or_default()
                        .extend(edges);
                }
                graph
                    .aliases
                    .insert(inner, (container.clone(), Side::Inlet, outer));
            }
            let declared_outlets = child.outlets.len();
            for (port, inner) in child.outlets {
                let outer = crate::run_graph::container_boundary_outlet(
                    declaration,
                    declared_outlets,
                    &port,
                );
                graph
                    .aliases
                    .insert((container.clone(), Side::Outlet, outer), inner);
            }
            graph.flows.extend(child.graph.flows);
            graph.incoming.extend(child.graph.incoming);
            graph.aliases.extend(child.graph.aliases);
        }
        Ok(Layer {
            graph,
            inlets,
            outlets,
        })
    })?;
    let graph = layer.graph;
    let positions: BTreeSet<_> = graph
        .flows
        .keys()
        .chain(graph.aliases.keys())
        .chain(graph.incoming.keys())
        .cloned()
        .collect();
    let mut results = BTreeMap::new();
    for key in positions {
        let result = graph.resolve(&key, &mut BTreeSet::new(), &mut results);
        results.insert(key, result);
    }
    Ok(results)
}
impl ShapeGraph {
    fn resolve(
        &self,
        key: &Position,
        visiting: &mut BTreeSet<Position>,
        results: &mut BTreeMap<Position, Result<GroundShape, String>>,
    ) -> Result<GroundShape, String> {
        if let Some(result) = results.get(key) {
            return result.clone();
        }
        if !visiting.insert(key.clone()) {
            return Err(format!(
                "runtime shape needs a causal input in a cycle at {key:?}"
            ));
        }
        let result = self.resolve_inner(key, visiting, results);
        visiting.remove(key);
        if result.is_ok() {
            results.insert(key.clone(), result.clone());
        }
        result
    }
    fn resolve_inner(
        &self,
        key: &Position,
        visiting: &mut BTreeSet<Position>,
        results: &mut BTreeMap<Position, Result<GroundShape, String>>,
    ) -> Result<GroundShape, String> {
        if let Some(source) = self.aliases.get(key) {
            return self.resolve(source, visiting, results);
        }
        if key.1 == Side::Inlet
            && let Some(edges) = self.incoming.get(key)
        {
            let mut shape = None;
            for (from, id, program) in edges {
                let source = self.resolve(from, visiting, results)?;
                let output = program
                    .output(
                        id,
                        &GroundFlow::try_new(Flow::Stream(source.as_shape().clone())).unwrap(),
                        true,
                    )
                    .map_err(|error| format!("runtime shape program: {error:?}"))?;
                let next = GroundShape::try_new(output.as_flow().item().clone()).unwrap();
                if shape.as_ref().is_some_and(|old| old != &next) {
                    return Err(format!(
                        "runtime shape needs the selected incoming edge at {key:?}"
                    ));
                }
                shape = Some(next);
            }
            if let Some(shape) = shape {
                return Ok(shape);
            }
        }
        let flow = self
            .flows
            .get(key)
            .ok_or_else(|| format!("runtime shape has no declared port {key:?}"))?;
        let mut substitution = Substitution::new();
        for variable in flow.variables() {
            let mut shape = None;
            for (inlet, input) in &self.flows {
                if inlet.0 != key.0
                    || inlet.1 != Side::Inlet
                    || inlet == key
                    || !input.variables().any(|v| v == variable)
                {
                    continue;
                }
                let next = self.resolve(inlet, visiting, results)?;
                if shape.as_ref().is_some_and(|old| old != &next) {
                    return Err(format!(
                        "runtime shape variable {variable:?} needs its originating inlet at {key:?}"
                    ));
                }
                shape = Some(next);
            }
            let shape = shape.ok_or_else(|| {
                format!("runtime shape has no ground source for {key:?}/{variable:?}")
            })?;
            substitution
                .insert(variable.clone(), shape)
                .map_err(|e| format!("runtime shape substitution: {e:?}"))?;
        }
        substitution
            .ground(flow)
            .map(|flow| GroundShape::try_new(flow.as_flow().item().clone()).unwrap())
            .ok_or_else(|| format!("runtime shape is unresolved at {key:?}"))
    }
}
