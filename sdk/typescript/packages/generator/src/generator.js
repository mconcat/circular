import { compareSourceNames } from './source-order.js';
import { joinConfigIssue, assembleConfigIssue } from "@circular/core/internal";
import { compactTemplateCommands } from './template-commands.js';
/** Product reconstruction of flat programs, including Concrete and Template module bundles. */
import { printExportSurface } from './export-surface-printer.js';
import { noteFromCommand } from './notes.js';
import * as core from '@circular/core';
import { decodePortFlow, deriveBoundaryPortId } from '@circular/protocol';
import { constructorSpelling, COMBINATOR_NAMES, publicName } from '@circular/core/internal';
import { DEFAULT_EDGE_ATTRS, declarationPayloadValue, declarationCommandFromValue } from '@circular/protocol/declaration';
import { actorSpelling, boundarySpelling, bindingIdentifier, replicatorPolicy, propertyKey, identifierName } from './generator-syntax.js';

function refuse(message, commandIndex = null, kind = null, detail = null) {
  return Object.freeze({ status: "rejected", diagnostics: Object.freeze([Object.freeze({
    phase: "Lowering", class: "Rejection", code: 0,
    primary: commandIndex === null
      ? { kind: "Bundle", module: "main.ts", specifier: null }
      : { kind: "Command", commandIndex, path: null },
    related: [], message, args: [kind, detail],
  })]) });
}

function sameBytes(left, right) {
  return left instanceof Uint8Array && right instanceof Uint8Array
    && left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function literal(value, seen = new Set()) {
  if (value === null) return "null";
  if (typeof value === "string" || typeof value === "boolean") return JSON.stringify(value).replace(/\u2028/g, "\\u2028").replace(/\u2029/g, "\\u2029");
  if (typeof value === "bigint") return `${value}n`;
  if (typeof value === "number") {
    if (Object.is(value, -0)) return "-0";
    if (Number.isNaN(value)) return "Number.NaN";
    if (value === Infinity) return "Number.POSITIVE_INFINITY";
    if (value === -Infinity) return "Number.NEGATIVE_INFINITY";
    return String(value);
  }
  if (value instanceof Uint8Array) return `new Uint8Array([${[...value].join(", ")}])`;
  if (typeof value !== "object" || seen.has(value)) throw new TypeError("unsupported or cyclic source value");
  seen.add(value);
  try {
    if (Array.isArray(value)) {
      if (Object.keys(value).length !== value.length) throw new TypeError("sparse or extended array");
      return `[${value.map((item) => literal(item, seen)).join(", ")}]`;
    }
    if (![Object.prototype, null].includes(Object.getPrototypeOf(value))) throw new TypeError("non-value object");
    if (Reflect.ownKeys(value).some((key) => typeof key !== "string")) throw new TypeError("symbol field");
    return `{ ${Object.keys(value).sort().map((key) => {
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      if (!descriptor.enumerable || !("value" in descriptor)) throw new TypeError("non-data field");
      return `${propertyKey(key)}: ${literal(descriptor.value, seen)}`;
    }).join(", ")} }`;
  } finally { seen.delete(value); }
}

const CONTENT = new Set(['UpsertActor', 'UpsertEdge', 'UpsertScope', 'MoveToScope', 'SetFlags', 'SetPresentation', 'UpsertExportMount', 'UpsertAnnotation']);
const ENVELOPE = new Set(['BeginEpoch', 'ValidateEpoch', 'CommitEpoch']);
const DEFAULT_FLAGS = { bypass: false, mute: false, pause: false };
const same = (a, b) => literal(a) === literal(b);
const commandSame = (a, b) => a.kind === 'SetPresentation'
  ? same({ ...a, presentation: b.presentation }, b) && same(declarationPayloadValue(a, { context: (a.owner.actor ?? a.owner.annotation).arm === 'relative' ? 'snapshot' : 'mutation' }),
    declarationPayloadValue(b, { context: (b.owner.actor ?? b.owner.annotation).arm === 'relative' ? 'snapshot' : 'mutation' }))
  : a.kind === 'UpsertEdge'
  ? same({ ...a, declaration: { ...a.declaration, attrs: { ...a.declaration.attrs, preprocess: a.declaration.attrs.preprocess ?? [] } } }, b) : same(a, b);
const property = name => identifierName(name) ? `.${name}` : `[${literal(name)}]`;

function pinnedCommand(command) {
  if (command.kind !== 'UpsertExportMount' || !Object.hasOwn(command.declaration ?? {}, 'surface')) return command;
  const { surface, ...declaration } = command.declaration;
  if (!same(Object.keys(declaration.roles).sort(), Object.keys(surface.roles).sort())
    || !same(declaration.operations ?? null, surface.operations)) throw new TypeError('surface differs from export roles or operations');
  return command;
}

function inputSource(config) {
  if (config === null || typeof config !== 'object' || !Object.hasOwn(config, 'shape')) return config;
  const flow = decodePortFlow(config.shape);
  if (flow.kind !== 'Stream' || flow.item.kind !== 'Base') throw new TypeError('input shape is not a stream of one base type');
  return { ...config, shape: flow.item.base };
}
function boundarySource(type, config) {
  const keys = type === 'input' ? ['label', 'shape'] : ['label'];
  if (typeof config?.label !== 'string' || !config.label || Object.keys(config).some(key => !keys.includes(key))) return null;
  const { label: topic, ...declared } = type === 'input' ? inputSource(config) : config;
  return { topic, ...declared };
}

function configArguments(spelling, config) {
  if (['bang', 'tap', 'counter', 'match'].includes(spelling)) {
    if (spelling === 'bang' ? !config || Object.keys(config).length !== 0 : config !== null) throw new TypeError('invalid no-config value');
    return [];
  }
  const key = { map: 'transform', filter: 'predicate', alert: 'predicate' }[spelling];
  if (key) {
    if (!config || typeof config[key] !== 'string') throw new TypeError('stored CEL must be a string');
    if (spelling === 'alert') {
      const { predicate, ...rest } = config;
      return [literal(predicate), literal(rest)];
    }
    if (Object.keys(config).length !== 1) throw new TypeError('constructor cannot preserve extra config fields');
    return [literal(config[key])];
  }
  if (spelling === 'input') return [literal(inputSource(config))];
  const canonical = literal(config);
  if (spelling === 'form') {
    let flow;
    try { flow = decodePortFlow(config?.fields); }
    catch { return [canonical]; }
    if (flow.kind === 'Stream' && flow.item.kind === 'Object' && !flow.item.open
      && flow.item.fields.every(field => field.shape.kind === 'Base')) {
      const fields = flow.item.fields;
      const names = Object.keys(Object.fromEntries(fields.map(field => [field.name, field.shape.base])));
      if (names.every((name, index) => name === fields[index].name)) {
        const map = `{ ${fields.map(field => `${propertyKey(field.name)}: ${literal(field.shape.base)}`).join(', ')} }`;
        return [`{ ${Object.keys(config).sort().map(key => `${propertyKey(key)}: ${key === 'fields' ? map : literal(config[key])}`).join(', ')} }`];
      }
    }
  }
  return [canonical];
}

const reconstructionPort = (id, primary) => Object.freeze({ id, primary });
export const FIXED_SOURCE_RECONSTRUCTION_PORTS = Object.freeze({
  join: Object.freeze({
    in_ports: Object.freeze(['event', 'state', 'remove'].map(id => reconstructionPort(id, id === 'event'))),
    out_ports: Object.freeze([reconstructionPort('event', true)]),
  }),
  assemble: Object.freeze({
    in_ports: Object.freeze([reconstructionPort('event', true)]),
    out_ports: Object.freeze([reconstructionPort('event', true), reconstructionPort('_error', false)]),
  }),
  match: Object.freeze({
    in_ports: Object.freeze([reconstructionPort('event', true)]),
    out_ports: Object.freeze([reconstructionPort('ok', true), reconstructionPort('err', false)]),
  }),
});

/**
 * The order a generated program declares its actors in, read off the fold's own wires.
 * The program reads in the pipeline's flow: the operational chain first, the side taps after it.
 * - Back-edges come from one depth-first search, from the actors with no upstream and then from
 *   what is left (a component that is only a cycle): a wire into an actor still on that search's
 *   stack. A back-edge is an ordinary wire; it blocks nothing and prints as `.into` once both of
 *   its ends are declared. A wire back into its own actor is no upstream.
 * - An actor is declared after its upstreams over the other wires (so a join follows all of its
 *   inputs). A walk that reaches an actor while one of those upstreams is still undeclared (on the
 *   walk's stack, pulling its own upstreams) leaves it waiting: that upstream reaches it again, as
 *   its downstream, once it is declared. A waiting actor is not pulled as an upstream; it still
 *   misses one of its own.
 * - Once an actor is declared, its downstreams that go on (have a downstream of their own) are
 *   walked at once. Its downstreams that end there (taps, views, sinks) wait in the walk's queue,
 *   which is drained after the walk from each root, so side taps follow the chain they hang off,
 *   in chain order. An actor whose only downstream ends there walks it at once: that downstream
 *   ends the chain.
 * - Every actor is declared exactly once. Every tie is broken by the actor name's bytes, so the order
 *   does not depend on the order `items` arrive in. `wires` are [from, to] names; the adjacency is
 *   built once.
 */
function flowOrder(items, nameOf, wires) {
  const sorted = [...items].sort((a, b) => compareSourceNames(nameOf(a), nameOf(b)));
  const byName = new Map(sorted.map(item => [nameOf(item), item]));
  const downOf = new Map(sorted.map(item => [item, new Set()])), upOf = new Map(sorted.map(item => [item, new Set()]));
  for (const [from, to] of wires) {
    const source = byName.get(from), target = byName.get(to);
    if (!source || !target || source === target) continue;
    downOf.get(source).add(target); upOf.get(target).add(source);
  }
  const down = new Map(sorted.map(item => [item, []])), up = new Map(sorted.map(item => [item, []]));
  for (const item of sorted) {
    for (const source of upOf.get(item)) down.get(source).push(item);
    for (const target of downOf.get(item)) up.get(target).push(item);
  }
  const ends = item => down.get(item).length === 0;
  const color = new Map(), backTo = new Map(sorted.map(item => [item, new Set()]));
  const search = start => {
    if (color.has(start)) return;
    color.set(start, 'open');
    for (const stack = [[start, 0]]; stack.length;) {
      const top = stack[stack.length - 1], next = down.get(top[0]);
      if (top[1] === next.length) { color.set(top[0], 'done'); stack.pop(); continue; }
      const child = next[top[1]++];
      if (color.get(child) === 'open') backTo.get(top[0]).add(child);
      else if (!color.has(child)) { color.set(child, 'open'); stack.push([child, 0]); }
    }
  };
  for (const item of sorted) if (!up.get(item).length) search(item);
  for (const item of sorted) search(item);
  const ordered = [], declared = new Set(), busy = new Set(), waiting = new Set();
  const frame = item => ({ item, declared: false, at: 0, next: up.get(item).filter(source => !backTo.get(source).has(item)) });
  const emit = (start, queue) => {
    if (declared.has(start) || busy.has(start)) return;
    waiting.delete(start); busy.add(start);
    for (const stack = [frame(start)]; stack.length;) {
      const top = stack[stack.length - 1];
      for (; top.at < top.next.length; top.at += 1) {
        const item = top.next[top.at];
        if (!declared.has(item) && !busy.has(item) && (top.declared || !waiting.has(item))) break;
      }
      if (top.at < top.next.length) {
        const item = top.next[top.at++];
        waiting.delete(item); busy.add(item); stack.push(frame(item));
      } else if (top.declared) {
        busy.delete(top.item); stack.pop();
      } else if (!top.next.every(source => declared.has(source))) {
        busy.delete(top.item); waiting.add(top.item); stack.pop();
      } else {
        top.declared = true; declared.add(top.item); ordered.push(top.item);
        const after = down.get(top.item);
        top.at = 0; top.next = after.length === 1 ? after : after.filter(item => !ends(item));
        if (after.length > 1) queue.push(...after.filter(ends));
      }
    }
  };
  const walk = start => {
    const queue = [];
    emit(start, queue);
    for (let at = 0; at < queue.length; at += 1) emit(queue[at], queue);
  };
  for (const item of sorted) if (!up.get(item).length) walk(item);
  for (const item of sorted) walk(item);
  return ordered;
}

/** Complete compacted log + pinned catalog + optional admission ports keyed by actor binding. */
function generateFlatProgram(commands, options, projections = new Map(), bundle = null, template = null) {
  if (!Array.isArray(commands)) return refuse('authoring.generator.complete-log-required');
  if (typeof options?.sdkVersion !== 'string' || !options.sdkVersion || !(options.specSet instanceof Uint8Array)
    || typeof options.bindings?.canonical !== 'function') return refuse('authoring.generator.pin-required');
  const actors = [], edges = [], flags = [], presentations = [], mounts = [], annotations = [], byName = new Map();
  let begin = null, validated = false, committed = false, validationEpoch;
  for (let index = 0; index < commands.length; index++) {
    const command = commands[index], kind = command?.kind;
    if (kind === 'UpsertActor' && COMBINATOR_NAMES.includes(command.declaration?.actorType)) return refuse('authoring.generator.combinator-actor-not-supported', index, kind, command.declaration.actorType);
    if (kind === 'UpsertExportMount') {
      if (command.mount?.value?.scope?.length || !bundle && Object.values(command.declaration?.roles ?? {}).some(e => e?.actor?.scope?.length)) {
        return refuse('authoring.generator.scope-not-supported', index, kind);
      }
      if (Object.hasOwn(command.declaration ?? {}, 'operations') && !Object.hasOwn(command.declaration, 'surface')) return refuse('authoring.generator.operations-not-supported', index, kind);
    }
    if (!CONTENT.has(kind) && !ENVELOPE.has(kind)) return refuse('authoring.generator.unsupported-command', index, kind ?? null);
    try {
      let context = 'mutation', payload;
      try { payload = declarationPayloadValue(pinnedCommand(command), { context }); }
      catch (error) {
        if (error.code !== 'ADDRESS_ARM_NOT_ADMITTED') throw error;
        context = 'snapshot'; payload = declarationPayloadValue(pinnedCommand(command), { context });
      }
      if (!commandSame(pinnedCommand(command), declarationCommandFromValue(kind, payload, { context }))) throw new TypeError('unrepresented or noncanonical command field');
      if (kind === 'BeginEpoch') {
        if (index !== 0) throw new TypeError('BeginEpoch must open the single epoch');
        if (command.scope.value.length) return refuse('authoring.generator.scope-not-supported', index, kind);
        if (!sameBytes(command.expectedEnvironment.specSet, options.specSet)) return refuse('authoring.generator.spec-set-mismatch', index, kind);
        begin = command;
      } else if (kind === 'ValidateEpoch') {
        if (!begin || validated || committed) throw new TypeError('ValidateEpoch is out of order');
        validated = true; validationEpoch = command.epoch;
      } else if (kind === 'CommitEpoch') {
        if (!begin || committed || index !== commands.length - 1 || validated && !same(command.epoch, validationEpoch)) throw new TypeError('CommitEpoch is out of order');
        committed = true;
      } else {
        if (validated || committed) throw new TypeError('content follows validation');
        if (kind === 'UpsertActor') {
          const name = command.actor.value.local, d = command.declaration;
          if (!bindingIdentifier(name) || byName.has(name)) throw new TypeError('actor identity is not a unique export binding');
          if (d.actorType === 'assemble') {
            const issue = assembleConfigIssue(d.config);
            if (issue) throw new TypeError(issue);
          }
          if (d.actorType === "join") {
            const issue = joinConfigIssue(d.config);
            if (issue) throw new TypeError(issue);
          }

          const resolved = options.bindings.canonical(d.actorType);
          if (resolved.status !== 'resolved') return refuse('authoring.generator.constructor-unresolved', index, kind, d.actorType);
          const b = resolved.binding;
          if (b.actorTypeId !== d.actorType || b.importSpecifier !== '@circular/core' || constructorSpelling(core[b.constructorExport]) !== d.actorType) {
            return refuse('authoring.generator.constructor-not-normalized', index, kind, d.actorType);
          }
          const catalog = options.catalog?.find(row => row.actor_type === d.actorType);
          const admission = options.admissions?.get(name);
          if (admission && (admission.actor_type !== d.actorType || !same(admission.config, d.config))) throw new TypeError('admission does not describe the stored actor');
          const boundaryPorts = d.actorType === 'input' ? { in_ports: [], out_ports: [{
            id: deriveBoundaryPortId('inlet', command.actor.value, 0n), primary: true,
          }] } : null;
          const fixedSourcePorts = !catalog ? FIXED_SOURCE_RECONSTRUCTION_PORTS[d.actorType] ?? null : null;
          const ports = admission ?? boundaryPorts ?? (catalog?.ports_unavailable_reason == null ? catalog : null) ?? fixedSourcePorts;
          if (d.actorType === 'match' && !ports) throw new TypeError('match ports unavailable in catalog/admission');
          const projection = projections.get(name);
          const actor = { name, index, order: actors.length, command, spelling: projection?.spelling ?? d.actorType,
            portNames: projection?.portNames, portsKnown: Boolean(projection || admission || fixedSourcePorts || (catalog && catalog.ports_unavailable_reason == null)),
            args: projection?.args ?? configArguments(d.actorType, d.config), inputs: projection?.inputs ?? ports?.in_ports ?? [], outputs: projection?.outputs ?? ports?.out_ports ?? [] };
          actors.push(actor); byName.set(name, actor);
        } else if (kind === 'UpsertEdge') edges.push({ index, command });
        else if (kind === 'UpsertExportMount') mounts.push({ index, command });
        else if (kind === 'SetPresentation') presentations.push({ index, command });
        else if (kind === 'UpsertAnnotation') annotations.push({ index, command });
        else flags.push({ index, command });
      }
    } catch (error) { return refuse('authoring.generator.invalid-command', index, kind, error.message); }
  }
  if (begin && !committed) return refuse('authoring.generator.incomplete-epoch');
  const flow = flowOrder(actors, actor => actor.name, edges.map(({ command }) =>
    [command.declaration.from.actor.local, command.declaration.to.actor.local]));
  actors.splice(0, actors.length, ...flow);
  actors.forEach((actor, order) => { actor.order = order; });
  if (bundle) for (const actor of actors) bundle.actors.set([...bundle.scope, actor.name].join('/'), {
    ...actor, name: [...bundle.scope, actor.name].join('.actors.'),
  });
  const keyOf = local => [...(bundle?.scope ?? []), local].join('/');
  const actorOwner = actor => ({ actor: keyOf(actor.name) });
  const lines = [], imports = new Map(), used = new Set([...byName.keys(), ...(template?.bindings ?? []), ...(template?.name ? [template.name] : [])]), notesById = new Map();
  const say = (text, owner, refs = []) => lines.push({ text, owner, refs: [...new Set(refs)] });
  for (const { index, command } of annotations) {
    try {
      const note = noteFromCommand(command);
      if (notesById.has(note.id) || note.refs.some(ref => !byName.has(ref))) throw new TypeError('duplicate Note or unresolved ref');
      let name = bindingIdentifier(note.id) ? note.id : 'explanation';
      while (used.has(name)) name += '_';
      used.add(name);
      notesById.set(note.id, { ...note, name, order: actors.length + notesById.size });
    } catch (error) { return refuse('authoring.generator.annotation-not-spellable', index, command.kind, error.message); }
  }
  let finalPresentations;
  const primary = (ports, id) => ports.filter(p => p.primary).length === 1 && ports.some(p => p.primary && p.id === id);
  const prefix = edge => (edge.command.declaration.attrs.preprocess ?? []).map(step => `.${step.kind}(${configArguments(step.kind, step.config).join(', ')})`).join('');
  const portName = (actor, port, direction) => actor.portNames?.[direction].get(port) ?? port;
  const endpoint = (actor, port, direction) => {
    const ports = direction === 'out' ? actor.outputs : actor.inputs;
    if (actor.portsKnown && !ports.some(p => p.id === port)) {
      throw new TypeError(`unknown ${direction} port ${actor.name}.${port} in catalog/admission`);
    }
    if (actor.spelling === 'match' && direction === 'out') return actor.name + property(port);
    return actor.name + (primary(ports, port) ? '' : `.${direction}${property(portName(actor, port, direction))}`);
  };
  const resolve = key => {
    if (key.scope.length) throw new TypeError('command references a scoped actor');
    const actor = byName.get(key.local);
    if (!actor) throw new TypeError('command references an undeclared actor');
    return actor;
  };
  try {
    const byOwner = new Map();
    for (const entry of presentations) {
      const address = entry.command.owner.actor ?? entry.command.owner.annotation;
      const owner = entry.command.owner.actor ? resolve(address.value) : notesById.get(address.value.local);
      if (!owner || address.value.scope.length) throw new TypeError('command references an undeclared presentation owner');
      entry.owner = entry.command.owner.actor ? actorOwner(owner) : { annotation: keyOf(owner.id) };
      byOwner.set(owner, entry);
    }
    for (const [owner, entry] of byOwner) {
      const p = entry.command.presentation, chain = [];
      const add = (method, ...args) => chain.push(`.${method}(${args.map(value => literal(value)).join(', ')})`);
      if (p.label != null) add('label', p.label);
      if (p.fixed != null) add('at', Number(p.fixed.x), Number(p.fixed.y));
      if (p.size != null) add('size', Number(p.size.w), Number(p.size.h));
      if (p.board != null) add('board', ...['col', 'row', 'w', 'h'].map(k => Number(p.board[k])));
      if (p.group != null) add('group', p.group);
      if (p.collapsed) add('collapsed', true);
      if (p.view != null) {
        if (p.view.config == null) add('view', p.view.kind);
        else add('view', p.view.kind, p.view.config);
      }
      if (p.anchor === 'Flow') return refuse('authoring.generator.presentation-not-spellable', entry.index, entry.command.kind, 'anchor Flow');
      entry.ready = owner.order; entry.refs = [];
      if (p.anchor != null) {
        const target = resolve(p.anchor.target);
        entry.ready = Math.max(entry.ready, target.order); entry.refs = [keyOf(target.name)];
        const method = p.anchor.kind === 'Relative'
          ? { Before: 'before', After: 'after' }[p.anchor.relation]
          : { Horizontal: 'alignHorizontal', Vertical: 'alignVertical' }[p.anchor.axis];
        chain.push(`.${method}(${target.name})`);
      }
      if (!chain.length && p.collapsed === false) add('collapsed', false);
      entry.text = chain.length ? `${owner.name}${chain.join('')};` : null;
    }
    finalPresentations = [...byOwner.values()];
    for (const edge of edges) {
      const d = edge.command.declaration;
      edge.source = resolve(d.from.actor); edge.target = resolve(d.to.actor);
      if (['match', 'join'].includes(edge.target.spelling)) endpoint(edge.target, d.to.port, 'in');
      const pairCount = edges.filter(e => same(e.command.declaration.from, d.from) && same(e.command.declaration.to, d.to)).length;
      edge.options = {};
      if (pairCount > 1 || d.ordinal !== 0) edge.options.ordinal = d.ordinal;
      if (!same(d.attrs.delay, DEFAULT_EDGE_ATTRS.delay)) edge.options.delay = d.attrs.delay;
      if (!same(d.attrs.policy, DEFAULT_EDGE_ATTRS.policy)) edge.options.policy = d.attrs.policy;
      edge.fold = edge.source.order < edge.target.order && Object.keys(edge.options).length === 0
        && !['pipeline_actor', 'replicator'].includes(edge.target.command.declaration.actorType)
        && (edge.target.spelling !== 'match' || edges.filter(e => same(e.command.declaration.to.actor, d.to.actor)).length === 1);
      edge.ready = Math.max(edge.source.order, edge.target.order);
    }
    for (const actor of actors) {
      const incoming = edges.filter(e => e.target === actor && e.fold), args = [...actor.args];
      let receiver;
      if (incoming.length === 1 && actor.spelling !== 'match') {
        const edge = incoming[0], d = edge.command.declaration;
        receiver = `${endpoint(edge.source, d.from.port, 'out')}${prefix(edge)}.${publicName(actor.spelling)}`;
        if (!primary(actor.inputs, d.to.port)) args.push(`{ at: ${literal(portName(actor, d.to.port, 'in'))} }`);
      } else {
        if (!imports.has(actor.spelling)) {
          const sdkName = publicName(actor.spelling);
          let alias = sdkName;
          if (used.has(alias)) alias = `make${sdkName[0].toUpperCase()}${sdkName.slice(1)}`;
          while (used.has(alias)) alias += '_';
          used.add(alias); imports.set(actor.spelling, alias);
        }
        receiver = imports.get(actor.spelling);
        if (incoming.length) {
          const groups = new Map();
          for (const edge of incoming) {
            const d = edge.command.declaration;
            if (!groups.has(d.to.port)) groups.set(d.to.port, []);
            groups.get(d.to.port).push(endpoint(edge.source, d.from.port, 'out') + prefix(edge));
          }
          if (actor.spelling === 'match' && incoming.length === 1) args.push([...groups.values()][0][0]);
          else args.push(`{ ${[...groups].map(([port, sources]) => `${propertyKey(portName(actor, port, 'in'))}: ${sources.length > 1 ? `[${sources.join(', ')}]` : sources[0]}`).join(', ')} }`);
        }
      }
      say(`export let ${actor.name} = ${receiver}(${args.join(', ')});`, actorOwner(actor), incoming.map(edge => keyOf(edge.source.name)));
      for (const entry of finalPresentations.filter(p => p.ready === actor.order && p.text)) say(entry.text, entry.owner, entry.refs);
      if (!same(actor.command.declaration.flags, DEFAULT_FLAGS)) say(`${actor.name}.setFlags(${literal(actor.command.declaration.flags)});`, actorOwner(actor));
      for (const edge of edges.filter(e => !e.fold && e.ready === actor.order)) {
        const d = edge.command.declaration;
        say(`${endpoint(edge.source, d.from.port, 'out')}${prefix(edge)}.into(${endpoint(edge.target, d.to.port, 'in')}${Object.keys(edge.options).length ? `, ${literal(edge.options)}` : ''});`,
          actorOwner(edge.target), [keyOf(edge.source.name)]);
      }
    }
    for (const entry of flags) {
      const actor = resolve(entry.command.actor.value);
      say(`${actor.name}.setFlags(${literal(entry.command.flags)});`, actorOwner(actor));
    }
  } catch (error) { return refuse('authoring.generator.invalid-command', null, null, error.message); }
  const mountNames = new Set();
  let exportAlias = 'circularExports';
  while (used.has(exportAlias)) exportAlias += '_';
  let hasSurfaces = false;
  for (const { index, command } of mounts) {
    try {
      const name = command.mount.value.local, roles = command.declaration.roles;
      if (mountNames.has(name) || !Object.keys(roles).length && !Object.hasOwn(command.declaration, 'surface')) throw new TypeError('mount requires unique name and nonempty roles');
      mountNames.add(name);
      for (const role of ['request', 'progress', 'result', 'error']) {
        if (!Object.hasOwn(roles, role)) continue;
        const e = roles[role], actor = e.actor.scope.length && bundle
          ? bundle.actors.get([...bundle.scope, ...e.actor.scope.map(s => s.name), e.actor.local].join('/')) : resolve(e.actor);
        const receiverActor = actor && { ...actor, name: [...e.actor.scope.map(s => s.name), e.actor.local].join('.actors.') };
        if (!actor) throw new TypeError('mount references an undeclared actor');
        if (role === 'request' && (actor.command.declaration.actorType !== 'input' || !primary(actor.outputs, e.port))) {
          throw new TypeError('request requires the writable boundary primary endpoint');
        }
        const receiver = role === 'request' ? receiverActor.name
          : actor.command.declaration.actorType === 'input' ? `${receiverActor.name}.out${property(e.port)}`
          : actor.outputs.some(p => p.id === e.port) || !actor.inputs.some(p => p.id === e.port) ? endpoint(receiverActor, e.port, 'out')
          : `${receiverActor.name}.in${property(portName(actor, e.port, 'in'))}`;
        say(`${receiver}.mount(${literal(name)}, ${literal(role)});`,
          { actor: [...(bundle?.scope ?? []), ...e.actor.scope.map(s => s.name), e.actor.local].join('/') });
      }
      if (Object.hasOwn(command.declaration, 'surface')) {
        hasSurfaces = true;
        say(`${exportAlias}.surface(${literal(name)}, ${printExportSurface(command.declaration.surface, literal, exportAlias + '.')});`, { mount: keyOf(name) });
      }
    } catch (error) { return refuse('authoring.generator.invalid-command', index, command.kind, error.message); }
  }
  if (notesById.size) {
    let alias = 'note';
    if (used.has(alias)) alias = 'makeNote';
    while (used.has(alias)) alias += '_';
    used.add(alias); imports.set('note', alias);
    for (const note of notesById.values()) {
      const id = note.name === note.id ? '' : `id: ${literal(note.id)}, `;
      say(`export let ${note.name} = ${alias}({ ${id}refs: [${note.refs.join(', ')}], text: ${literal(note.text)} });`,
        { annotation: keyOf(note.id) }, note.refs.map(keyOf));
      for (const entry of finalPresentations.filter(p => p.ready === note.order && p.text)) say(entry.text, entry.owner, entry.refs);
    }
  }
  const namedImports = [...imports].map(([spelling, alias]) => publicName(spelling) === alias
    ? alias : `${publicName(spelling)} as ${alias}`);
  const head = [...(template?.imports ?? []),
    ...(namedImports.length ? [`import { ${namedImports.join(', ')} } from "@circular/core";`] : []),
    ...(hasSurfaces ? [`import * as ${exportAlias} from "@circular/exports";`] : []), ''];
  const body = template?.name ? [`export function ${template.name}() {`,
    ...lines.map(({ text }) => '  ' + text.replace(/^export let /, 'let ')), '}'] : lines.map(({ text }) => text);
  const source = [...head, ...body, ''].join('\n');
  const first = head.length + (template?.name ? 2 : 1);
  const statements = lines.map(({ owner, refs }, index) => Object.freeze({ module: 'main.ts', line: first + index,
    owner: Object.freeze(template?.name ? { template: template.name } : owner), refs: Object.freeze(template?.name ? [] : refs) }));
  return Object.freeze({ status: 'complete', diagnostics: [], value: Object.freeze({
    program: { entry: 'main.ts', modules: new Map([['main.ts', new TextEncoder().encode(source)]]) },
    pin: { sdkVersion: options.sdkVersion, specSet: new Uint8Array(options.specSet) }, begin,
    statements: Object.freeze(statements),
  }) });
}

const TEMPLATE_VALUE = new Set(['UpsertTemplate', 'RetireTemplate']);
const CONTAINERS = ['pipeline_actor', 'replicator'];

/** The stored Template values a log ends with, by name. A retired name is gone. */
function templateValues(commands) {
  const templates = new Map();
  for (const command of commands.filter(c => TEMPLATE_VALUE.has(c?.kind))) {
    if (command.kind === 'RetireTemplate') { declarationPayloadValue(command); templates.delete(command.name); continue; }
    if (Object.keys(command).sort().join(',') !== 'commands,kind,name'
      || typeof command.name !== 'string' || !bindingIdentifier(command.name)
      || !Array.isArray(command.commands)) throw new TypeError('invalid template value');
    const canonical = compactTemplateCommands(command.commands);
    if (!same(canonical, command.commands)) throw new TypeError('template commands are not compact and canonical');
    templates.set(command.name, command.commands);
  }
  return templates;
}

function templateImports(reserved, specifier) {
  const used = new Set(reserved), references = new Map();
  return {
    used, references,
    reference(template) {
      if (!references.has(template)) {
        let alias = template;
        if (used.has(alias)) alias = `__template${references.size}`;
        while (used.has(alias)) alias += '_';
        used.add(alias); references.set(template, alias);
      }
      return references.get(template);
    },
    template: name => ({ name, bindings: [...references.values()], imports: [...references].map(([target, alias]) =>
      `import { ${target}${target === alias ? '' : ` as ${alias}`} } from ${JSON.stringify(specifier(target))};`) }),
  };
}

/**
 * A container bound to a stored Template value, in a Template function or at the root. Its config
 * names the value; its ports are the value's boundaries.
 */
function templateContainer(command, templates, imports, ports) {
  const { actorType, config } = command.declaration;
  if (!templates.has(config?.template) || !Array.isArray(config.in) || !Array.isArray(config.out)) throw new TypeError('container template missing');
  const expected = actorType === 'replicator' ? ['at','capacity','in','out','template','ttl'] : ['in','out','template'];
  if (Object.keys(config).sort().join(',') !== expected.join(',')) throw new TypeError('invalid template config');
  if (actorType === 'replicator' && !replicatorPolicy({at:config.at,ttl:config.ttl,capacity:config.capacity})) throw new TypeError('invalid replicator policy');
  const args = [`{ template: ${imports.reference(config.template)}, in: ${literal(config.in)}, out: ${literal(config.out)}${actorType === 'replicator' ? `, at: ${literal(config.at)}, ttl: ${literal(config.ttl)}, capacity: ${literal(config.capacity)}` : ''} }`];
  const boundaries = (type, direction) => templates.get(config.template)
    .filter(c => c.kind === 'UpsertActor' && c.declaration.actorType === type)
    .map(c => [deriveBoundaryPortId(direction, c.actor.value, 0n), c.declaration.config.label]);
  const portNames = { in: new Map(boundaries('input', 'inlet')), out: new Map(boundaries('output', 'outlet')) };
  return { spelling: actorType, args, portNames,
    inputs: actorType === 'replicator' ? ports?.in_ports ?? [] : [...portNames.in.keys()].map(id => ({ id, primary: false })),
    outputs: [...portNames.out.keys()].map(id => ({ id, primary: false })) };
}

/**
 * One stored Template value as its function module, `templates/<name>.ts`, printed by the flat idiom
 * emitter. A Template value has its own namespace: the admissions of standing actors, keyed by their
 * paths, never describe its actors, so its ports come from the catalog and its boundaries.
 */
function generateTemplateModule(name, local, templates, options) {
  const actors = local.filter(c => c.kind === 'UpsertActor');
  const projections = new Map(), imports = templateImports([...actors.map(c => c.actor.value.local), name], target => `./${target}`);
  const catalogPorts = actorType => options.catalog?.find(r => r.actor_type === actorType);
  for (const command of actors) {
    const { actorType, config } = command.declaration, binding = command.actor.value.local;
    if (command.actor.value.scope.length) throw new TypeError('runtime descendant is not authorable');
    if (boundarySpelling(actorType)) {
      const source = boundarySource(actorType, config);
      if (!source) throw new TypeError('invalid template boundary');
      const input = actorType === 'input', port = deriveBoundaryPortId(input ? 'inlet' : 'outlet', command.actor.value, 0n);
      projections.set(binding, { spelling: boundarySpelling(actorType), args: [literal(source)],
        inputs: input ? [] : [{ id: port, primary: true }], outputs: input ? [{ id: port, primary: true }] : [] });
    } else if (CONTAINERS.includes(actorType)) projections.set(binding, templateContainer(command, templates, imports, catalogPorts(actorType)));
  }
  return generateFlatProgram(local, { ...options, admissions: new Map() }, projections, null, imports.template(name));
}

export function generateProgram(commands, options) {
  if (!Array.isArray(commands) || !commands.some(c => TEMPLATE_VALUE.has(c?.kind) || ['UpsertScope', 'MoveToScope'].includes(c?.kind)
    || c?.kind === 'UpsertActor' && (c.actor?.value?.scope?.length || CONTAINERS.includes(c.declaration?.actorType)))) {
    return generateFlatProgram(commands, options);
  }
  const templateModules = new Map();
  let templates;
  try {
    templates = templateValues(commands);
    for (const [name, local] of [...templates].sort(([a], [b]) => compareSourceNames(a, b))) {
      const result = generateTemplateModule(name, local, templates, options);
      if (result.status !== 'complete') return result;
      const module = `templates/${name}.ts`;
      templateModules.set(module, { bytes: result.value.program.modules.get('main.ts'),
        statements: result.value.statements.map(statement => Object.freeze({ ...statement, module })) });
    }
  } catch (error) { return refuse('authoring.generator.invalid-template', null, 'UpsertTemplate', error.message); }
  const pathOf = scope => scope.map(segment => {
    if (Object.keys(segment).length !== 1 || !bindingIdentifier(segment.name)) throw new TypeError('scope name is not a BindingIdentifier');
    return segment.name;
  }).join('/');
  const actorId = key => [...key.scope.map(s => s.name), key.local].join('/');
  const scopes = new Map(), actors = new Map(), content = [], envelopes = [];
  let commandIndex = null, commandKind = null;
  const at = command => { commandIndex = commands.indexOf(command); commandKind = command?.kind ?? null; };
  let begin = null, ended = false, validated = false, validationEpoch;
  try {
    for (const [index, command] of commands.entries()) {
      at(command);
      const kind = command?.kind;
      if (TEMPLATE_VALUE.has(kind)) continue;
      if (kind === 'UpsertActor' && COMBINATOR_NAMES.includes(command.declaration?.actorType)) return refuse('authoring.generator.combinator-actor-not-supported', index, kind, command.declaration.actorType);
      if (kind === 'UpsertExportMount') {
        if (Object.hasOwn(command.declaration ?? {}, 'operations') && !Object.hasOwn(command.declaration, 'surface')) return refuse('authoring.generator.operations-not-supported', index, kind);
        for (const endpoint of Object.values(command.declaration?.roles ?? {})) {
          pathOf(endpoint.actor.scope);
          if (!bindingIdentifier(endpoint.actor.local)) throw new TypeError('mount endpoint is not a BindingIdentifier');
        }
      }
      if (!CONTENT.has(kind) && !ENVELOPE.has(kind) && !['UpsertScope', 'MoveToScope'].includes(kind)) return refuse('authoring.generator.unsupported-command', index, kind);
      let context = 'mutation', payload;
      try { payload = declarationPayloadValue(pinnedCommand(command), { context }); }
      catch (error) { if (error.code !== 'ADDRESS_ARM_NOT_ADMITTED') throw error; context = 'snapshot'; payload = declarationPayloadValue(pinnedCommand(command), { context }); }
      if (!commandSame(pinnedCommand(command), declarationCommandFromValue(kind, payload, { context }))) return refuse('authoring.generator.invalid-command', index, kind);
      if (ENVELOPE.has(kind)) {
        envelopes.push(command);
        if (kind === 'BeginEpoch') {
          if (index !== 0 || command.scope.value.length) throw new TypeError('bundle requires a root-relative epoch');
          begin = command;
        } else if (kind === 'ValidateEpoch') {
          if (!begin || validated || ended) throw new TypeError('invalid validation order');
          validated = true; validationEpoch = command.epoch;
        } else {
          if (!begin || ended || index !== commands.length - 1 || validated && !same(validationEpoch, command.epoch)) throw new TypeError('invalid commit order');
          ended = true;
        }
        continue;
      }
      if (validated || ended) throw new TypeError('content follows validation');
      if (kind === 'UpsertScope') {
        const path = pathOf(command.scope.value);
        if (!path || scopes.has(path)) throw new TypeError('duplicate or root scope declaration');
        scopes.set(path, command);
      } else if (kind === 'MoveToScope') {
        return refuse('authoring.generator.compacted-log-required', index, kind);
      } else {
        content.push(command);
        if (kind === 'UpsertActor') {
          pathOf(command.actor.value.scope);
          const id = actorId(command.actor.value);
          if (actors.has(id)) throw new TypeError('duplicate actor identity');
          actors.set(id, command);
        }
      }
    }
    if (begin && !ended) return refuse('authoring.generator.incomplete-epoch');
    const noteIds = new Set();
    for (const command of content) {
      if (command.kind !== 'UpsertAnnotation') continue;
      try {
        const scope = command.annotation.value.scope, path = pathOf(scope);
        if (path && !scopes.has(path)) throw new TypeError('Note scope is not declared');
        const note = noteFromCommand(command, scope), id = JSON.stringify([scope, note.id]);
        if (noteIds.has(id) || note.refs.some(local => ![...actors.values()].some(actor => same(actor.actor.value, { scope, local })))) throw new TypeError('duplicate Note or unresolved ref');
        noteIds.add(id);
      } catch (error) { return refuse('authoring.generator.annotation-not-spellable', commands.indexOf(command), command.kind, error.message); }
    }
    const sourceScope = command => command.declaration.actorType === 'pipeline_actor' && command.declaration.config === null;
    for (const [id, command] of actors) {
      at(command);
      if (sourceScope(command) && !scopes.has(id)) throw new TypeError('container has no explicit paired scope');
      const parent = pathOf(command.actor.value.scope);
      if (parent && !scopes.has(parent)) throw new TypeError('actor has no declared scope');
    }
    const projections = new Map();
    const projectionsOf = path => { if (!projections.has(path)) projections.set(path, new Map()); return projections.get(path); };
    const byName = (a, b) => compareSourceNames(a.actor.value.local, b.actor.value.local);
    const root = templateImports([...actors.values()].filter(c => !c.actor.value.scope.length).map(c => c.actor.value.local), target => `./templates/${target}`);
    for (const command of [...actors.values()].filter(c => CONTAINERS.includes(c.declaration.actorType) && !sourceScope(c)).sort(byName)) {
      at(command);
      if (command.actor.value.scope.length) throw new TypeError('template-bound container in a source scope has no SDK spelling');
      projectionsOf('').set(command.actor.value.local, templateContainer(command, templates, root,
        options?.admissions?.get(command.actor.value.local) ?? options?.catalog?.find(row => row.actor_type === command.declaration.actorType)));
    }
    for (const [path, scope] of scopes) {
      at(scope);
      const container = actors.get(path);
      if (scope.declaration.role === 'Template') throw new TypeError('template scope has no authored form');
      if (!container || !sourceScope(container)) throw new TypeError('scope has no paired container of matching role');
      const parent = pathOf(container.actor.value.scope);
      const config = { source: `scopes/${path}.ts`, in: [], out: [] }, seen = new Set();
      const portNames = { in: new Map(), out: new Map() };
      for (const [direction, type] of [['in', 'input'], ['out', 'output']]) {
        const entries = scope.declaration.boundary[direction === 'in' ? 'inlets' : 'outlets'];
        const topics = new Set();
        for (const binding of entries) {
          const id = actorId(binding.inner.actor), child = actors.get(id);
          if (!child || pathOf(child.actor.value.scope) !== path || child.declaration.actorType !== type || seen.has(id)) throw new TypeError('boundary identity or direction mismatch');
          seen.add(id);
          const source = boundarySource(type, child.declaration.config), topic = source?.topic;
          if (!source) throw new TypeError('boundary config differs from parent topic');
          if (topics.has(topic)) throw new TypeError('duplicate boundary topic');
          topics.add(topic);
          const { inner: { port: inner }, outer } = binding;
          config[direction].push(topic);
          portNames[direction].set(outer, topic);
          projectionsOf(path).set(child.actor.value.local, { spelling: actorSpelling(type, child.actor.value.scope), args: [literal(source)],
            inputs: direction === 'out' ? [{ id: inner, primary: true }] : [], outputs: direction === 'in' ? [{ id: inner, primary: true }] : [] });
        }
      }
      for (const [id, child] of actors) if (pathOf(child.actor.value.scope) === path && ['input', 'output'].includes(child.declaration.actorType) && !seen.has(id)) throw new TypeError('boundary actor has no counterpart');
      projectionsOf(parent).set(container.actor.value.local, { spelling: 'pipeline_actor',
        args: [literal(config)], portNames,
        inputs: [...portNames.in.keys()].map(id => ({ id, primary: false })),
        outputs: [...portNames.out.keys()].map(id => ({ id, primary: false })) });
    }
    const groups = new Map([['', []], ...[...scopes.keys()].map(path => [path, []])]);
    for (const command of content) {
      at(command);
      if (command.kind === 'UpsertExportMount') {
        const path = pathOf(command.mount.value.scope);
        if (!groups.has(path)) throw new TypeError('mount scope not declared');
        const scope = command.mount.value.scope;
        const roles = Object.fromEntries(Object.entries(command.declaration.roles).map(([role, endpoint]) => {
          if (!same(endpoint.actor.scope.slice(0, scope.length), scope)) throw new TypeError('mount endpoint is outside its scope');
          return [role, { ...endpoint, actor: { ...endpoint.actor, scope: endpoint.actor.scope.slice(scope.length) } }];
        }));
        groups.get(path).push({ ...command, mount: { ...command.mount, value: { ...command.mount.value, scope: [] } },
          declaration: { ...command.declaration, roles } });
      } else if (command.kind === 'UpsertEdge') {
        const d = command.declaration, path = pathOf(d.from.actor.scope);
        if (path !== pathOf(d.to.actor.scope)) throw new TypeError('edge crosses module boundary without container');
        if (!groups.has(path)) throw new TypeError('edge scope not declared');
        const endpoint = e => ({ ...e, actor: { ...e.actor, scope: [] } });
        const from = endpoint(d.from), to = endpoint(d.to);
        groups.get(path).push({ ...command, edge: { ...command.edge, value: { from, to, ordinal: d.ordinal } }, declaration: { ...d, from, to } });
      } else if (command.kind === 'SetPresentation') {
        const path = pathOf((command.owner.actor ?? command.owner.annotation).value.scope), p = command.presentation;
        if (!groups.has(path)) throw new TypeError('presentation scope not declared');
        let anchor = p.anchor;
        if (anchor != null && anchor !== 'Flow') {
          if (pathOf(anchor.target.scope) !== path) throw new TypeError('presentation anchor crosses module boundary');
          anchor = { ...anchor, target: { ...anchor.target, scope: [] } };
        }
        groups.get(path).push({ ...command,
          owner: Object.fromEntries(Object.entries(command.owner).map(([kind, address]) => [kind, { ...address, value: { ...address.value, scope: [] } }])),
          presentation: { ...p, ...(anchor === undefined ? {} : { anchor }) } });
      } else if (command.kind === 'UpsertAnnotation') {
        const scope = command.annotation.value.scope, path = pathOf(scope);
        if (!groups.has(path)) throw new TypeError('Note scope is not declared');
        groups.get(path).push({ ...command,
          annotation: { ...command.annotation, value: { ...command.annotation.value, scope: [] } },
          declaration: { ...command.declaration, refs: command.declaration.refs.map(ref => ({ ...ref, scope: [] })) } });
      } else {
        const path = pathOf(command.actor.value.scope);
        if (!groups.has(path)) throw new TypeError('actor scope not declared');
        groups.get(path).push({ ...command, actor: { ...command.actor, value: { ...command.actor.value, scope: [] } } });
      }
    }
    const printed = new Map(), mountActors = new Map(); let pin;
    const ordered = [...groups].sort(([a], [b]) => (a ? a.split('/').length : 0) - (b ? b.split('/').length : 0));
    for (const [path, local] of [...ordered.filter(([path]) => path).reverse(), ordered[0]]) {
      const localAdmissions = new Map(local.filter(c => c.kind === 'UpsertActor').flatMap(c => {
        const row = options?.admissions?.get(path ? `${path}/${c.actor.value.local}` : c.actor.value.local);
        return row ? [[c.actor.value.local, row]] : [];
      }));
      const result = generateFlatProgram(path ? local : [...envelopes.filter(c => c.kind === 'BeginEpoch'), ...local, ...envelopes.filter(c => c.kind !== 'BeginEpoch')],
        { ...options, admissions: localAdmissions }, projectionsOf(path), { actors: mountActors, scope: path ? path.split('/') : [] }, path ? null : root.template(null));
      if (result.status !== 'complete') return result;
      pin = result.value.pin;
      const module = path ? `scopes/${path}.ts` : 'main.ts';
      printed.set(module, { bytes: result.value.program.modules.get('main.ts'),
        statements: result.value.statements.map(statement => Object.freeze({ ...statement, module })) });
    }
    const registrations = [...templates.keys()].filter(target => !root.references.has(target)).sort().map((target, index) => {
      let alias = `__registeredTemplate${index}`;
      while (root.used.has(alias)) alias += '_';
      root.used.add(alias);
      return `export { ${target} as ${alias} } from ${JSON.stringify(`./templates/${target}`)};`;
    });
    if (registrations.length) {
      const main = printed.get('main.ts');
      printed.set('main.ts', { bytes: new TextEncoder().encode([...registrations, new TextDecoder().decode(main.bytes)].join('\n')),
        statements: main.statements.map(statement => Object.freeze({ ...statement, line: statement.line + registrations.length })) });
    }
    const parentFirst = [...ordered.map(([path]) => {
      const module = path ? `scopes/${path}.ts` : 'main.ts';
      return [module, printed.get(module)];
    }), ...templateModules];
    return Object.freeze({ status: 'complete', diagnostics: [], value: Object.freeze({
      program: { entry: 'main.ts', modules: new Map(parentFirst.map(([module, { bytes }]) => [module, bytes])) }, pin, begin,
      statements: Object.freeze(parentFirst.flatMap(([, { statements }]) => statements)) }) });
  } catch (error) { return refuse('authoring.generator.invalid-command', commandIndex, commandKind, error.message); }
}

/** The owner-local session consumes More pages on one correlation; only its complete result is admitted. */
export async function generateProgramFromSession(session, options) {
  let result;
  try { result = await session.authoringSnapshot(options.scope ?? [], options.pageLimit ?? 256); }
  catch (error) { return refuse("authoring.generator.snapshot-failed", null, null, `${error?.code ?? error?.name ?? "Error"}: ${String(error?.message ?? error)}`); }
  if (result?.status !== "accepted" || !Array.isArray(result.value?.commands)) {
    return refuse("authoring.generator.snapshot-incomplete", null, null, result?.diagnostic?.message ?? result?.diagnostics?.map((diagnostic) => diagnostic.message).join("; ") ?? result?.status ?? null);
  }
  if (!sameBytes(result.value.anchor?.environment?.specSet, options.specSet)) {
    return refuse("authoring.generator.spec-set-mismatch");
  }
  return generateProgram(result.value.commands, options);
}
