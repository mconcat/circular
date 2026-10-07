export function configuration(graph, selected) {
  const node = graph.nodes.find(node => node.id === selected);
  return node ? { node, actor: node.address, title: node.title, config: node.declaration.config,
    flags: node.declaration.flags, inlets: node.in, outlets: node.out, portsAvailable: node.portsAvailable } : null;
}
export default { name:'configure', label:'Configure',
  select(surface, selected) { return {...surface, configuration:configuration(surface.graph, selected)}; },
  async read(_session, graph) { return {graph}; },
  project({graph}, selected) { return {kind:'configure',graph,configuration:configuration(graph,selected)}; } };
