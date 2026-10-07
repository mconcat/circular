import { actor, stamp, wireKey } from './card-scene.mjs';
import { catalogRow } from './fixtures.mjs';

const url = 'http://127.0.0.1:9090/api/v1/query?query=sum';
export const PREDICATE = 'event.failed_per_minute > 0.5';
const kinds = {
  poll: ['timer', { every: 15000n }],
  orders: ['request', { method: 'get', url }],
  failing: ['alert', { predicate: PREDICATE, firing_delay: 10000n, recovery_delay: 120000n }],
  triage: ['agent', { harness: 'claude', queue_capacity: 4n, result: 'bytes' }],
  oncall: ['notify', { channel: 'oncall', minimum_interval: 0n, during_interval: 'queue' }],
  act: ['tool_executor', { tools: { page: { effect: 'spawn', program: '/t/bin/page', arguments: [] },
    flag_off: { effect: 'spawn', program: '/t/bin/flag-off', arguments: ['x', '-'], approval: 'required' } } }],
  settle: ['debounce', { quiet_window: 90000n }],
  recheck: ['request', { method: 'get', url }],
  verdict: ['tap', {}],
};
const ports = {
  timer: [[['bang', true]], [['tick', true], ['_error', false]]],
  request: [[['event', true]], [['response', true], ['_error', false]]],
  alert: [[['event', true]], [['event', true], ['transition', false]]],
  agent: [[['turn', true], ['tool_result', false]], [['record', false], ['tool_request', false], ['result', true], ['_error', false]]],
  notify: [[['notification', true]], [['_error', false]]],
  tool_executor: [[['call', true]], [['result', true], ['_error', false]]],
  debounce: [[['event', true]], [['event', true]]],
  tap: [[['event', true]], [['event', true]]],
};
export const portRows = list => list.map(([id, primary]) => ({ id, primary }));
const ORDER = ['act', 'poll', 'oncall', 'orders', 'settle', 'triage', 'failing', 'recheck', 'verdict'];
export const actors = ORDER.map(local => {
  const [type, config] = kinds[local];
  return { local, type, config, in: ports[type][0].map(([id]) => id), out: ports[type][1].map(([id]) => id) };
});
export const catalog = Object.entries(ports).map(([type, [ins, outs]]) =>
  catalogRow(type, 1n, { in_ports: portRows(ins), out_ports: portRows(outs) }));
const parse = [{ kind: 'parse', config: { arguments: {}, decoder: 'json', field: 'body' } }, { kind: 'flatten', config: { at: ['data', 'result'] } }];
const map = transform => ({ kind: 'map', config: { transform } });
const filter = predicate => ({ kind: 'filter', config: { predicate } });
const firing = filter("event.from == 'Ok' && event.to == 'Firing'");
export const wires = [
  { from: 'poll', out: 'tick', to: 'orders', in: 'event' },
  { from: 'orders', out: 'response', to: 'failing', in: 'event', preprocess: [...parse, map("{'failed_per_minute': double(event.data.result.value[1])}")] },
  { from: 'failing', out: 'transition', to: 'triage', in: 'turn', preprocess: [firing, map("'The Astronomy Shop is failing orders.'")] },
  { from: 'triage', out: 'result', to: 'oncall', in: 'notification', preprocess: [map("{'title': 'Astronomy Shop: orders failing', 'body': string(event)}")] },
  { from: 'failing', out: 'transition', to: 'act', in: 'call', preprocess: [firing, map("{'id': b'rollback', 'tool': 'flag_off', 'arguments': b''}")] },
  { from: 'act', out: 'result', to: 'settle', in: 'event' },
  { from: 'settle', out: 'event', to: 'recheck', in: 'event' },
  { from: 'recheck', out: 'response', to: 'verdict', in: 'event', preprocess: [...parse, map("{'resolved': true}")] },
  { from: 'verdict', out: 'event', to: 'oncall', in: 'notification', preprocess: [filter('event.resolved'), map("{'title': 'recovered'}")] },
  { from: 'verdict', out: 'event', to: 'act', in: 'call', preprocess: [filter('!event.resolved'), map("{'tool': 'page'}")] },
];
export const NOTE = '1. Open the flagd config or UI (`demo.flagd.json` / `/feature`) and look for checkout-path flags that are on: '
  + '`paymentFailure`, `paymentUnreachable`, `cartFailure`, `productCatalogFailure`, `kafkaQueueProblems`.\n'
  + '2. In Jaeger, find a failed checkout trace and read the span that errors.';
const transition = { from: 'Ok', to: 'Firing' };
const emitted = (local, port, index, ms, sequence, body) => ({ kind: 'actor_emission', actor: actor(local), port, index, body,
  observed_at_ms: ms, at: stamp(local, ms, sequence) });
export const rows = [
  ...[0n, 1n, 2n].map(i => emitted('poll', 'tick', i + 1n, 1000n + i * 15000n, i, { sequence: i })),
  emitted('failing', 'transition', 3n, 40000n, 3n, transition),
  emitted('triage', 'result', 1n, 52000n, 1n, NOTE),
  ...[0n, 1n, 2n].map(i => ({ kind: 'actor_arrival', actor: actor('orders'), port: 'event', index: i, body: { sequence: i },
    observed_at_ms: 1000n + i * 15000n, origin: stamp('poll', 1000n + i * 15000n, i), edge: wireKey(wires[0]) })),
  { kind: 'actor_arrival', actor: actor('triage'), port: 'turn', index: 0n, body: transition, observed_at_ms: 40000n,
    origin: stamp('failing', 40000n, 3n), edge: wireKey(wires[2]) },
  { kind: 'actor_arrival', actor: actor('act'), port: 'call', index: 0n, body: transition, observed_at_ms: 40000n,
    origin: stamp('failing', 40000n, 3n), edge: wireKey(wires[4]) },
  { kind: 'actor_arrival', actor: actor('oncall'), port: 'notification', index: 0n, body: NOTE, observed_at_ms: 52000n,
    origin: stamp('triage', 52000n, 1n), edge: wireKey(wires[3]) },
];

export const CYCLE = ['act', 'settle', 'recheck', 'verdict'];
