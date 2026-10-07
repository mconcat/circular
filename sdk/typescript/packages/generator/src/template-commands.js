import { compareSourceNames } from './source-order.js';
import { declarationRow } from '@circular/protocol/internal/declaration-rows';

const order = ['UpsertActor', 'UpsertEdge', 'SetPresentation', 'UpsertExportMount', 'UpsertAnnotation'];
const authorable = new Set([...order, 'SetFlags']);

/** Fresh declaration state: retain final replacements, fold flags, then print in stable identity order. */
export function compactTemplateCommands(commands, relative = true) {
  const groups = new Map(order.map(kind => [kind, new Map()]));
  const identity = address => JSON.stringify(address.actor || address.annotation ? Object.fromEntries(Object.entries(address).map(([kind, address]) => [kind, address.value])) : address.value, (_key, value) => value && typeof value === 'object' && !Array.isArray(value)
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, value[key]])) : value);
  for (let command of commands) {
    const addressField = authorable.has(command.kind) && declarationRow(command.kind).field;
    if (!addressField) throw new TypeError(`authoring.template.unsupported-command: ${command.kind}`);
    const owner = command[addressField];
    const address = command.kind === "SetPresentation" ? owner.actor ?? owner.annotation : owner;
    if (relative && (command.kind === 'UpsertEdge'
      ? command.declaration.from.actor.scope.length || command.declaration.to.actor.scope.length
      : address.value.scope.length)) throw new TypeError('authoring.template.nonrelative-command');
    const adjusted = { ...address, ...(relative ? { arm: 'relative' } : {}) };
    command = { ...command, [addressField]: command.kind === "SetPresentation" ? { [owner.actor ? "actor" : "annotation"]: adjusted } : adjusted };
    if (command.kind === 'UpsertExportMount' && Object.values(command.declaration.roles).some(e => e.actor.scope.length)) {
      throw new TypeError('authoring.prepass.runtime-descendant-not-authorable');
    }
    const id = identity(owner);
    if (command.kind === 'SetFlags') {
      const actor = groups.get('UpsertActor').get(id);
      if (!actor) throw new TypeError('authoring.template.flags-without-actor');
      groups.get('UpsertActor').set(id, { ...actor, declaration: { ...actor.declaration, flags: command.flags } });
    } else groups.get(command.kind).set(id, command);
  }
  if (relative) {
    const localActor = actor => groups.get('UpsertActor').has(identity({ value: actor }));
    for (const edge of groups.get('UpsertEdge').values()) {
      if (!localActor(edge.declaration.from.actor) || !localActor(edge.declaration.to.actor)) throw new TypeError('authoring.template.foreign-reference');
    }
    for (const mount of groups.get('UpsertExportMount').values()) {
      if (Object.values(mount.declaration.roles).some(endpoint => !localActor(endpoint.actor))) throw new TypeError('authoring.template.foreign-reference');
    }
  }
  return order.flatMap(kind => [...groups.get(kind).values()].sort((a, b) => {
    const aKey = a[declarationRow(kind).field].value, bKey = b[declarationRow(kind).field].value;
    return kind === 'UpsertActor' ? compareSourceNames(aKey.local, bKey.local)
      : compareSourceNames(identity(a[declarationRow(kind).field]), identity(b[declarationRow(kind).field]));
  }));
}
