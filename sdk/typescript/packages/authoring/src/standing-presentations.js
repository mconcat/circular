/**
 * The presentation each actor holds in the fold, as one program's verbs leave it.
 *
 * A program means only the presentation axes it says; the runtime lays them over the owner's value
 * here, as a canvas gesture lays its change over the node's (ui/app/renderer/edit.mjs `present`).
 * The value comes from the complete snapshot the epoch is fenced by, which the caller has already read;
 * nothing is read again. Within the run it changes as the fold would: a SetPresentation sets it, a
 * RetireActor takes it away with the actor, and a RetireScope takes away every one below that scope
 * (fold.rs `retire_actor`, `retire_scope`).
 */
const unarmed = address => address && typeof address === 'object' && 'arm' in address && 'value' in address ? address.value : address;
const text = value => JSON.stringify(value, (_name, item) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.keys(item).sort().map(name => [name, item[name]])) : item);
const part = owner => owner.actor ?? owner.annotation;
const keyOf = owner => text([owner.actor ? "actor" : "annotation", unarmed(part(owner))]);
const below = (scope, ancestor) => ancestor.length <= scope.length && ancestor.every((segment, index) => text(segment) === text(scope[index]));

export function standingPresentations(snapshot, targetScope) {
  const values = new Map();
  const set = (actor, value) => values.set(keyOf(actor), { scope: unarmed(part(actor)).scope, value });
  const absolute = actor => ({ ...actor, scope: [...targetScope, ...actor.scope] });
  for (const command of snapshot?.commands ?? []) {
    if (command.kind !== 'SetPresentation') continue;
    const p = command.presentation;
    const anchor = p.anchor && typeof p.anchor === 'object' ? { ...p.anchor, target: absolute(unarmed(p.anchor.target)) } : p.anchor ?? null;
    set(Object.fromEntries(Object.entries(command.owner).map(([kind, address]) => [kind, absolute(unarmed(address))])), Object.freeze({ label: p.label ?? null, group: p.group ?? null, anchor,
      fixed: p.fixed ?? null, size: p.size ?? null, board: p.board ?? null, view: p.view ?? null, collapsed: p.collapsed }));
  }
  return Object.freeze({
    of: owner => values.get(keyOf(owner))?.value ?? null,
    observe(command) {
      if (command?.kind === 'SetPresentation') set(command.owner, command.presentation);
      else if (command?.kind === 'RetireActor') values.delete(keyOf({ actor: command.actor }));
      else if (command?.kind === 'RetireAnnotation') values.delete(keyOf({ annotation: command.annotation }));
      else if (command?.kind === 'RetireScope') {
        const scope = unarmed(command.scope);
        for (const [key, entry] of values) if (below(entry.scope, scope)) values.delete(key);
      }
    },
  });
}
