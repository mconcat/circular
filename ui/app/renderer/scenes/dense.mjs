export default { name: 'dense', label: 'Dense', async read(_session, graph) { return { graph }; },
  project({ graph }) { return { kind: 'dense', graph }; } };
