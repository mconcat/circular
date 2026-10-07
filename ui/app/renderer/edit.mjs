import { DEFAULT_EDGE_ATTRS, edgeKeyFromDeclaration, declarationAddressFromValue } from '@circular/protocol/declaration';
import { deriveBoundaryPortId } from '@circular/protocol';
import { compactTemplateCommands, replicatorInlet, replicatorPolicy } from '@circular/generator/internal';
import { key, newCardSize, landing } from './scene.mjs';
import { footprint } from './layout.mjs';
import { UNMEASURED } from './card-size.mjs';
import { LENS } from './verbs.mjs';
import { retireNote } from './annotations.mjs';
import { actorCreateAdmission, setAgentHarness } from '@circular/client';
export const address = value => ({ arm: 'absolute', value });
export const configure = (node, config) => ({ kind: 'UpsertActor', actor: address(node.address),
  declaration: { ...node.declaration, config } });
export const setFlags = (node, flags) => ({ kind: 'SetFlags', actor: address(node.address), flags });
export const present = (node, changes, owner = { actor: address(node.address) }) => ({ kind: 'SetPresentation', owner,
  presentation: { collapsed: false, ...node.presentation, ...changes } });
export const move = (node, x, y, owner) => present(node, { fixed: { x: BigInt(Math.round(x)), y: BigInt(Math.round(y)) } }, owner);
const same = (a, b) => a !== undefined && b !== undefined && BigInt(a.x) === b.x && BigInt(a.y) === b.y;
export const organize = (nodes, positions) => nodes.flatMap(node => {
  const at = positions.get(node.id), command = move(node, at.x, at.y);
  return same(node.presentation?.fixed, command.presentation.fixed) ? [] : [command];
});
export const dragMove = (node, x, y, owner) =>
  Math.round(x) === Math.round(node.x) && Math.round(y) === Math.round(node.y) ? [] : [move(node, x, y, owner)];
export const dragResize = (node, width, height, owner) =>
  Math.round(width) === Math.round(node.width) && Math.round(height) === Math.round(node.height) ? []
    : [present(node, { size: { w: BigInt(Math.round(width)), h: BigInt(Math.round(height)) } }, owner)];
export const moveToScope = (nodes, target) => ({ kind: 'MoveToScope', actors: nodes.map(n => address(n.address)), target: address(target) });
export const disconnect = edge => ({ kind: 'RetireEdge', edge: address(edge.address) });
const isChildScope = (node, scope) => scope.length === node.address.scope.length + 1
  && key(scope.slice(0, -1)) === key(node.address.scope)
  && (scope.at(-1).name ?? scope.at(-1).of) === node.address.local;
const under = (scope, prefix) => scope.length >= prefix.length && key(scope.slice(0, prefix.length)) === key(prefix);
export function removeActors(graph, nodes) {
  const retired = new Set(), held = [];
  const scopes = graph.scopes.filter(s => s.declaration).map(s => s.address);
  const known = new Map([...scopes, ...graph.nodes.flatMap(n => n.address.scope.map((_, i) => n.address.scope.slice(0, i + 1)))]
    .map(s => [key(s), s]));
  const visitScope = scope => {
    for (const child of graph.nodes.filter(n => key(n.address.scope) === key(scope))) visitActor(child);
    for (const inner of known.values()) if (inner.length === scope.length + 1 && under(inner, scope)
      && !graph.nodes.some(n => isChildScope(n, inner))) visitScope(inner);
    for (const note of (graph.annotations ?? []).filter(n => key(n.address.scope) === key(scope))) held.push(retireNote(note));
    for (const mount of (graph.exportMounts ?? []).filter(m => key(m.address.scope) === key(scope)))
      held.push({ kind: 'RetireExportMount', mount: address(mount.address) });
    if (scopes.some(s => key(s) === key(scope))) held.push({ kind: 'RetireScope', scope: address(scope) });
  };
  const visitActor = node => {
    if (retired.has(node.id)) return;
    retired.add(node.id);
    for (const scope of known.values()) if (isChildScope(node, scope)) visitScope(scope);
    held.push({ kind: 'RetireActor', actor: address(node.address) });
  };
  for (const node of nodes) visitActor(node);
  const taken = new Set(held.filter(c => c.kind === 'RetireExportMount').map(c => key(c.mount.value)));
  const mounts = (graph.exportMounts ?? []).filter(m => !taken.has(key(m.address))
    && Object.values(m.declaration.roles ?? {}).some(binding => binding && retired.has(key(binding.actor))))
    .map(m => ({ kind: 'RetireExportMount', mount: address(m.address) }));
  return [...graph.edges.filter(edge => retired.has(edge.from) || retired.has(edge.to)).map(disconnect), ...mounts, ...held];
}
export function connect(graph, from, out, to, inlet, preprocess = []) {
  const used = new Set(graph.edges.filter(edge => edge.from === from.id && edge.out === out && edge.to === to.id && edge.in === inlet)
    .map(edge => Number(edge.address.ordinal)));
  let ordinal = 0; while (used.has(ordinal)) ordinal++;
  const declaration = { from: { actor: from.address, port: out }, to: { actor: to.address, port: inlet }, ordinal,
    attrs: { ...DEFAULT_EDGE_ATTRS, preprocess } };
  return { kind: 'UpsertEdge', edge: address(edgeKeyFromDeclaration(declaration)), declaration };
}
export const mountRequest = (actor, port) => ({ kind: 'UpsertExportMount', mount: address({ scope: actor.scope, local: actor.local }),
  declaration: { roles: { request: { actor, port } } } });
export async function bindHarness(session, name, program) {
  try { return await setAgentHarness(session, { name, program }); }
  catch (error) {
    if (error instanceof TypeError) throw Object.assign(new Error('RESULT_UNEXPECTED'), { code: 'RESULT_UNEXPECTED' });
    throw error;
  }
}
const standing = (graph, actor) => graph.nodes.some(node => key(node.address) === key(actor));
export function freshLocal(graph, scope, base) {
  let local = base;
  for (let n = 2; standing(graph, { scope, local }); n++) local = `${base}${n}`;
  return local;
}
export async function createActor(session, graph, scope, local, registration, config, flags) {
  const actor = address({ scope, local });
  if (standing(graph, actor.value)) throw Object.assign(new Error('ACTOR_EXISTS'), { code: 'ACTOR_EXISTS' });
  const result = await actorCreateAdmission(session, registration.actor_type, config, actor.value);
  if (result.status !== 'accepted') return result;
  const row = result.value.items[0];
  if (!row?.authored_actor) throw Object.assign(new Error('ADMISSION_UNAVAILABLE'), { code: 'ADMISSION_UNAVAILABLE' });
  return { status: 'accepted', value: { kind: 'UpsertActor', actor: declarationAddressFromValue([1n, row.authored_actor], 'actor', 'mutation'),
    declaration: { actorType: row.actor_type, config: row.config, flags } }, ports: { in: row.in_ports, out: row.out_ports } };
}
const refused = (code, message) => Object.assign(new Error(message ?? code), { code });
const unflagged = () => ({ bypass: false, mute: false, pause: false });
function newContainer(graph, nodes, parent, local, type) {
  const actor = { scope: parent, local };
  if (standing(graph, actor)) throw refused('ACTOR_EXISTS');
  const size = newCardSize(type, local, graph.metrics);
  const [at] = landing(graph, key(parent), [{ x: Math.min(...nodes.map(n => n.x)), y: Math.min(...nodes.map(n => n.y)), w: size.w, h: size.h }],
    new Set(nodes.map(n => n.id)));
  return { actor, place: move({ address: actor }, at.x, at.y) };
}
export function foldIntoNewScope(graph, nodes, parent, local, registration) {
  const { actor, place } = newContainer(graph, nodes, parent, local, registration.actor_type);
  const target = [...parent, { name: local }];
  return [
    { kind: 'UpsertActor', actor: address(actor),
      declaration: { actorType: registration.actor_type, config: registration.template_config, flags: unflagged() } },
    { kind: 'UpsertScope', scope: address(target), declaration: { role: 'Concrete', boundary: { inlets: [], outlets: [] } } },
    place,
    moveToScope(nodes, target),
  ];
}
export function foldIntoNewReplicator(graph, nodes, parent, local, registration, policy) {
  const { actor, place } = newContainer(graph, nodes, parent, local, registration.actor_type);
  if (!replicatorPolicy(policy)) throw refused('authoring.prepass.replicator-policy');
  const members = new Set(nodes.map(n => n.id));
  const inCell = a => ({ ...a, scope: a.scope.slice(parent.length) });
  const relative = value => ({ arm: 'relative', value });
  const endpoint = (end, at = inCell) => ({ actor: at(end.actor), port: end.port });
  const wire = (from, to, ordinal, attrs, spell = relative) => {
    const declaration = { from, to, ordinal, attrs };
    return { kind: 'UpsertEdge', edge: spell(edgeKeyFromDeclaration(declaration)), declaration };
  };
  const value = [];
  for (const n of nodes) {
    value.push({ kind: 'UpsertActor', actor: relative(inCell(n.address)), declaration: n.declaration });
    const presentation = LENS.presentation.view(graph.declared, { actor: n.address });
    if (presentation) value.push({ kind: 'SetPresentation', owner: { actor: relative(inCell(n.address)) },
      presentation: presentation.anchor?.target ? { ...presentation, anchor: { ...presentation.anchor, target: inCell(presentation.anchor.target) } } : presentation });
    for (const scope of graph.scopes.filter(s => s.declaration && isChildScope(n, s.address)))
      value.push({ kind: 'UpsertScope', scope: relative(scope.address.slice(parent.length)), declaration: scope.declaration });
  }
  for (const mount of (graph.exportMounts ?? []).filter(m => Object.values(m.declaration.roles ?? {}).some(b => b && members.has(key(b.actor)))))
    value.push({ kind: 'UpsertExportMount', mount: relative(inCell(mount.address)), declaration: { ...mount.declaration,
      roles: Object.fromEntries(Object.entries(mount.declaration.roles).filter(([, b]) => b != null).map(([role, b]) => [role, endpoint(b)])) } });
  const boundaries = { in: new Map(), out: new Map() }, outer = [];
  const boundary = (side, inner) => {
    const at = key([inner.actor, inner.port]);
    if (boundaries[side].has(at)) return boundaries[side].get(at);
    const local = `b${new TextEncoder().encode(inner.actor.local).length}_${inner.actor.local}_${inner.port}_${side}`;
    const made = { label: `${inner.actor.local}.${inner.port}`, at: { scope: [], local },
      port: deriveBoundaryPortId(side === 'in' ? 'inlet' : 'outlet', { scope: [], local }, 0n) };
    value.push({ kind: 'UpsertActor', actor: relative(made.at), declaration: { actorType: side === 'in' ? 'input' : 'output',
      config: { label: made.label }, flags: unflagged() } },
    side === 'in' ? wire({ actor: made.at, port: made.port }, endpoint(inner), 0, DEFAULT_EDGE_ATTRS)
      : wire(endpoint(inner), { actor: made.at, port: made.port }, 0, DEFAULT_EDGE_ATTRS));
    boundaries[side].set(at, made);
    return made;
  };
  const inlet = registration.in_ports?.find(port => port.primary)?.id ?? registration.in_ports?.[0]?.id;
  for (const edge of graph.edges) {
    const from = members.has(edge.from), to = members.has(edge.to), { ordinal } = edge.address;
    if (from && to) value.push(wire(endpoint(edge.address.from), endpoint(edge.address.to), ordinal, edge.attributes));
    else if (to) {
      boundary('in', edge.address.to);
      if (inlet === undefined) throw refused('CONNECT_PORT_REQUIRED');
      outer.push(wire(edge.address.from, { actor, port: inlet }, ordinal, edge.attributes, address));
    } else if (from) outer.push(wire({ actor, port: boundary('out', edge.address.from).port }, edge.address.to, ordinal, edge.attributes, address));
  }
  const topics = type => value.filter(c => c.kind === 'UpsertActor' && c.declaration.actorType === type)
    .map(c => c.declaration.config?.label).sort();
  let name = 'cell';
  for (let n = 2; graph.declared.templates.has(key(name)); n++) name = `cell${n}`;
  const config = { template: name, in: topics('input'), out: topics('output'), ...policy };
  if (!replicatorInlet(config)) throw refused('authoring.prepass.template-inlet-count');
  let commands;
  try { commands = compactTemplateCommands(value); }
  catch (error) { if (error instanceof TypeError) throw refused(error.message.split(':')[0], error.message); throw error; }
  return [
    { kind: 'UpsertTemplate', name, commands },
    ...removeActors(graph, nodes),
    { kind: 'UpsertActor', actor: address(actor), declaration: { actorType: registration.actor_type, config, flags: unflagged() } },
    place,
    ...outer,
  ];
}
export function moveIntoScope(graph, nodes, target) {
  const placed = nodes.filter(n => n.presentation?.fixed);
  const places = landing(graph, key(target), placed.map(n => footprint(n, graph.metrics ?? UNMEASURED)));
  return [...placed.flatMap((n, i) => dragMove(n, places[i].x, places[i].y)), moveToScope(nodes, target)];
}

export function uuid() {
  const b = crypto.getRandomValues(new Uint8Array(16));
  b[6] = (b[6] & 0x0f) | 0x40; b[8] = (b[8] & 0x3f) | 0x80;
  const h = [...b].map(v => v.toString(16).padStart(2, '0')).join('');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
export function editor(session, issueCommitId = () => new TextEncoder().encode(uuid()), { onSettled } = {}) {
  let epoch, pending = false, busy = false, uncertain = false, prepared;
  const issued = new Set();
  let opening;
  const spell = id => Array.from(id, b => b.toString(16).padStart(2, '0')).join('');
  const ended = () => { issued.delete(opening); opening = undefined; };
  let lost, settling;
  let cursor, continuous = false, generation = 0;
  const waits = new Set();
  const report = outcome => { onSettled?.(outcome); return outcome; };
  const unread = () => { for (const wait of [...waits]) { waits.delete(wait); wait.resolve(false); } };
  const readThrough = (until, since) => !continuous || generation !== since ? Promise.resolve(false)
    : cursor >= until ? Promise.resolve(true) : new Promise(resolve => waits.add({ until, resolve }));
  const outcome = async (id, since) => {
    let now;
    if (issued.has(id)) try { now = await session.authoringSnapshot?.([], 256); } catch { now = undefined; }
    const until = now?.status === 'accepted' ? now.value.anchor.cursor : undefined;
    const read = typeof until === 'bigint' && await readThrough(until, since);
    if (!issued.has(id)) return report({ code: 'EDIT_LOST_COMMITTED' });
    if (!read) return report({ code: 'EDIT_LOST_UNKNOWN' });
    issued.delete(id);
    return report({ code: 'EDIT_LOST_NOT_COMMITTED' });
  };
  const settle = async () => {
    if (!lost || lost.epoch === undefined) return;
    let answer;
    try { answer = await session.declare({ kind: 'AbortEpoch', epoch: lost.epoch }); }
    catch { return; }
    const id = spell(lost.commitId), { since } = lost;
    lost = undefined; uncertain = false; epoch = undefined; pending = false;
    if (answer.status === 'accepted') { ended(); report({ code: 'EDIT_LOST_NOT_COMMITTED' }); return; }
    opening = undefined;
    void outcome(id, since);
  };
  const settleOnce = () => settling ??= settle().finally(() => { settling = undefined; });
  const serial = async action => {
    if (uncertain && !busy) await settleOnce();
    if (busy || uncertain) throw Object.assign(new Error('EDIT_BUSY'), { code: 'EDIT_BUSY' });
    busy = true;
    try { return await action(); } finally { busy = false; if (uncertain) settleOnce(); }
  };
  const send = async command => {
    const since = generation;
    try { return await session.declare(command); }
    catch (error) {
      uncertain = true;
      lost = { epoch: command.kind === 'BeginEpoch' ? undefined : epoch, commitId: prepared?.commitId, since };
      throw error;
    }
  };
  const abort = async () => {
    const result = await send({ kind: 'AbortEpoch', epoch });
    if (result.status === 'accepted') { epoch = undefined; pending = false; ended(); }
    return result;
  };
  return {
    get pending() { return pending; }, get busy() { return busy; }, get uncertain() { return uncertain; },
    settle: () => uncertain && !busy ? settleOnce() : Promise.resolve(),
    authored: commitId => issued.delete(spell(commitId)),
    following: after => { unread(); cursor = after; continuous = true; generation += 1; },
    observed: at => { cursor = at; for (const wait of [...waits]) if (wait.until <= at) { waits.delete(wait); wait.resolve(true); } },
    release: () => { continuous = false; unread(); },
    prepare: (anchor, commands) => serial(async () => {
      if (epoch !== undefined) throw Object.assign(new Error('EDIT_PENDING'), { code: 'EDIT_PENDING' });
      if (!anchor?.authoringRevision || !anchor?.environment) throw Object.assign(new Error('EDIT_BASELINE_UNAVAILABLE'), { code: 'EDIT_BASELINE_UNAVAILABLE' });
      const commitId = issueCommitId();
      opening = spell(commitId); issued.add(opening);
      prepared = { commitId };
      const begin = { kind: 'BeginEpoch', scope: address(anchor.scope), commitId,
        expectedRevision: anchor.authoringRevision, expectedEnvironment: anchor.environment };
      const opened = await send(begin);
      if (opened.status !== 'accepted') { ended(); return { ...opened, command: begin }; }
      epoch = opened.value.epoch;
      for (const command of commands) {
        const result = await send(command);
        if (result.status !== 'accepted') return { ...result, abort: await abort(), command };
      }
      const validate = { kind: 'ValidateEpoch', epoch };
      const result = await send(validate);
      if (result.status !== 'accepted') return { ...result, abort: await abort(), command: validate };
      pending = true;
      return result;
    }),
    commit: () => serial(async () => {
      if (!pending) throw Object.assign(new Error('EDIT_NOT_VALIDATED'), { code: 'EDIT_NOT_VALIDATED' });
      const command = { kind: 'CommitEpoch', epoch }, since = generation;
      const result = await send(command);
      epoch = undefined; pending = false;
      if (result.status === 'accepted') opening = undefined;
      else ended();
      return result.status === 'accepted' ? { ...result, folded: readThrough(result.value.metadata.cursor, since) }
        : { ...result, command };
    }),
    abort: () => serial(async () => {
      if (epoch === undefined) throw Object.assign(new Error('EDIT_NOT_OPEN'), { code: 'EDIT_NOT_OPEN' });
      return abort();
    }),
  };
}
