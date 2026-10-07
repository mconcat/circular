import { CircularUInt } from '@circular/protocol';
import { authoringSnapshotArgumentsValue, authoringSnapshotQueryResultFromValue } from '@circular/protocol/authoring-query';
import { reconstructiveCommandFromValue, declarationPayloadValue } from '@circular/protocol/declaration';
import { authoringActorPortsItemFromValue } from '@circular/protocol/actor-query';
import * as adapter from '../renderer/adapter.mjs';
import { UNMEASURED } from '../renderer/card-size.mjs';
export const uint = value => new CircularUInt(BigInt(value));
export const ack = {kind:{verb:'SubscribeAck'},payload:1n};
export const creditAck = {kind:{verb:'SubscribeAck'},payload:[1n,{pending_after:uint(0n)}]};
const address = (local, scope = []) => [3n, { local, scope }];
const root = [[1n, 'desk']];
const absolute = (local, scope = root) => ({ local, scope });
const flags = { bypass: false, mute: false, pause: false };
export const anyStream = [1n, [1n, [1n]]];
export const answered = ports => ports.status !== 'accepted' ? ports
  : { ...ports, value: { ...ports.value, items: ports.value.items.map(authoringActorPortsItemFromValue) } };
export function fixture() {
  const items = [
    { kind: 'UpsertActor', actor: address('a'), declaration: { domain: { actor_type: 'arbitrary_vendor', config: null }, flags } },
    { kind: 'UpsertActor', actor: address('b'), declaration: { domain: { actor_type: 'future_actor', config: {} }, flags } },
    { kind: 'UpsertActor', actor: address('a', [[1n, 'child']]), declaration: { domain: { actor_type: 'future_actor', config: {} }, flags } },
    { kind: 'SetPresentation', owner: { actor: address('a') }, presentation: { collapsed: false, label: 'A <label>', fixed: { x: 100n, y: -2n }, size: { w: 244n, h: 248n }, view: { kind: 'feed', config: null } } },
  ];
  const edge = (from, to, ordinal) => reconstructiveCommandFromValue({
    kind: 'UpsertEdge',
    edge: [3n, [1n, {actor:{scope:[],local:from},port:'event'}, {actor:{scope:[],local:to},port:'input'}, ordinal]],
    declaration: {from:[{scope:[],local:from},'event'], to:[{scope:[],local:to},'input'], ordinal,
      attrs:{delay:{num:0n,den:1n},policy:{delivery:2n},preprocess:[]}},
  });
  const snapshot = { status: 'accepted', value: { anchor: { scope: [{ name: 'desk' }], cursor: 7n, authoringRevision: {kind:'At', revision:new Uint8Array(32).fill(7)}, environment:{declarationSchema:new Uint8Array([1]),specSet:new Uint8Array([3])} },
    commands: [...items.map(reconstructiveCommandFromValue), edge('a', 'b', 0n), edge('b', 'a', 0n), edge('a', 'b', 1n)] } };
  const ports = { status: 'accepted', value: { items: ['a', 'b'].map(local => ({ actor: [1n, absolute(local)],
    in_ports: [{ id: 'input', flow: anyStream, label: null }], out_ports: [{ id: 'event', flow: anyStream, label: null }] })) } };
  const catalog = { status: 'accepted', value: { items: [] } };
  const health = { anchor: { lifecycle: 'running' }, items: [{ actor: absolute('b'), state: 'failed', reason: 'interpreter_fault' }] };
  const events = { items: [{ kind: 'actor_arrival', actor: absolute('a'), port: 'input', body: '<observed>', observed_at_ms: 10n }] };
  return { snapshot, ports, catalog, health, events };
}

export function snapshotAnswer(f, terminal = 2n) {
  const a = f.snapshot.value.anchor;
  return {kind:{verb:'QueryResult'}, payload:[1n, {
    anchor:{scope:a.scope.map(s => [1n,s.name]),cursor:a.cursor,
      authoring_revision:[2n,a.authoringRevision.revision],topology_revision:[2n,a.authoringRevision.revision],
      environment:{declaration_schema:a.environment.declarationSchema,spec_set:a.environment.specSet}},
    items:f.snapshot.value.commands.map(c => declarationPayloadValue(c,{context:'snapshot',includeKind:true})), terminal,
  }]};
}

export const catalogRow = (actor_type, config_schema = 1n, fields = {}) => ({ actor_type, label: actor_type, description: '',
  presentation_role: [1n], source: false, view_config: null, config_schema, creatable: false, template_config: null,
  in_ports: [], out_ports: [], ports_unavailable_reason: null, unavailable_reason: null, ...fields });
export const createInputSlot = fields => ({
  constraint: null, snippet: null, label: null, description: null, group: null, ...fields });
export function healthPage(items = [], anchor = {}) {
  return { anchor: { config_defaults: [], dead_letter: null, journal: { ceiling: null, usage: null }, lifecycle: 'running',
    storage: null, version: uint(1n), wall_clock: null, ...anchor },
  items: items.map(row => ({ detail: null, reason: null, record: uint(1n), since_ms: uint(0n), ...row })), terminal: 2n };
}
export const approvalRow = (item, state = 1n, emitter = [1n, [[1n, 'desk']], 'a']) => ({ item, emitter, target_effect: item,
  state, summary: [2n, 1n], cause: null });
export const approvalPage = (items) => ({ anchor: { producer: 1n, persistence: [3n] }, items, terminal: 2n });
export const deadLetterItem = (body, { producer = body.origin.actor, sequence = 0n, at = 9_000n } = {}) =>
  ({ ...body, dropped: [uint(at), uint(0n), producer, uint(sequence), uint(1n)] });
export const harnessCandidate = (name, found = null) => ({ name, found });
export const harnessCandidatesPage = (declared = []) =>
  ({ anchor: null, items: declared.map(([name, found]) => harnessCandidate(name, found)) });
export const edgeDepthRow = row => row.actor !== undefined
  ? { actor: row.actor, code: uint(31n) }
  : { edge: row.edge, depth: uint(row.depth ?? 0n), queued: uint(row.queued ?? 0n), capacity: row.capacity == null ? null : uint(row.capacity) };
export const edgeDepthsComplete = frames => {
  const anchor = new Uint8Array(8);
  new DataView(anchor.buffer).setBigUint64(0, BigInt(frames));
  return { reason: 9n, code: 0n, anchor };
};

const queryResult = payload => ({kind:{partition:'Query',verb:'QueryResult'}, payload});
function answerOf(value) {
  if (value?.kind?.verb) return value;
  if (value?.status === 'accepted') return queryResult([1n, {anchor:value.value.anchor ?? null, items:value.value.items, terminal:2n}]);
  if (value?.status === 'rejected') {
    const [d] = value.diagnostics ?? [];
    return queryResult([2n, {code:BigInt(d?.code ?? 1n), message:d?.message ?? 'rejected'}]);
  }
  if (value?.rejected) return queryResult([2n, value.rejected]);
  return queryResult([1n, {...(Object.hasOwn(value, 'anchor') ? {} : {anchor:null}), ...value, terminal:2n}]);
}

export function fakeSession({ answers = {}, subscribe, scripts = {}, onSend, onRelease, ...members } = {}) {
  const calls = [], streams = new Map();
  let slots = 0;
  const answer = async (name, request) => {
    let value = answers[name];
    if (typeof value === 'function') value = await value(request);
    if (Array.isArray(value)) value = value.length > 1 ? value.shift() : value[0];
    if (value === undefined) throw new Error(`Missing test query: ${name}`);
    return answerOf(value);
  };
  const session = {
    calls, answers,
    hold(name) {
      const slot = ++slots;
      if (name.startsWith('subscription-')) {
        const target = name.slice('subscription-'.length), script = scripts[target], queue = script ?? [], waiting = [];
        const stream = { open: true, push(envelope) { const resume = waiting.shift(); resume ? resume(envelope) : queue.push(envelope); } };
        streams.set(target, [...(streams.get(target) ?? []), stream]);
        let asked = 0;
        const arrived = entry => !(entry?.kind?.verb === 'SubscribeAck' && asked === 0) && typeof entry?.then !== 'function';
        return { slot,
          async send(partition, verb, value) {
            onSend?.(target, verb, value);
            asked += 1;
            if (script) return;
            const scripted = subscribe?.(target, verb, value);
            if (scripted !== null) stream.push(scripted ?? (verb === 'Credit' ? creditAck : ack));
          },
          next(waitMs) {
            if (queue.length && (waitMs !== 0 || arrived(queue[0]))) {
              const entry = queue.shift();
              if (entry?.kind?.verb === 'SubscribeAck') asked = Math.max(0, asked - 1);
              return Promise.resolve(typeof entry === 'function' ? entry() : entry);
            }
            if (waitMs === 0) return Promise.resolve(null);
            return new Promise(resolve => waiting.push(resolve));
          },
          release() { stream.open = false; onRelease?.(target); } };
      }
      calls.push(['hold', name]);
      let request, released = false;
      return { slot,
        async send(partition, verb, value) { request = value; calls.push(['send', partition, verb, value]); },
        async next() { return answer(name, request); },
        release() { if (released) throw new Error('Double release'); released = true; calls.push(['release']); } };
    },
    async exchange(partition, verb, payload) {
      const stream = session.hold(payload?.name ?? verb);
      try { await stream.send(partition, verb, payload); return await stream.next(); } finally { stream.release(); }
    },
    authoringSnapshot: (scope, limit) => collect(session, scope, limit),
    async declare() { throw Object.assign(new Error('EDIT_UNAVAILABLE'), { code: 'EDIT_UNAVAILABLE' }); },
    push(target, envelope) { (streams.get(target) ?? []).filter(s => s.open).at(-1).push(envelope); },
    subscriptions: target => (streams.get(target) ?? []).filter(s => s.open).length,
    async goodbye() {}, async close() {},
    ...members,
  };
  return session;
}

export function fixtureSDK(f = fixture(), overrides = {}, members = {}) {
  return fakeSession({ ...members, answers: {
    'authoring-snapshot': () => snapshotAnswer(f),
    'actor.catalog': () => f.catalog,
    'authoring.actor-ports': () => f.ports,
    'daemon.health': () => healthPage(f.health.items, f.health.anchor), 'actor.events': () => f.events,
    records: { anchor: ['records'], items: [{ cursor: { anchor: [new Uint8Array(32), [[1n, 'desk']]], domain: 'records', position: new Uint8Array([1, 2]) }, fact: new Uint8Array([9, 8, 7]) }] },
    'runtime.approvals': approvalPage([{ ...approvalRow([1n]), target_effect: [2n] }]),
    'dead.letters': { anchor: null, items: [deadLetterItem({ origin: { actor: { scope: [[1n, 'desk']], local: 'a' }, port: 'event' },
      reason: { code: 'destination_gone', detail: null }, scope: [[1n, 'desk']], subject: { shape: [2n, 'string'], value: 'lost' },
      target: { actor: { scope: [[1n, 'desk']], local: 'a' }, port: 'event' } })] },
    'instance.transitions': { anchor: null, items: [] },
    'actor.create-inputs': () => f.createInputs ?? { anchor: [], items: [] },
    'agent.harnesses': (rows => ({ anchor: rows, items: rows }))([{ name:'local-reader', program:'/fixture/reader', saved:'/fixture/reader' }]),
    'agent.harness-candidates': () => harnessCandidatesPage(f.harnessCandidates), ...overrides,
  } });
}

async function collect(session, scope, limit) {
  const stream = session.hold('authoring-snapshot'), items = [];
  try {
    for (let cursor;;) {
      await stream.send('Query', 'Query', {name:'authoring-snapshot', args:authoringSnapshotArgumentsValue(scope),
        page:{limit:BigInt(limit), ...(cursor === undefined ? {} : {cursor})}});
      const result = authoringSnapshotQueryResultFromValue((await stream.next()).payload);
      if (result.status !== 'accepted') return result;
      items.push(...result.value.items);
      if (result.value.terminal === 'Complete') return {status:'accepted', value:{anchor:result.value.anchor, commands:items}};
      if (result.value.terminal === 'Diagnostic') return {status:'partial', anchor:result.value.anchor, items, diagnostic:result.value.diagnostic};
      cursor = result.value.next;
    }
  } finally { stream.release(); }
}

export async function productMount(window, session, { app = {}, product = {} } = {}) {
  window.circularConnection ??= async () => ({ connection: 'connected' });
  await adapter.initialize({ connect: async () => session });
  window.StudyApp = { state: { scope: 'root' }, archive: {},
    graph: () => window.STUDY.root, liveScopes: () => Object.fromEntries(Object.entries(window.STUDY).filter(([, v]) => v.nodes)),
    timeMachine: { refresh() {} }, renderGraph() {}, renderActors() { return []; }, renderJournal() {}, toast() {}, ...app };
  window.Product = { refreshEditTools() {}, ...product };
  const mounted = await adapter.mount({ metrics: UNMEASURED });
  return { mounted, observed: await adapter.observation };
}
