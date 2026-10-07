import { editDaemon } from './edit-peer.mjs';
import { fixture, catalogRow, healthPage, uint } from './fixtures.mjs';
import { reconstructiveCommandFromValue } from '@circular/protocol/declaration';
import { views } from '../renderer/views.mjs';

const flags = { bypass: false, mute: false, pause: false };
const address = local => [3n, { local, scope: [] }];
const absolute = local => ({ local, scope: [[1n, 'desk']] });
export const actorName = i => `a${String(i).padStart(2, '0')}`;

export function flowDaemon({ count = 33, columns = 7 } = {}) {
  const f = fixture(), kinds = views.kinds();
  const kindOf = i => kinds[i % kinds.length];
  const typeOf = i => `flow_${kindOf(i)}`;
  const commands = [];
  for (let i = 0; i < count; i++) {
    commands.push(reconstructiveCommandFromValue({ kind: 'UpsertActor', actor: address(actorName(i)),
      declaration: { domain: { actor_type: typeOf(i), config: null }, flags } }));
    commands.push(reconstructiveCommandFromValue({ kind: 'SetPresentation', owner: { actor: address(actorName(i)) },
      presentation: { collapsed: false, label: actorName(i), fixed: { x: BigInt(40 + (i % columns) * 320), y: BigInt(40 + Math.floor(i / columns) * 420) },
        view: { kind: kindOf(i), config: null } } }));
  }
  for (let i = 0; i < count; i++) {
    const from = actorName(i), to = actorName((i + 1) % count);
    commands.push(reconstructiveCommandFromValue({ kind: 'UpsertEdge',
      edge: [3n, [1n, { actor: { scope: [], local: from }, port: 'event' }, { actor: { scope: [], local: to }, port: 'input' }, 0n]],
      declaration: { from: [{ scope: [], local: from }, 'event'], to: [{ scope: [], local: to }, 'input'], ordinal: 0n,
        attrs: { delay: { num: 0n, den: 1n }, policy: { delivery: 2n }, preprocess: [] } } }));
  }
  f.snapshot.value.commands = commands;
  const names = Array.from({ length: count }, (_, i) => actorName(i));
  f.ports = { status: 'accepted', value: { items: names.map(local => ({ actor: [1n, absolute(local)],
    in_ports: [{ id: 'input', flow: [1n, [1n, [1n]]], label: null }], out_ports: [{ id: 'event', flow: [1n, [1n, [1n]]], label: null }] })) } };
  const types = [...new Set(names.map((_, i) => typeOf(i)))];
  f.catalog.value.items = types.map(type => catalogRow(type, 1n, { description: `Declared ${type} actor.` }));
  f.createInputs = { anchor: types, items: types.map(actor_type => ({ actor_type, state: [1n] })) };
  f.health = healthPage(names.map(local => ({ actor: absolute(local), state: 'running' })));
  f.events = { anchor: null, items: [] };
  f.queries = Object.fromEntries(['authoring.actor-access', 'timeline.bins', 'timeline.at'].map(name =>
    [name, { rejected: { code: 22n, message: 'This witness reads declarations and arrivals' } }]));
  const daemon = editDaemon({ fixtureValue: f, quietFeeds: true });
  const received = new Map(), emitted = new Map();
  const row = (to, at, body) => {
    const from = (to + count - 1) % count, index = received.get(to) ?? 0n, sequence = emitted.get(from) ?? 0n;
    received.set(to, index + 1n); emitted.set(from, sequence + 1n);
    return { kind: 'actor_arrival', actor: absolute(actorName(to)), port: 'input', index, observed_at_ms: BigInt(at),
      body, origin: [uint(at), uint(0n), absolute(actorName(from)), uint(sequence), uint(1n)],
      edge: [1n, { actor: absolute(actorName(from)), port: 'event' }, { actor: absolute(actorName(to)), port: 'input' }, 0n] };
  };
  return { ...daemon, count, arrive: (to, at, body = { value: at % 97, text: `line ${at}` }) => daemon.feed('actor.events', row(to, at, body)) };
}
