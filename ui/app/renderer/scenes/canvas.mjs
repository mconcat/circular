export default { name: 'canvas', label: 'Canvas', async read(_session, graph) { return { graph }; },
  project({ graph }) { return { kind: 'canvas', graph }; } };
