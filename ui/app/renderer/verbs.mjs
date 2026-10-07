import { edgeKeyFromDeclaration } from '@circular/protocol/declaration';
import { identity as key } from './query.mjs';

const put = (scene, field, at, entry) => ({ ...scene, [field]: new Map(scene[field]).set(at, entry) });
const drop = (scene, field, at) => {
  if (!scene[field].has(at)) return scene;
  const next = new Map(scene[field]);
  next.delete(at);
  return { ...scene, [field]: next };
};
export const absolute = value => ({ arm: 'absolute', value });
const inside = (root, scope) => [...root, ...scope];
const actorAt = (root, actor) => ({ ...actor, scope: inside(root, actor.scope) });
const endpointAt = (root, endpoint) => ({ ...endpoint, actor: actorAt(root, endpoint.actor) });
const ownerPart = owner => owner.actor ?? owner.annotation;
const ownerCollection = owner => owner.actor ? 'actors' : 'annotations';
const ownerAt = (root, owner) => Object.fromEntries(Object.entries(owner).map(([kind, at]) => [kind, actorAt(root, at)]));
const onActor = (root, at, value) => [actorAt(root, at), value];

export const ACTOR_KINDS = ['actor', 'flags', 'presentation'];
const actorPart = ({ actorType, config }) => ({ actorType, config });
const UNDECLARED = { collapsed: false };
const presentationOf = p => {
  const value = Object.fromEntries(Object.entries({ ...UNDECLARED, ...p }).filter(([, v]) => v !== undefined && v !== null));
  return Object.keys(value).length === 1 && value.collapsed === false ? null : value;
};

const partial = row => ({ ...row,
  set: (scene, at, value) => value === undefined ? scene : row.set(scene, at, value),
  verbs: (at, value) => value === undefined ? [] : row.verbs(at, value) });
const declared = (collection, field, upsert, retire, lower = onActor) => ({
  view: (scene, at) => scene[collection].get(key(at))?.declaration,
  set: (scene, at, declaration) => declaration === undefined ? drop(scene, collection, key(at))
    : put(scene, collection, key(at), { ...scene[collection].get(key(at)), address: at, declaration }),
  verbs: (at, declaration) => [declaration === undefined ? { kind: retire, [field]: absolute(at) }
    : { kind: upsert, [field]: absolute(at), declaration }],
  lower,
});

export const LENS = {
  actor: {
    view: (scene, at) => { const d = scene.actors.get(key(at))?.declaration; return d && actorPart(d); },
    set: (scene, at, value) => {
      if (value === undefined) return drop(scene, 'actors', key(at));
      const entry = scene.actors.get(key(at));
      return put(scene, 'actors', key(at), { address: at, declaration: { ...entry?.declaration, ...value },
        presentation: entry?.presentation ?? {} });
    },
    verbs: (at, value, flags) => [value === undefined ? { kind: 'RetireActor', actor: absolute(at) }
      : { kind: 'UpsertActor', actor: absolute(at), declaration: { ...value, flags } }],
    lower: onActor,
  },
  flags: partial({
    view: (scene, at) => scene.actors.get(key(at))?.declaration?.flags,
    set: (scene, at, flags) => {
      const entry = scene.actors.get(key(at));
      return entry?.declaration ? put(scene, 'actors', key(at), { ...entry, declaration: { ...entry.declaration, flags } }) : scene;
    },
    verbs: (at, flags) => [{ kind: 'SetFlags', actor: absolute(at), flags }],
    lower: onActor,
  }),
  presentation: partial({
    view: (scene, owner) => { const entry = scene[ownerCollection(owner)].get(key(ownerPart(owner))); return entry?.declaration ? presentationOf(entry.presentation) : undefined; },
    set: (scene, owner, presentation) => { const field = ownerCollection(owner), at = ownerPart(owner); return put(scene, field, key(at), { address: at, ...scene[field].get(key(at)), presentation }); },
    verbs: (at, presentation) => [{ kind: 'SetPresentation', owner: Object.fromEntries(Object.entries(at).map(([kind, address]) => [kind, absolute(address)])), presentation: presentation ?? UNDECLARED }],
    lower: (root, at, p) => {
      const target = p.anchor?.target;
      return [ownerAt(root, at), target ? { ...p, anchor: { ...p.anchor, target: actorAt(root, target) } } : p];
    },
  }),
  edge: {
    view: (scene, at) => scene.edges.get(key(at))?.declaration.attrs,
    set: (scene, at, attrs) => attrs === undefined ? drop(scene, 'edges', key(at))
      : put(scene, 'edges', key(at), { address: at, declaration: { ...at, attrs } }),
    verbs: (at, attrs) => [attrs === undefined ? { kind: 'RetireEdge', edge: absolute(at) }
      : { kind: 'UpsertEdge', edge: absolute(at), declaration: { ...at, attrs } }],
    lower: (root, at, attrs) => [{ ...at, from: endpointAt(root, at.from), to: endpointAt(root, at.to) }, attrs],
  },
  scope: declared('scopes', 'scope', 'UpsertScope', 'RetireScope', (root, at, d) => [inside(root, at), d]),
  mount: declared('mounts', 'mount', 'UpsertExportMount', 'RetireExportMount', (root, at, d) => [actorAt(root, at),
    { ...d, roles: Object.fromEntries(Object.entries(d.roles).map(([role, binding]) => [role, binding == null ? binding : endpointAt(root, binding)])) }]),
  annotation: declared('annotations', 'annotation', 'UpsertAnnotation', 'RetireAnnotation'),
  template: {
    view: (scene, name) => scene.templates.get(key(name))?.declaration,
    set: (scene, name, commands) => commands === undefined ? drop(scene, 'templates', key(name))
      : put(scene, 'templates', key(name), { address: name, declaration: commands }),
    verbs: (name, commands) => [commands === undefined ? { kind: 'RetireTemplate', name }
      : { kind: 'UpsertTemplate', name, commands }],
    lower: (_root, name, commands) => [name, commands],
  },
};

export const VERBS = {
  UpsertActor: c => [['actor', c.actor.value, actorPart(c.declaration)], ['flags', c.actor.value, c.declaration.flags]],
  RetireActor: c => ACTOR_KINDS.map(kind => [kind, kind === 'presentation' ? { actor: c.actor.value } : c.actor.value, undefined]),
  SetFlags: c => [['flags', c.actor.value, c.flags]],
  SetPresentation: c => [['presentation', Object.fromEntries(Object.entries(c.owner).map(([kind, address]) => [kind, address.value])), c.presentation]],
  UpsertEdge: c => [['edge', edgeKeyFromDeclaration(c.declaration), c.declaration.attrs]],
  RetireEdge: c => [['edge', c.edge.value, undefined]],
  UpsertScope: c => [['scope', c.scope.value, c.declaration]],
  RetireScope: c => [['scope', c.scope.value, undefined]],
  UpsertExportMount: c => [['mount', c.mount.value, c.declaration]],
  RetireExportMount: c => [['mount', c.mount.value, undefined]],
  UpsertAnnotation: c => [['annotation', c.annotation.value, c.declaration]],
  RetireAnnotation: c => [['annotation', c.annotation.value, undefined], ['presentation', { annotation: c.annotation.value }, undefined]],
  UpsertTemplate: c => [['template', c.name, c.commands]],
  RetireTemplate: c => [['template', c.name, undefined]],
};

const MOVE = {
  keys: c => c.actors.flatMap(({ value }) => [value, { ...value, scope: c.target.value }]
    .flatMap(at => ACTOR_KINDS.map(kind => [kind, kind === 'presentation' ? { actor: at } : at]))),
  at: c => c.target.value,
};
const lensVerb = spell => ({
  keys: spell,
  at: c => { const [kind, at] = spell(c)[0] ?? []; return kind === 'presentation' ? ownerPart(at) : at; },
  fold: (scene, c, root) => spell(c).reduce((next, [kind, at, value]) =>
    LENS[kind].set(next, ...(root ? LENS[kind].lower(root, at, value) : [at, value])), scene),
});
/**
 * The one reading of a declaration verb the canvas knows: `{keys, at, fold}` — the keys it names
 * (`[kind, address, value]`), the address it names as a whole, and its fold. Every verb is its line
 * of VERBS read through LENS, except MoveToScope, which is MOVE and has no fold. A kind that is no
 * declaration verb has no reading (undefined): the fold refuses it, and it names no key.
 */
export const verbOf = kind => kind === 'MoveToScope' ? MOVE
  : Object.hasOwn(VERBS, kind) ? lensVerb(VERBS[kind]) : undefined;
export const keysOf = command => verbOf(command.kind)?.keys(command) ?? [];
