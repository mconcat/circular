import { reconstructiveCommandFromValue } from '@circular/protocol/declaration';
import { fixture, healthPage, uint, anyStream } from './fixtures.mjs';
import { editDaemon } from './edit-peer.mjs';
import { canvas } from './chrome-canvas.mjs';
import { search } from './harness-browser.mjs';

export const desk = [[1n, 'desk']];
export const actor = local => ({ local, scope: desk });
export const stamp = (local, ms, sequence = 0n) => [uint(ms), uint(0n), actor(local), uint(sequence), uint(1n)];
export const wireKey = ({ from, out, to, in: inlet }) => [1n, { actor: actor(from), port: out }, { actor: actor(to), port: inlet }, 0n];
export const reserved = local => [2n, actor(local)];

const relative = local => [3n, { local, scope: [] }];
const flags = { bypass: false, mute: false, pause: false };

export function sceneDaemon({ actors, wires = [], rows = [], catalog = [], queries = {} }) {
  const f = fixture();
  f.snapshot.value.commands = [
    ...actors.flatMap(({ local, type, config = {}, view, viewConfig = null, label = local, x, y, w = 244, h = 248 }) => [
      reconstructiveCommandFromValue({ kind: 'UpsertActor', actor: relative(local),
        declaration: { domain: { actor_type: type, config }, flags } }),
      ...(x === undefined ? [] : [reconstructiveCommandFromValue({ kind: 'SetPresentation', owner: { actor: relative(local) },
        presentation: { collapsed: false, label, fixed: { x: BigInt(x), y: BigInt(y) }, size: { w: BigInt(w), h: BigInt(h) },
          view: { kind: view, config: viewConfig } } })]),
    ]),
    ...wires.map(({ from, out, to, in: inlet, preprocess = [] }) => reconstructiveCommandFromValue({ kind: 'UpsertEdge',
      edge: [3n, [1n, { actor: { scope: [], local: from }, port: out }, { actor: { scope: [], local: to }, port: inlet }, 0n]],
      declaration: { from: [{ scope: [], local: from }, out], to: [{ scope: [], local: to }, inlet], ordinal: 0n,
        attrs: { delay: { num: 0n, den: 1n }, policy: { delivery: 2n }, preprocess } } })),
  ];
  f.ports.value.items = actors.map(({ local, in: ins = [], out = [], flows = {} }) => ({ actor: [1n, actor(local)],
    in_ports: ins.map(id => ({ id, flow: flows[id] ?? anyStream, label: null })),
    out_ports: out.map(id => ({ id, flow: flows[id] ?? anyStream, label: null })) }));
  f.catalog.value.items = catalog;
  f.health = healthPage();
  f.events = { anchor: null, items: rows };
  const unobserved = { rejected: { code: 22n, message: 'This witness does not observe it' } };
  f.queries = { 'timeline.bins': unobserved, 'timeline.at': unobserved, 'authoring.actor-access': { items: [] }, ...queries };
  return editDaemon({ fixtureValue: f, quietFeeds: true });
}

export async function onScene(scene, run, label = 'card scene') {
  const daemon = sceneDaemon(scene);
  await canvas(async (evaluate, call) => {
    await evaluate(`(() => { const a = StudyApp; a.state.zoom = 1; a.state.x = 24; a.state.y = 24; a.transformWorld(); })()`);
    const read = async (names, until = () => true) => {
      let cards;
      for (let turn = 0; turn < 150; turn++) {
        cards = await evaluate(`(() => Object.fromEntries([...document.querySelectorAll('#nodes > article.node')].map(card => {
          const viewer = card.querySelector('.node-viewer');
          return [card.querySelector('.node-title')?.textContent, viewer && { kind: viewer.dataset.viewer, text: viewer.innerText,
            reasons: [...viewer.querySelectorAll('[data-reason]')].map(el => [el.innerText, el.dataset.reason]),
            tier: document.querySelector('#canvas').dataset.tier }];
        })))()`);
        if (names.every(name => cards[name]) && until(cards)) return cards;
        await new Promise(resolve => setTimeout(resolve, 40));
      }
      return cards;
    };
    await run(read, evaluate, call);
  }, label, { daemon, daemonSearch: search() });
}
