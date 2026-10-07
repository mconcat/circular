import { initialize, mount, observation } from '../renderer/adapter.mjs';
import { UNMEASURED } from '../renderer/card-size.mjs';
import { fixture, catalogRow } from './fixtures.mjs';
export { catalogRow };
import { editPeer, createInputsAnswer, anyShapes } from './edit-peer.mjs';
import { viewerMaps } from './viewer-maps.mjs';

export const declaredSchema = [2n, 'registry ConfigSchema is an incomplete frame'];

export function withConfig(config, slots) {
  const f = fixture();
  f.snapshot.value.commands = f.snapshot.value.commands.map((row, i) => i === 1
    ? { ...row, declaration: { ...row.declaration, config } } : row);
  f.catalog.value.items = [catalogRow('future_actor', declaredSchema), catalogRow('arbitrary_vendor', 1n)];
  f.createInputs = createInputsAnswer({ future_actor: slots ?? anyShapes(config), arbitrary_vendor: null });
  return f;
}
export const form = values => ({ elements: { namedItem: name => name in values ? { value: values[name] } : null } });

export async function product(fixtureValue, reject, run, extras = {}) {
  const peer = await editPeer({ fixtureValue, reject });
  const previous = { window: globalThis.window, location: globalThis.location, document: globalThis.document };
  try {
    const calls = [], session = { ...peer.session, declare: command => { calls.push('declare'); return peer.session.declare(command); } };
    globalThis.window = { circularConnection: async () => ({ connection: 'connected' }) };
    globalThis.location = { search: '' };
    globalThis.document = { querySelector: () => ({}), querySelectorAll: () => [], getElementById: () => null };
    await initialize({ connect: async () => session });
    const { drafts, rawDrafts, submissions } = viewerMaps();
    let historical = false;
    window.StudyApp = { state: { scope: 'root', selected: null }, archive: {}, historical: () => historical,
      liveScopes: () => Object.fromEntries(Object.entries(window.STUDY).filter(([, v]) => v.nodes)),
      graph: () => window.STUDY.root, renderGraph() {}, renderInspector() {}, renderActors() { return []; }, renderJournal() {}, timeMachine: { refresh() {} },
      toast() {}, changedKeys: n => { const d = drafts.get(n.id); return d ? Object.keys(d).filter(k => JSON.stringify(d[k]) !== JSON.stringify(n.config[k])) : []; } };
    window.Product = { refreshEditTools() {}, ...extras };
    await mount({ metrics: UNMEASURED });
    if (await observation !== true) throw new Error('fixture observation failed');
    const node = local => window.STUDY.root.nodes.find(n => n.title === local || n.id === local);
    await run({ peer, node, calls, drafts, rawDrafts, submissions, past: value => { historical = value; } });
    for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve));
  } finally { await peer.session.close(); Object.assign(globalThis, previous); }
}
