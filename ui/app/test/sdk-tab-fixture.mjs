import { fixture, catalogRow, anyStream } from './fixtures.mjs';
import { createInputsAnswer, anyShapes } from './edit-peer.mjs';

export function programFixture(value = { count: 5n, bytes: new Uint8Array([0, 255]) }) {
  const f = fixture(), flags = { bypass: false, mute: false, pause: false };
  const key = local => ({ scope: [], local });
  const address = local => ({ arm: 'relative', value: key(local) });
  const from = { actor: key('source'), port: 'value' }, to = { actor: key('result'), port: 'event' };
  f.snapshot.value.anchor.scope = [];
  f.snapshot.value.commands = [
    { kind: 'UpsertActor', actor: address('source'), declaration: { actorType: 'json', config: { value }, flags } },
    { kind: 'UpsertActor', actor: address('result'), declaration: { actorType: 'tap', config: null, flags } },
    { kind: 'UpsertEdge', edge: { arm: 'relative', value: { from, to, ordinal: 0 } },
      declaration: { from, to, ordinal: 0, attrs: { delay: { num: 0n, den: 1n }, policy: { delivery: 'Lossless' }, preprocess: [] } } },
    { kind: 'UpsertExportMount', mount: address('reading'), declaration: { roles: { result: { actor: key('result'), port: 'event' } } } },
    { kind: 'UpsertAnnotation', annotation: address('explanation'), declaration: { kind: 'Note', refs: [key('result')], body: 'Read the values here.' } },
    { kind: 'SetPresentation', owner: { actor: address('result') }, presentation: { collapsed: false, label: 'Tap <label>', fixed: { x: 300n, y: 100n }, size: { w: 244n, h: 248n }, view: { kind: 'feed', config: null } } },
  ];
  f.catalog.value.items = [
    catalogRow('json', [2n, 'configuration frame'], { out_ports: [{ id: 'value', primary: true }] }),
    catalogRow('tap', 1n, { in_ports: [{ id: 'event', primary: true }], out_ports: [{ id: 'event', primary: true }] }),
  ];
  f.ports.value.items = f.catalog.value.items.map((row, i) => ({ actor: [1n, key(i ? 'result' : 'source')],
    in_ports: row.in_ports.map(p => ({ id: p.id, flow: anyStream, label: null })),
    out_ports: row.out_ports.map(p => ({ id: p.id, flow: anyStream, label: null })) }));
  f.health.items = []; f.events.items = [];
  f.createInputs = createInputsAnswer({ json: anyShapes({ value }), tap: null });
  return f;
}
