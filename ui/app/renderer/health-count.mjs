import { actorIdentityFromValue, scopeIdentityValue } from '@circular/protocol/establishment';
import { healthState, lifeLabel, NOT_STANDING, EMPTY } from './reasons.mjs';
import { identity, actorKey } from './query.mjs';
import { runPause } from './scene.mjs';

const under = (scope, root) => root.length <= scope.length
  && root.every((segment, n) => identity(segment) === identity(scope[n]));

export function healthSet(graph, root = []) {
  const counts = { alive: 0, dead: 0, [NOT_STANDING]: 0, unobserved: 0 };
  const members = new Set(), declared = new Set();
  if (root === null) return null;
  for (const node of graph?.nodes ?? []) {
    if (!under(node.address.scope, root) || members.has(node.id)) continue;
    members.add(node.id); declared.add(node.id);
    counts[healthState(node.health?.state)]++;
  }
  for (const row of graph?.healthPage?.items ?? []) {
    const id = actorKey(row.actor);
    if (members.has(id)) continue;
    const address = actorIdentityFromValue(row.actor), at = address.scope.findIndex(segment => segment.of !== undefined);
    if (at < 0 || !declared.has(identity({ scope: address.scope.slice(0, at), local: address.scope[at].of }))) continue;
    const live = graph.instances?.tables?.get(identity(scopeIdentityValue([...address.scope.slice(0, at), { name: address.scope[at].of }])));
    if (!live?.some(key => identity(key) === identity(address.scope[at].key))) continue;
    members.add(id);
    counts[healthState(row.state)]++;
  }
  return { counts, members };
}

export const countsText = (counts, word = state => state) =>
  ['alive', 'dead', ...[NOT_STANDING, 'unobserved'].filter(state => counts[state])]
    .map(state => `${word(state)} ${counts[state]}`).join(' · ');
const setState = counts => !Object.values(counts).some(Boolean) ? EMPTY
  : counts.dead ? 'dead' : counts[NOT_STANDING] ? NOT_STANDING : counts.unobserved ? 'unobserved' : 'alive';
export function healthReading(set, page, code) {
  const held = set && page ? set.counts : null, last = held ? setState(held) : null;
  const pause = code == null ? runPause(page) : null;
  return { state: code != null ? 'unobserved' : pause ? 'paused' : last ?? 'unobserved', held, last, pause };
}
export const heldText = ({ held, last }) => !held ? null : last === EMPTY ? lifeLabel(EMPTY) : countsText(held, lifeLabel);
export const lastObservation = seen => `last observation ${seen ?? 'unrecorded'}`;
export const lastObservedText = (reading, seen) => !reading.held ? null : `${lastObservation(seen)} · ${heldText(reading)}`;
