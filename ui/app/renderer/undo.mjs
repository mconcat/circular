import { sameValue as same } from '@circular/protocol';
import { actorCreateAdmission } from '@circular/client';
import { key, addressPath } from './scene.mjs';
import { LENS, ACTOR_KINDS, absolute, keysOf } from './verbs.mjs';

const target = (kind, value) => ({ kind, value, id: key([kind, value]) });
const actorKeys = at => ACTOR_KINDS.map(kind => target(kind, kind === 'presentation' ? { actor: at } : at));
const named = command => keysOf(command).map(([kind, at]) => target(kind, at));
export function touchedKeys(commands) {
  const keys = new Map();
  for (const k of commands.flatMap(named)) if (!keys.has(k.id)) keys.set(k.id, k);
  return [...keys.values()];
}

export const valueAt = (graph, k) => LENS[k.kind].view(graph.declared, k.value);
export function undoEntry(graph, commands) {
  const keys = touchedKeys(commands);
  return { before: new Map(keys.map(k => [k.id, valueAt(graph, k)])), touchedKeys: keys };
}

const under = (scope, prefix) => scope.length >= prefix.length && key(scope.slice(0, prefix.length)) === key(prefix);
const depth = k => k.kind === 'actor' ? k.value.scope.length + 1 : k.value.length;
const segment = s => s.name ?? `${s.of}[${String(s.key)}]`;
const where = k => k.kind === 'edge'
  ? `${addressPath(k.value.from.actor)}.${k.value.from.port} → ${addressPath(k.value.to.actor)}.${k.value.to.port}`
  : k.kind === 'scope' ? '/' + k.value.map(segment).join('/') : addressPath(k.kind === 'presentation' ? k.value.actor ?? k.value.annotation : k.value);
const verbsOf = (k, value, flags) => LENS[k.kind].verbs(k.value, value, flags);

const authored = (k, value) => k.kind === 'scope' && value !== undefined ? { ...value, boundary: { inlets: [], outlets: [] } } : value;
const endpoint = (actor, port) => key([actor, port]);
const bindings = value => [...(value?.boundary?.inlets ?? []).map(b => ({ ...b, side: 'out' })),
  ...(value?.boundary?.outlets ?? []).map(b => ({ ...b, side: 'in' }))];

/**
 * The ports the daemon's create admission answers now for each boundary actor `entry`
 * would stand again — `key(actor)` → `{in, out}`, the port ids on each side. A boundary actor is one
 * a binding of a scope the entry sets back names, so the daemon's own boundary says which actors
 * are asked, and no actor type is read here. The admission derives a boundary actor's port from its
 * address and its current retire generation (actor_catalog.rs), which the Undo epoch folds on. An
 * actor whose answer is refused or lost has no entry, and `restore` reports what names its port.
 */
export async function boundaryPorts(session, graph, entry) {
  const ports = new Map();
  for (const scope of entry.touchedKeys.filter(k => k.kind === 'scope')) {
    for (const { inner } of bindings(entry.before.get(scope.id))) {
      const actor = target('actor', inner.actor), declared = entry.before.get(actor.id);
      if (declared === undefined || valueAt(graph, actor) !== undefined || ports.has(key(inner.actor))) continue;
      let answer;
      try { answer = await actorCreateAdmission(session, declared.actorType, declared.config, inner.actor); } catch { continue; }
      const row = answer.status === 'accepted' ? answer.value.items[0] : undefined;
      if (row) ports.set(key(inner.actor), { in: row.in_ports.map(p => p.id), out: row.out_ports.map(p => p.id) });
    }
  }
  return ports;
}

/**
 * The new edit that sets `entry`'s keys back to its `before` on the confirmed fold `graph`.
 *
 * Returns `{commands, report}`. `commands` are published DeclarationCommands in an order the
 * daemon admits: wire and note retirements, upserted scopes and actors (outer first), moves,
 * retired actors and scopes (inner first), restored notes, flags and presentations, then wires and mounts.
 * `report` names every key that was not set back — the same form as a re-application report:
 * the key, its reason code and the intent left unspelled. `marks.edited` holds the
 * keys another author's accepted epoch named after the entry's edit, and `marks.dependents` the
 * keys it declared something on (a wire to the actor, an actor in the scope). What they named
 * stays as that author left it, and nothing they built on is retired from under them, so no undo
 * erases an intervening edit. `ports` is `boundaryPorts`'s answer for the entry:
 * a wire or mount on a boundary port of a scope this edit stands again is declared on the port the
 * daemon answers now, and one with no answer is reported gone.
 */
export function restore(graph, entry, { edited = new Set(), dependents = new Set() } = {}, ports = new Map()) {
  const keys = entry.touchedKeys, index = new Map(keys.map(k => [k.id, k]));
  const want = k => entry.before.get(k.id);
  const now = new Map(keys.map(k => [k.id, valueAt(graph, k)]));
  const held = new Set(keys.filter(k => edited.has(k.id)).map(k => k.id));
  const kept = k => (k.kind === 'actor'
    ? [k, ...actorKeys(k.value).slice(1), target('scope', [...k.value.scope, { name: k.value.local }])] : k.kind === 'annotation' ? [k, target('presentation', { annotation: k.value })] : [k])
    .some(x => held.has(x.id) || dependents.has(x.id));
  const moved = new Set(), settled = new Set(), report = [];
  const intent = k => `${moved.has(k.id) ? 'MoveToScope' : verbsOf(k, want(k))[0].kind} ${where(k)}`;
  const skip = (k, code) => { settled.add(k.id); report.push({ key: k.id, code, intent: intent(k) }); };
  const retiredScopes = [];
  const actorStands = at => {
    if (retiredScopes.some(s => under(at.scope, s))) return false;
    const k = index.get(target('actor', at).id);
    return k ? now.get(k.id) !== undefined : valueAt(graph, target('actor', at)) !== undefined;
  };

  const moves = new Map(), actors = keys.filter(k => k.kind === 'actor'), pairs = new Map();
  for (const back of actors.filter(k => want(k) !== undefined)) {
    const from = actors.find(k => want(k) === undefined && !moved.has(k.id) && k.value.local === back.value.local
      && key(k.value.scope) !== key(back.value.scope));
    if (!from) continue;
    moved.add(from.id); moved.add(back.id);
    pairs.set(back.id, from);
  }
  const scopeStands = scope => {
    const k = index.get(target('scope', scope).id);
    return key(scope) === key(graph.anchor?.scope ?? []) || (k ? now.get(k.id) !== undefined : graph.scopes.some(s => s.id === key(scope)));
  };
  const moveBack = (back, from) => {
    const here = now.get(back.id) !== undefined, there = now.get(from.id) !== undefined;
    if (here && !there) return;
    if (!there || here || !scopeStands(back.value.scope)) {
      skip(back, here ? 'UNDO_KEY_EDITED' : 'UNDO_TARGET_GONE');
      for (const k of actorKeys(back.value)) settled.add(k.id);
      return;
    }
    const group = key([from.value.scope, back.value.scope]);
    if (!moves.has(group)) moves.set(group, { kind: 'MoveToScope', actors: [], target: absolute(back.value.scope) });
    moves.get(group).actors.push(absolute(from.value));
    actorKeys(from.value).forEach((there, i) => {
      const here = actorKeys(back.value)[i];
      now.set(here.id, now.get(there.id)); now.set(there.id, undefined);
      if (held.has(there.id)) held.add(here.id);
    });
  };

  const upserts = [], retires = [], scopesAgain = [], actorsAgain = new Set();
  const hold = (list, k, flags, value = want(k)) => list.push(...verbsOf(k, value, flags).map(command => ({ key: k, command })));
  const structure = keys.filter(k => (k.kind === 'actor' && (!moved.has(k.id) || pairs.has(k.id))) || k.kind === 'scope')
    .sort((a, b) => depth(a) - depth(b) || (a.kind === 'actor' ? -1 : 1) - (b.kind === 'actor' ? -1 : 1));
  for (const k of structure) {
    if (pairs.has(k.id)) { moveBack(k, pairs.get(k.id)); continue; }
    if (same(authored(k, now.get(k.id)), authored(k, want(k)))) continue;
    if (held.has(k.id)) { skip(k, 'UNDO_KEY_EDITED'); continue; }
    if (k.kind === 'scope') {
      if (want(k) === undefined) {
        if (kept(k)) { skip(k, 'UNDO_KEY_EDITED'); continue; }
        hold(retires, k);
        retiredScopes.push(k.value);
      } else {
        const container = k.value.at(-1), parent = k.value.slice(0, -1);
        if (container?.name !== undefined && parent.length >= (graph.anchor?.scope ?? []).length
          && !actorStands({ scope: parent, local: container.name })) { skip(k, 'UNDO_TARGET_GONE'); continue; }
        if (now.get(k.id) === undefined) scopesAgain.push(k);
        hold(upserts, k, undefined, authored(k, want(k)));
      }
      now.set(k.id, want(k));
      continue;
    }
    if (want(k) === undefined) {
      if (kept(k)) { skip(k, 'UNDO_KEY_EDITED'); continue; }
      hold(retires, k);
      for (const gone of actorKeys(k.value)) if (index.has(gone.id)) now.set(gone.id, undefined);
      continue;
    }
    const flags = index.get(target('flags', k.value).id);
    const declared = flags && !held.has(flags.id) && want(flags) !== undefined ? want(flags)
      : now.get(flags?.id) ?? LENS.flags.view(graph.declared, k.value);
    hold(upserts, k, declared);
    const presentation = index.get(target('presentation', { actor: k.value }).id);
    if (presentation && now.get(k.id) === undefined) now.set(presentation.id, null);
    if (now.get(k.id) === undefined) actorsAgain.add(k.id);
    now.set(k.id, want(k));
    if (flags) now.set(flags.id, declared);
  }

  const renamed = new Map();
  for (const k of scopesAgain) for (const b of bindings(want(k))) {
    const answer = actorsAgain.has(target('actor', b.inner.actor).id) ? ports.get(key(b.inner.actor))?.[b.side] : undefined;
    const fresh = answer?.length === 1 ? answer[0] : undefined;
    renamed.set(endpoint(b.inner.actor, b.inner.port), fresh);
    renamed.set(endpoint({ scope: k.value.slice(0, -1), local: k.value.at(-1).name }, b.outer), b.outer === b.inner.port ? fresh : undefined);
  }
  const port = end => {
    const id = endpoint(end.actor, end.port);
    if (!renamed.has(id)) return end;
    return renamed.get(id) === undefined ? undefined : { ...end, port: renamed.get(id) };
  };
  const redeclared = k => {
    if (k.kind === 'edge') {
      const from = port(k.value.from), to = port(k.value.to);
      return from && to && { at: { ...k.value, from, to }, value: want(k) };
    }
    if (k.kind !== 'mount') return { at: k.value, value: want(k) };
    let whole = true;
    const roles = Object.fromEntries(Object.entries(want(k).roles ?? {}).map(([role, binding]) => {
      if (binding == null) return [role, binding];
      const at = port(binding);
      whole &&= at !== undefined;
      return [role, at];
    }));
    return whole ? { at: k.value, value: { ...want(k), roles } } : undefined;
  };

  const standsOn = k => k.kind === 'edge' ? [k.value.from.actor, k.value.to.actor]
    : k.kind === 'mount' ? Object.values(want(k).roles ?? {}).filter(Boolean).map(binding => binding.actor) : [];
  const early = [], late = [];
  for (const k of keys.filter(k => ['edge', 'mount', 'annotation', 'template'].includes(k.kind))) {
    if (same(now.get(k.id), want(k))) continue;
    if (held.has(k.id)) { skip(k, 'UNDO_KEY_EDITED'); continue; }
    if (want(k) === undefined) {
      if (kept(k)) { skip(k, 'UNDO_KEY_EDITED'); continue; }
      early.push(...verbsOf(k, want(k))); now.set(k.id, undefined); continue;
    }
    const spelled = standsOn(k).every(actorStands) && redeclared(k);
    if (!spelled) { skip(k, 'UNDO_TARGET_GONE'); continue; }
    late.push(...LENS[k.kind].verbs(spelled.at, spelled.value));
    now.set(k.id, want(k));
  }

  const sets = [];
  for (const k of keys.filter(k => k.kind === 'flags' || k.kind === 'presentation')) {
    if (settled.has(k.id) || want(k) === undefined || same(now.get(k.id), want(k))) continue;
    if (held.has(k.id)) { skip(k, 'UNDO_KEY_EDITED'); continue; }
    const at = k.kind === 'presentation' ? k.value.actor ?? k.value.annotation : k.value;
    const annotation = target('annotation', at);
    const exists = k.kind === 'presentation' && k.value.annotation
      ? !retiredScopes.some(s => under(at.scope, s)) && (index.has(annotation.id)
        ? now.get(annotation.id) !== undefined : valueAt(graph, annotation) !== undefined)
      : actorStands(at);
    if (!exists) { skip(k, 'UNDO_TARGET_GONE'); continue; }
    sets.push(...verbsOf(k, want(k)));
    now.set(k.id, want(k));
  }

  const outerFirst = (a, b) => depth(a.key) - depth(b.key) || (a.key.kind === 'actor' ? -1 : 1) - (b.key.kind === 'actor' ? -1 : 1);
  const innerFirst = (a, b) => depth(b.key) - depth(a.key) || (a.key.kind === 'scope' ? -1 : 1) - (b.key.kind === 'scope' ? -1 : 1);
  return { report, commands: [...early, ...upserts.sort(outerFirst).map(u => u.command), ...moves.values(),
    ...retires.sort(innerFirst).map(r => r.command), ...late.filter(c => c.kind === 'UpsertAnnotation'), ...sets, ...late.filter(c => c.kind !== 'UpsertAnnotation')] };
}

function standsOn(k) {
  if (k.kind === 'presentation' && k.value.annotation) return [target('annotation', k.value.annotation)];
  if (k.kind === 'edge') return [target('actor', k.value.from.actor), target('actor', k.value.to.actor)];
  const scope = k.kind === 'scope' ? k.value.slice(0, -1) : k.kind === 'presentation' ? (k.value.actor ?? k.value.annotation).scope : ACTOR_KINDS.includes(k.kind) ? k.value.scope : [];
  return scope.flatMap((segment, i) => [target('scope', scope.slice(0, i + 1)),
    ...(segment.name === undefined ? [] : [target('actor', { scope: scope.slice(0, i), local: segment.name })])]);
}

export function undoHistory() {
  const stacks = { undo: [], redo: [] }, watched = new Set();
  const slot = entry => ({ entry, edited: new Set(), dependents: new Set() });
  return {
    top: from => stacks[from].at(-1),
    get size() { return { undo: stacks.undo.length, redo: stacks.redo.length }; },
    watch(entry) { const s = slot(entry); watched.add(s); return s; },
    unwatch(s) { watched.delete(s); },
    observed(keys) {
      const named = new Set(keys.map(k => k.id)), built = new Set(keys.flatMap(standsOn).map(k => k.id));
      for (const s of [...stacks.undo, ...stacks.redo, ...watched])
        for (const k of s.entry.touchedKeys) {
          if (named.has(k.id)) s.edited.add(k.id);
          if (built.has(k.id)) s.dependents.add(k.id);
        }
    },
    edited(s) { stacks.undo.push(s); stacks.redo.length = 0; },
    moved(from, s) { stacks[from].pop(); stacks[from === 'undo' ? 'redo' : 'undo'].push(s); },
    spent(from) { stacks[from].pop(); },
  };
}
