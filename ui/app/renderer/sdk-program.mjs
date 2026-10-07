import { sceneCommands } from './fold.mjs';
import { inSpace } from './reasons.mjs';

export function projectSDKProgram(graph, ports) {
  const projection = { pending: true, diagnostics: [], program: null };
  projection.ready = (async () => {
    const { generateProgram, generatorEnvironment } = await import('@circular/generator');
    const answered = ports && await ports;
    const admissions = new Map();
    for (const node of graph.nodes) {
      const admission = answered ? answered.admissions.get(node.id) : graph.observed.actors.get(node.id)?.admission;
      if (admission?.status === 'accepted') admissions.set([...node.address.scope.map(s => s.name), node.address.local].join('/'), admission.value.items[0]);
      else if (admission) return {
        diagnostics: (admission.diagnostics ?? [admission.diagnostic]).map(d => ({ ...d, reason: inSpace('Query', d.code) })) };
    }
    const result = generateProgram(sceneCommands(graph.declared), {
      ...generatorEnvironment, specSet: graph.anchor.environment.specSet, catalog: graph.catalog, admissions,
    });
    if (result.status !== 'complete') return { diagnostics: result.diagnostics.map(d => ({ ...d, reason: d.message })) };
    const { program, statements } = result.value;
    return { statements, program: { entry: program.entry,
      modules: new Map([...program.modules].map(([name, bytes]) => [name, new TextDecoder().decode(bytes)])) } };
  })().catch(error => {
    console.error('SDK program unavailable', error);
    return { diagnostics: [{ code: error.code ?? 'SDK_PROGRAM_UNAVAILABLE' }] };
  }).then(result => {
    Object.assign(projection, result, { pending: false });
    if (projection.diagnostics.length) console.error('SDK program unavailable', projection.diagnostics);
  });
  return projection;
}

export function sdkProgram(graph, actor) {
  const { pending, diagnostics, program, statements } = graph.sdkProgram;
  if (pending || diagnostics.length) return { pending, spans: [], text: '', diagnostics };
  const key = [...actor.scope.map(s => s.name), actor.local].join('/');
  const lines = new Map([...program.modules].map(([module, text]) => [module, text.split('\n')]));
  const spans = statements.filter(statement => statement.owner.actor === key || statement.refs.includes(key))
    .map(({ module, line, owner }) => ({ module, line, owner: Object.values(owner)[0], own: owner.actor === key,
      code: lines.get(module)[line - 1] }));
  const text = spans.filter(span => span.own).map(span => `${span.code}\n`).join('');
  return { spans, text, diagnostics: [] };
}
