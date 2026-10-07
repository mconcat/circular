import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { sameValue } from '@circular/protocol';
import { resolveViewConfig } from '@circular/core';

import { healthLabel, inSpace, codeText, lifecycleLabel } from './reasons.mjs';
import { placed, portSlot, portsFoot, footprint } from './layout.mjs';
import { identity, readFirstPage, journalRows, domId } from './query.mjs';
import { readProblems } from './wire-inspector.mjs';
import { readInstances } from './view-instances.mjs';
import { views } from './views.mjs';
import { size, UNMEASURED } from './card-size.mjs';
import { place } from './placement.mjs';
import { distribute, viewRecords } from './arrivals.mjs';
import { recordValue } from './record-text.mjs';
import { sceneFromSnapshot } from './fold.mjs';
import { actorCatalog, authoringActorPorts, actorCreateAdmission } from '@circular/client';
import { readSnapshot, readDaemonHealth } from './session.mjs';
export const key = identity;
const segment = s => s.name ?? `${s.of}[${String(s.key)}]`;
export const scopeLabel = scope => scope.map(segment).join(' / ') || 'Workspace';
export const addressPath = address => '/' + [...address.scope.map(segment), String(address.local)].join('/');
export function accepted(result) {
  if (result.status !== 'accepted') {
    const error = new Error(result.diagnostic?.message ?? result.diagnostics?.[0]?.message ?? result.status);
    error.code = inSpace('Query', result.diagnostic?.code ?? result.diagnostics?.[0]?.code);
    throw error;
  }
  return result.value;
}
export const ORIGIN = Object.freeze({ x: 72, y: 72 });
const NOTHING = Object.freeze({});
function declaredView(view, actorType, typeDefault) {
  const kind = views.viewOf(view?.kind, actorType);
  try {
    return { kind, config: resolveViewConfig(view?.config, typeDefault) };
  } catch (error) {
    if (error?.code !== 'CIRCULAR_VIEW_CONFIG_INVALID') throw error;
    return { kind: kind.code ? kind : { ...kind, code: error.code }, config: {} };
  }
}
export const newCardSize = (type, local, metrics) => size(views.of(views.viewOf(undefined, type).kind), local, metrics);
export const landing = (graph, scope, asked, leaving = new Set()) =>
  place(asked, graph.nodes.filter(n => n.scope === scope && !leaving.has(n.id)).map(n => footprint(n, graph.metrics ?? UNMEASURED)));
export function actorNode({ address, declaration, presentation = {} }, observation = NOTHING, registration, metrics) {
  const p = presentation, view = declaredView(p.view, declaration.actorType, registration?.view_config);
  const { ports, portsReason, ...seen } = observation;
  const title = p.label ?? address.local;
  const box = p.size ? { w: p.size.w, h: p.size.h }
    : size(views.of(view.kind.kind), title, metrics, portsFoot(Math.max(ports?.in_ports?.length ?? 0, ports?.out_ports?.length ?? 0), metrics),
      declaration.config);
  return { id: key(address), address, scope: key(address.scope), title,
    type: declaration.actorType, registration, declaration,
    presentation: p, viewKind: view.kind, viewConfig: view.config,
    ...(p.fixed ? { x: Number(p.fixed.x), y: Number(p.fixed.y) } : {}),
    width: Number(box.w), height: Number(box.h),
    in: (ports?.in_ports ?? []).map((port, n) => [port.id, port.flow, portSlot(n, metrics), port.label]),
    out: (ports?.out_ports ?? []).map((port, n) => [port.id, port.flow, portSlot(n, metrics), port.label]),
    portsAvailable: Boolean(ports),
    portsUnavailableReason: ports ? null : codeText(portsReason ?? registration?.ports_unavailable_reason ?? 'PORTS_UNAVAILABLE'),
    health: null, arrivals: [], ...seen, activity: healthLabel(seen.health?.state) };
}
export const PORT_UNANSWERED = 'PORT_UNANSWERED';
const shapeText = shape => ({
  Any: () => 'any', Base: () => shape.base, Variable: () => shape.name,
  Array: () => `array<${shapeText(shape.item)}>`,
  Object: () => `{${[...shape.fields.map(f => `${f.name}: ${shapeText(f.shape)}`), ...(shape.open ? ['…'] : [])].join(', ')}}`,
})[shape.kind]();
const rateText = rate => rate.kind === 'Period' ? `period ${rate.ticks}` : rate.name;
export const portShape = availability => availability.kind === 'Unavailable' ? availability.reason
  : availability.flow.kind === 'Signal'
    ? `signal<${shapeText(availability.flow.item)}> · ${rateText(availability.flow.rate)}`
    : `stream<${shapeText(availability.flow.item)}>`;
export function unansweredPorts(node, edges, metrics) {
  if (!node.portsAvailable) return { in: [], out: [] };
  const missing = (side, ids) => [...new Set(ids)].filter(id => !node[side].some(p => p[0] === id))
    .map((id, n) => [id, PORT_UNANSWERED, portSlot(node[side].length + n, metrics)]);
  return { in: missing('in', edges.filter(e => e.to === node.id).map(e => e.in)),
    out: missing('out', edges.filter(e => e.from === node.id).map(e => e.out)) };
}
export function annotationNote({ address, declaration, presentation = {} }) {
  return { id: key(address), address, scope: key(address.scope), presentation,
    kind: declaration.kind, references: declaration.refs, body: declaration.body,
    ...(presentation.fixed ? { x: Number(presentation.fixed.x), y: Number(presentation.fixed.y) } : {}),
    width: Number(presentation.size?.w ?? 230), height: Number(presentation.size?.h ?? 120) };
}
export function scopeEntry(address, declaration) {
  return { id: key(address), address, name: scopeLabel(address), ...(declaration ? { declaration } : {}) };
}
function edgeEntry({ address, declaration }) {
  return { id: key(address), address, from: key(declaration.from.actor), out: declaration.from.port,
    to: key(declaration.to.actor), in: declaration.to.port, attributes: declaration.attrs };
}

const joinedActors = new WeakMap(), edgeOf = new WeakMap();
const ofFold = new WeakMap(), placedAt = new WeakMap();
const once = (memo, input, make) => memo.get(input) ?? memo.set(input, make()).get(input);
function foldReading(declared, actors) {
  return once(ofFold, declared, () => {
    const scopes = new Map([...declared.scopes.values()].map(s => [key(s.address), scopeEntry(s.address, s.declaration)]));
    const annotations = [...declared.annotations.values()].map((a, index) => annotationNote({ ...a, index }));
    for (const n of [...actors, ...annotations]) if (!scopes.has(n.scope)) scopes.set(n.scope, scopeEntry(n.address.scope));
    return { scopes: [...scopes.values()], annotations, exportMounts: [...declared.mounts.values()] };
  });
}
const drawnWith = new WeakMap();
function placesOf(declared, actors, edges, drawn, metrics) {
  const held = drawn && drawnWith.get(drawn);
  if (held?.declared === declared) return held.places;
  const shown = new Map((drawn ?? []).filter(n => !n.presentation?.fixed).map(n => [n.id, { x: n.x, y: n.y }]));
  return new Map(placed(actors, edges, ORIGIN, metrics, shown).filter((n, i) => n !== actors[i]).map(n => [n.id, { x: n.x, y: n.y }]));
}
const healthIndex = new WeakMap(), pendingIndex = new WeakMap(), NONE = new Map();
const healthRows = page => !page ? NONE : once(healthIndex, page, () => {
  const rows = new Map();
  for (const row of page.items ?? []) { const id = key(actorIdentityFromValue(row.actor)); if (!rows.has(id)) rows.set(id, row); }
  return rows;
});
const PENDING = ['requested', 'submitting', 'failed'];
const pendingRows = rows => !rows ? NONE : once(pendingIndex, rows, () => {
  const counts = new Map();
  for (const row of rows) if (row.actor && PENDING.includes(row.state)) counts.set(row.actor, (counts.get(row.actor) ?? 0) + 1);
  return counts;
});
export function joined({ drawn: carried, ...graph }) {
  const { declared, observed } = graph, drawn = carried ?? graph.nodes;
  const registration = type => graph.catalog?.find(r => r.actor_type === type);
  const health = healthRows(graph.healthPage), pending = pendingRows(graph.approvals);
  const actors = [];
  for (const entry of declared.actors.values()) {
    if (!entry.declaration) continue;
    const held = joinedActors.get(entry), id = held?.id ?? key(entry.address), dom = held?.dom ?? domId(id);
    const observation = observed.actors.get(id) ?? NOTHING, row = registration(entry.declaration.actorType);
    const read = { health: health.get(id) ?? null, approvalCount: pending.get(dom) ?? 0 };
    if (held && held.read.health !== read.health && sameValue(held.read.health, read.health)) read.health = held.read.health;
    if (held?.observation === observation && held.registration === row && held.metrics === graph.metrics
      && held.read.health === read.health && held.read.approvalCount === read.approvalCount) { actors.push(held.node); continue; }
    const node = actorNode(entry, { ...observation, ...read }, row, graph.metrics);
    joinedActors.set(entry, { id, dom, observation, registration: row, metrics: graph.metrics, read, node });
    actors.push(node);
  }
  if (drawn) {
    const rank = new Map(drawn.map((n, i) => [n.id, i]));
    const at = (node, i) => rank.get(node.id) ?? rank.size + i;
    const order = new Map(actors.map((node, i) => [node, at(node, i)]));
    actors.sort((a, b) => order.get(a) - order.get(b));
  }
  const edges = [...declared.edges.values()].map(entry => once(edgeOf, entry, () => edgeEntry(entry)));
  const fold = foldReading(declared, actors), places = placesOf(declared, actors, edges, drawn, graph.metrics);
  const nodes = actors.map(node => {
    const at = places.get(node.id), held = at && placedAt.get(node);
    if (!at) return node;
    if (held?.x === at.x && held.y === at.y) return held.node;
    const next = { ...node, ...at };
    placedAt.set(node, { ...at, node: next });
    return next;
  });
  drawnWith.set(nodes, { declared, places });
  return { ...graph, nodes, edges, scopes: fold.scopes, annotations: fold.annotations, exportMounts: fold.exportMounts };
}
const same = (held, patch) => Object.keys(patch).every(name => held[name] === patch[name]);
export function observeActors(graph, patchOf, only) {
  let actors;
  for (const node of graph.nodes) {
    if (only && !only.has(node.id)) continue;
    const patch = patchOf(node);
    const held = (actors ?? graph.observed.actors).get(node.id) ?? NOTHING;
    if (!patch || same(held, patch)) continue;
    (actors ??= new Map(graph.observed.actors)).set(node.id, { ...held, ...patch });
  }
  return actors ? joined({ ...graph, observed: { ...graph.observed, actors } }) : graph;
}
const readModel = (declared, fields = {}) =>
  joined({ ...fields, declared, observed: { actors: new Map() } });

export function snapshotToScene(result, portsResult, catalogResult, health, events, metrics) {
  const { anchor, commands, terminal } = accepted(result);
  const catalogObservation = accepted(catalogResult);
  const graph = readModel(sceneFromSnapshot({ anchor, commands }), { anchor, catalog: catalogObservation.items,
    catalogObservation, snapshotPage: { terminal }, journalPage: events, metrics });
  const answered = applyPorts(graph, portsResult);
  return distribute(applyHealth(answered, { page: health ?? null }), events);
}

export const viewContext = graph => ({display:recordValue, graph, pause:runPause(graph.healthPage),
  emitted:n => viewRecords(n, 'emitted'),
  configEntry:n => graph.createInputs?.entries.get(n.type), configCode:graph.createInputs?.diagnostic || undefined,
  mounts:graph.exportMounts, instances:graph.instances, page:graph.journalPage});

export function runPause(page) {
  const word = page?.anchor?.lifecycle;
  return word === 'stopped' ? { code: word, label: lifecycleLabel(word), text: lifecycleLabel(word) } : null;
}

export async function readPorts(session, scope = [], commands = [], catalog = []) {
  let result;
  try { result = await authoringActorPorts(session, scope); }
  catch (error) { result = {status:'rejected', diagnostic:{code:error.code ?? 'PORTS_UNAVAILABLE'}}; }
  const admissions = new Map();
  for (const command of commands.filter(c => c.kind === 'UpsertActor')) {
    const row = catalog.find(r => r.actor_type === command.declaration.actorType);
    if (!row || row.ports_unavailable_reason === null) continue;
    let admission;
    try { admission = await actorCreateAdmission(session, command.declaration.actorType, command.declaration.config, command.actor.value); }
    catch (error) { admission = {status:'rejected', diagnostic:{code:error.code ?? 'READ_UNAVAILABLE', message:error.message}}; }
    admissions.set(key(command.actor.value), admission);
  }
  return { ...result, admissions };
}
export function applyPorts(graph, portsResult, asked) {
  const ports = portsResult.status === 'accepted' ? portsResult.value.items : [];
  const portsReason = inSpace('Query', portsResult.diagnostic?.code ?? portsResult.diagnostics?.[0]?.code);
  const rows = new Map();
  for (const row of ports) { const id = key(actorIdentityFromValue(row.actor[1])); if (!rows.has(id)) rows.set(id, row); }
  return observeActors(graph, node => {
    const held = graph.observed.actors.get(node.id), row = rows.get(node.id);
    const admission = !asked || asked.has(node.id) ? portsResult.admissions?.get(node.id) : held?.admission;
    if (held && sameValue(held.admission, admission) && held.portsReason === portsReason && sameValue(held.ports, row)) return null;
    return { ports: row, portsReason, admission };
  });
}

export async function readHealth(session, lens) {
  try { return { page: await readDaemonHealth(session, lens), diagnostic: null }; }
  catch (error) { return { page: null, diagnostic: error.code ?? 'READ_UNAVAILABLE' }; }
}
export function applyHealth(graph, observation) {
  if (!observation.page && graph.healthPage) return { ...graph, healthDiagnostic: observation.diagnostic };
  const page = observation.page;
  return joined({ ...graph, health: page?.anchor ?? null, healthPage: page, healthDiagnostic: observation.diagnostic });
}

export async function readScene(session, scope = [], catalogObservation,
  readEvents = () => readFirstPage(session, 'actor.events', null, journalRows), anchored = async () => {}, lens, upto, metrics) {
  const snapshot = await readSnapshot(session, scope, 256, upto);
  await anchored(accepted(snapshot).anchor);
  const catalog = catalogObservation === undefined ? await actorCatalog(session)
    : {status:'accepted', value:catalogObservation};
  const ports = await readPorts(session, scope, accepted(snapshot).commands, accepted(catalog).items);
  const health = await readHealth(session, lens);
  const runtime = await readRuntime(session, lens);
  let events, journalDiagnostic;
  try { events = await readEvents(); }
  catch (error) { events = error.page; journalDiagnostic = error.code ?? 'READ_UNAVAILABLE'; }
  return { ...applyHealth(snapshotToScene(snapshot, ports, catalog, null, events, metrics), health), ...runtime, declarationCut: upto,
    ...(journalDiagnostic === undefined ? {} : { journalDiagnostic }) };
}
export const readRuntime = async (session, lens) => ({ problems: await readProblems(session, journalRows, lens),
  instances: await readInstances(session, journalRows, lens) });
