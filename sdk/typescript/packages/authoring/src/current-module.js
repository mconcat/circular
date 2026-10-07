import { portValueType, inputPortHints } from './portflow-types.js';
import { isDeepStrictEqual } from 'node:util';
import { COMBINATOR_NAMES, metadataOf, createCurrentActorHandle, createCurrentEdgeHandle, emitDeclaration, endpointOf } from '@circular/core/internal';
import { authoringActorPorts, actorCatalog } from '@circular/client';
import { declarationAddressFromValue } from '@circular/protocol/declaration';
import { authoringError } from './prepass.js';
import { actorSpelling, bindingIdentifier } from '@circular/generator/internal';
const span = { source: 'circular:current', startLine: 1, startColumn: 1, endLine: 1, endColumn: 1 };
const fail = name => { throw authoringError(`authoring.current.${name}`, span); };
const key = value => JSON.stringify(value, (_name, item) => {
  if (item && typeof item === 'object' && !Array.isArray(item)) return Object.fromEntries(Object.keys(item).sort().map(name => [name, item[name]]));
  return item;
});

const actorKeyOf = actor => actor && typeof actor === 'object' && 'arm' in actor && 'value' in actor ? actor.value : actor;
const refuse = (name, detail) => {
  const error = authoringError(`authoring.current.${name}`, span);
  error.circularDiagnostics = [Object.freeze({ ...error.circularDiagnostics[0], args: Object.freeze([detail]) })];
  throw error;
};
/**
 * `current.edge(source, target, { ordinal })` finds the edge by
 * its endpoints, so an input actor's outlet or a container boundary port — derived ids (`_bi1_…`) the
 * author never wrote — needs no spelled key. The endpoints resolve through the handles' own port tables
 * and the result is the same key the snapshot index already holds; an omitted ordinal is 0, as on
 * declaration. The one-argument key form is kept.
 */
function edgeLookup(edges, lookup) {
  return (first, target, options) => {
    if (target === undefined && options === undefined) return lookup(edges, String(first));
    if (options !== undefined && (options === null || typeof options !== 'object' || Array.isArray(options)
      || Object.keys(options).some(name => name !== 'ordinal'))) {
      refuse('edge-options-invalid', 'current.edge(source, target, options) takes only { ordinal }');
    }
    const ordinal = options?.ordinal ?? 0;
    if (!Number.isSafeInteger(ordinal) || ordinal < 0) refuse('edge-options-invalid', 'edge ordinal must be a non-negative integer');
    const from = endpointOf(first, 'source'), to = endpointOf(target, 'target');
    const searched = key({ from: { actor: actorKeyOf(from.actor), port: from.port }, ordinal,
      to: { actor: actorKeyOf(to.actor), port: to.port } });
    if (!edges.has(searched)) refuse('lookup-missing', searched);
    return edges.get(searched);
  };
}

/** Pure generation from a complete immutable index. No RPC during lookup or property access. */
export function generateCurrentModule(index) {
  index = { ...index, ...Object.fromEntries(['actors', 'edges', 'scopes', 'exports', 'annotations'].map(name => [name, new Map(index[name])])) };
  const lookup = (map, name) => { if (!map.has(name)) fail('lookup-missing'); return map.get(name); };
  const current = Object.freeze(Object.fromEntries(['actor', 'edge', 'scope', 'export', 'annotation'].map(
    (name, i) => [name, name === 'edge' ? edgeLookup(index.edges, lookup)
      : id => lookup(index[['actors', 'edges', 'scopes', 'exports', 'annotations'][i]], String(id))])));
  const exports = Object.create(null);
  exports.current = current; exports.default = current;
  const lines = ['// Generated from the authoring snapshot of the daemon; do not edit.', 'import { current } from "@circular/authoring/current";', 'export { current };', 'export default current;'];
  const declarations = ['// Generated from the authoring snapshot of the daemon: port types of what stands now; do not edit.', 'import type { CircularValue } from "@circular/protocol";', 'import type { CurrentHandleMode, SourceEndpoint, TargetEndpoint } from "@circular/core";', 'import type { CurrentLookupNamespace } from "@circular/authoring";', 'export declare const current: CurrentLookupNamespace;', 'export default current;'];
  const actorType = prefix => {
    const children = [...index.actors.keys()].filter(name => name.startsWith(prefix + '/') && !name.slice(prefix.length + 1).includes('/'));
    const handle = index.actors.get(prefix);
    const metadata = metadataOf(handle);
    const ports = side => metadata?.ports[side === 'in' ? 'inputs' : 'outputs'] ?? [];
    const properties = (side, endpoint) => Object.keys(handle[side] ?? {}).filter(name => side !== 'in' || inputPortHints(metadata?.spelling, [{ id: name }]).length).map(name => `readonly ${JSON.stringify(name)}: ${endpoint}<${portValueType(ports(side).find(p => p.id === name), metadata?.spelling, side === 'in' ? 'input' : 'output')}, CurrentHandleMode>;`).join(' ');
    let base = 'Omit<ReturnType<CurrentLookupNamespace["actor"]>, "in" | "out" | "mount"> & { readonly in: { ' + properties('in', 'TargetEndpoint') + ' }; readonly out: { ' + properties('out', 'SourceEndpoint') + ' }; }';
    if (metadata?.ports.defaultOutput != null) base += metadata.writable
      ? ` & import("@circular/core").WritableBoundaryEndpoint<${portValueType(ports('out').find(p => p.id === metadata.ports.defaultOutput))}, CurrentHandleMode>`
      : ` & SourceEndpoint<${portValueType(ports('out').find(p => p.id === metadata.ports.defaultOutput))}, CurrentHandleMode>`;
    return !handle.actors ? base : base + ' & { readonly actors: { ' + children.map(name => `readonly ${JSON.stringify(name.slice(prefix.length + 1))}: ${actorType(name)};`).join(' ') + ' } }';
  };
  let ordinal = 0;
  for (const [name, handle] of index.actors) {
    if (name.includes('/')) continue;
    if (Object.hasOwn(exports, name)) fail('export-collision');
    exports[name] = handle;
    const identifier = bindingIdentifier(name) ? name : `__actor${ordinal++}`;
    lines.push(`const ${identifier} = current.actor(${JSON.stringify(name)});`);
    lines.push(`export { ${identifier}${identifier === name ? '' : ` as ${JSON.stringify(name)}`} };`);
    declarations.push(`declare const ${identifier}: ${actorType(name)};`);
    declarations.push(`export { ${identifier}${identifier === name ? '' : ` as ${JSON.stringify(name)}`} };`);
  }
  return Object.freeze({ anchor: index.anchor, current, exports: Object.freeze(exports),
    text: lines.join('\n') + '\n', declarations: declarations.join('\n') + '\n' });
}

/** Snapshot then port facts on the same session, fenced by the exact authoring revision. */
export async function hydrateCurrentProject(session, scope, pageLimit, suppliedSnapshot = null) {
  const acquired = suppliedSnapshot ? { status: 'accepted', value: suppliedSnapshot } : await session.authoringSnapshot(scope, pageLimit);
  if (acquired.status !== 'accepted') fail('snapshot-incomplete');
  const snapshot = acquired.value;
  const root = scope.length === 0 ? acquired : await session.authoringSnapshot([], pageLimit);
  if (root.status !== 'accepted' || root.value.anchor.scope.length !== 0) fail('snapshot-incomplete');
  const rootSnapshot = root.value;
  const queried = await authoringActorPorts(session, []);
  if (queried.status !== 'accepted') fail('ports-rejected');
  const revision = rootSnapshot.anchor.authoringRevision;
  const wireRevision = revision.kind === 'Absent' ? 1n : [2n, revision.revision];
  if (!isDeepStrictEqual(queried.value.anchor, wireRevision)) fail('anchor-mismatch');
  const catalog = await actorCatalog(session);
  if (catalog.status !== 'accepted') fail('catalog-rejected');
  const portIndex = new Map(queried.value.items.map(row => [key(declarationAddressFromValue(row.actor, 'actor', 'acceptedHistory').value), row]));
  const index = { anchor: rootSnapshot.anchor, actors: new Map(), edges: new Map(), scopes: new Map(), exports: new Map(), annotations: new Map() };
  const absolute = address => {
    if (address.arm !== 'relative') fail('snapshot-address');
    const value = address.value;
    return { arm: 'absolute', value: Array.isArray(value) ? [...value]
      : { ...value, scope: [...value.scope] } };
  };
  const binding = value => [...value.scope.map(s => {
    if (s.name === undefined) fail('keyed-namespace-not-supported');
    return s.name;
  }), value.local].join('/');
  const mountsOf = new Map();
  for (const command of rootSnapshot.commands.filter(c => c.kind === 'UpsertExportMount')) {
    for (const binding of Object.values(command.declaration.roles ?? {})) {
      if (!binding?.actor) continue;
      const named = key(binding.actor);
      if (!mountsOf.has(named)) mountsOf.set(named, []);
      if (!mountsOf.get(named).some(m => key(m.value) === key(command.mount.value))) mountsOf.get(named).push(absolute(command.mount));
    }
  }
  for (const command of rootSnapshot.commands) {
    if (command.kind === 'UpsertActor') {
      if (COMBINATOR_NAMES.includes(command.declaration.actorType)) fail('combinator-actor-not-supported');
      const actor = absolute(command.actor), fact = portIndex.get(key(actor.value));
      if (!fact) fail('ports-missing');
      const row = catalog.value.items.find(r => r.actor_type === command.declaration.actorType);
      let topics;
      if (command.declaration.actorType === 'pipeline_actor') {
        topics = { in_ports: new Map(), out_ports: new Map() };
        for (const side of ['in_ports', 'out_ports']) for (const { id, label } of fact[side]) {
          if (label === null || topics[side].has(label)) fail('boundary-mismatch');
          topics[side].set(label, id);
        }
      }
      const port = (side, name) => {
        const id = topics ? topics[side].get(name) : name;
        if (!fact[side].some(p => p.id === id)) fail('port-missing'); return id;
      };
      const primary = side => {
        if (command.declaration.actorType === 'input' && side === 'out_ports' && fact[side].length === 1) return fact[side][0].id;
        const candidates = (row?.[side] ?? []).filter(p => p.primary && fact[side].some(f => f.id === p.id));
        return candidates.length === 1 ? candidates[0].id : null;
      };
      const handle = createCurrentActorHandle({ actor, declaration: command.declaration, revision: revision.revision,
        spelling: actorSpelling(command.declaration.actorType, command.actor.value.scope), mounts: mountsOf.get(key(actor.value)) ?? [], ports: {
          inputs: topics ? [...topics.in_ports.keys()].map(id => ({ id, primary: false, flow: fact.in_ports.find(p => p.id === topics.in_ports.get(id)).flow })) : fact.in_ports.map(p => ({ id: p.id, primary: p.id === primary('in_ports'), flow: p.flow })),
          outputs: topics ? [...topics.out_ports.keys()].map(id => ({ id, primary: false, flow: fact.out_ports.find(p => p.id === topics.out_ports.get(id)).flow })) : fact.out_ports.map(p => ({ id: p.id, primary: p.id === primary('out_ports'), flow: p.flow })), defaultInput: primary('in_ports'), defaultOutput: primary('out_ports'),
          input: id => port('in_ports', id), output: id => port('out_ports', id) } });
      if (['pipeline_actor', 'replicator'].includes(command.declaration.actorType)) {
        Object.defineProperty(handle, 'actors', { enumerable: true, value: Object.create(null) });
      }
      index.actors.set(binding(command.actor.value), handle);
    } else if (command.kind === 'UpsertEdge') {
      const endpoint = p => ({ ...p, actor: { arm: 'absolute', value: { ...p.actor, scope: [...p.actor.scope] } } });
      const declaration = { from: endpoint(command.declaration.from), to: endpoint(command.declaration.to),
        ordinal: command.declaration.ordinal, attrs: command.declaration.attrs };
      const edge = { arm: 'absolute', value: { from: { ...command.declaration.from, actor: declaration.from.actor.value },
        to: { ...command.declaration.to, actor: declaration.to.actor.value }, ordinal: declaration.ordinal } };
      index.edges.set(key(command.edge.value), createCurrentEdgeHandle({ edge, declaration }));
    } else if (command.kind === 'UpsertScope') {
      const address = absolute(command.scope);
      index.scopes.set(key(command.scope.value), Object.freeze({ id: address, remove: () => emitDeclaration({ kind: 'RetireScope', scope: address }) }));
    } else if (command.kind === 'UpsertExportMount' || command.kind === 'UpsertAnnotation') {
      const mount = command.kind === 'UpsertExportMount', field = mount ? 'mount' : 'annotation';
      const address = absolute(command[field]);
      index[mount ? 'exports' : 'annotations'].set(binding(command[field].value), Object.freeze({
        remove: () => emitDeclaration({ kind: mount ? 'RetireExportMount' : 'RetireAnnotation', [field]: address }) }));
    }
  }
  for (const [name, handle] of index.actors) {
    const at = name.lastIndexOf('/');
    if (at < 0) continue;
    const parent = index.actors.get(name.slice(0, at));
    const child = name.slice(at + 1);
    if (!parent?.actors || Object.hasOwn(parent.actors, child)) fail('namespace-collision');
    Object.defineProperty(parent.actors, child, { enumerable: true, value: handle });
  }
  for (const handle of index.actors.values()) {
    if (handle.actors) Object.freeze(handle.actors);
    Object.freeze(handle);
  }
  return { snapshot, index, module: generateCurrentModule(index) };
}
