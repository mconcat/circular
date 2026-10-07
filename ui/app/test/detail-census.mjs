import { canvas } from './chrome-canvas.mjs';
import { editDaemon } from './edit-peer.mjs';
import { fixture, catalogRow, healthPage, createInputSlot, uint } from './fixtures.mjs';
import { reconstructiveCommandFromValue } from '@circular/protocol/declaration';
import { views } from '../renderer/views.mjs';

const flags = { bypass: false, mute: false, pause: false };
const address = local => [3n, { local, scope: [] }];
const absolute = local => ({ local, scope: [[1n, 'desk']] });
const description = type => `Declared ${type} actor: what it does with each arrival is written here as one whole sentence.`;
const sample = { predicate: 'event.error_rate > 0.25 && event.window_seconds >= 300', recovery_delay: 300000n, capacity: 64n,
  inactivity_timeout: 300000n, label: 'Boundary of the fleet', channel: 'operations-pager', minimum_interval: 300000n,
  path: '/var/log/fleet/gateway-activity.log', name: 'night-shift-operator', realm: 'operations.example' };
const local = kind => `k_${kind}`;

export function censusDaemon({ wired = false, rows = () => [] } = {}) {
  const f = fixture(), kinds = views.kinds(), entries = views.all();
  const typeOf = entry => entry.defaultFor[0] ?? `plain_${entry.kind}`;
  const commands = [];
  entries.forEach((entry, i) => {
    const config = Object.fromEntries((entry.reads ?? []).filter(key => key in sample).map(key => [key, sample[key]]));
    if (entry.kind === 'timing') config.quiet_window = 300000n;
    commands.push(reconstructiveCommandFromValue({ kind: 'UpsertActor', actor: address(local(entry.kind)),
      declaration: { domain: { actor_type: typeOf(entry), config }, flags } }));
    commands.push(reconstructiveCommandFromValue({ kind: 'SetPresentation', owner: { actor: address(local(entry.kind)) },
      presentation: { collapsed: false, label: entry.kind, fixed: { x: BigInt(40 + (i % 5) * 280), y: BigInt(40 + Math.floor(i / 5) * 400) },
        view: { kind: entry.kind, config: null } } }));
  });
  if (wired) kinds.forEach((kind, i) => {
    const from = local(kind), to = local(kinds[(i + 1) % kinds.length]);
    commands.push(reconstructiveCommandFromValue({ kind: 'UpsertEdge',
      edge: [3n, [1n, { actor: { scope: [], local: from }, port: 'event' }, { actor: { scope: [], local: to }, port: 'input' }, 0n]],
      declaration: { from: [{ scope: [], local: from }, 'event'], to: [{ scope: [], local: to }, 'input'], ordinal: 0n,
        attrs: { delay: { num: 0n, den: 1n }, policy: { delivery: 2n }, preprocess: [] } } }));
  });
  f.snapshot.value.commands = commands;
  f.ports = { status: 'accepted', value: { items: kinds.map(kind => ({ actor: [1n, absolute(local(kind))],
    in_ports: [{ id: 'input', flow: [1n, [1n, [1n]]], label: null }], out_ports: [{ id: 'event', flow: [1n, [1n, [1n]]], label: null }] })) } };
  f.catalog.value.items = entries.map(entry => catalogRow(typeOf(entry), 1n, { description: description(typeOf(entry)) }));
  const types = entries.map(typeOf);
  f.createInputs = { anchor: types, items: entries.map(entry => entry.kind !== 'timing' ? { actor_type: typeOf(entry), state: [1n] }
    : { actor_type: typeOf(entry), state: [2n, { relations: [], slots: [createInputSlot({ path: [[1n, 'quiet_window']], requirement: [1n],
      shape: [2n, 'int'], constraint: [1n, 'milliseconds'] })] }, {}, [[[1n, 'quiet_window']]]] }) };
  f.health = healthPage();
  f.events = { anchor: null, items: rows({ kinds, local, absolute, uint }) };
  f.queries = Object.fromEntries(['authoring.actor-access', 'timeline.bins', 'timeline.at'].map(name =>
    [name, { rejected: { code: 22n, message: 'This census reads declarations and one page' } }]));
  return editDaemon({ fixtureValue: f });
}

export const measure = `(() => {
  const canvas = document.getElementById('canvas');
  return { tier: canvas.getAttribute('data-tier'), cards: [...document.querySelectorAll('#nodes > article.node')].map(card => {
    const viewer = card.querySelector('.node-viewer');
    if (!viewer) return { id: card.id, name: card.querySelector('.node-title')?.textContent, lines: [] };
    const frame = viewer.getBoundingClientRect(), cardBox = card.getBoundingClientRect();
    const lines = [...viewer.querySelectorAll('*')].filter(el => [...el.childNodes].some(n => n.nodeType === 3 && n.nodeValue.trim())
      && el.getClientRects().length && getComputedStyle(el).visibility !== 'hidden').map(el => {
      const style = getComputedStyle(el), box = el.getBoundingClientRect();
      const range = document.createRange(); range.selectNodeContents(el);
      const text = range.getBoundingClientRect();
      // Every ancestor (and itself) that clips or scrolls: what stands outside its box is not in sight.
      let clip = el, hidden = 0;
      for (; clip && clip !== card.parentElement; clip = clip.parentElement) {
        const s = getComputedStyle(clip);
        if (s.overflowX === 'visible' && s.overflowY === 'visible') continue;
        const c = clip.getBoundingClientRect();
        hidden = Math.max(hidden, text.right - c.right, text.bottom - c.bottom);
      }
      return { where: [...new Set([el.parentElement === viewer ? '' : (el.parentElement.className || el.parentElement.tagName.toLowerCase()).toString().split(' ')[0],
          (el.className || el.tagName.toLowerCase()).toString().split(' ').join('.')])].filter(Boolean).join(' > '),
        attrs: [...el.attributes].filter(a => a.name.startsWith('data-')).map(a => a.name).join(' '),
        text: el.textContent.trim().replace(/\\s+/g, ' ').slice(0, 110), full: el.textContent.trim(), title: el.title,
        reason: el.getAttribute('data-reason'), recorded: el.hasAttribute('data-recorded'),
        box: [Math.round(box.width), Math.round(box.height)], scroll: [el.scrollWidth, el.clientWidth, el.scrollHeight, el.clientHeight],
        nowrap: style.whiteSpace === 'nowrap', ellipsis: style.textOverflow === 'ellipsis', overflow: style.overflowX,
        // Cut: the element's own box holds less than its text (one line, clipped), or an ancestor's box hides part of it.
        cutWide: el.scrollWidth > el.clientWidth + 1 && style.overflowX !== 'visible',
        cutTall: el.scrollHeight > el.clientHeight + 1 && style.overflowY !== 'visible' && style.overflowY !== 'auto' && style.overflowY !== 'scroll',
        hidden: Math.round(hidden) };
    });
    // The blocks of the body that hold less than their content (the body itself included): what a card's
    // registered height does not hold.
    const squeezed = [viewer, ...viewer.querySelectorAll('*')].filter(el => el.scrollHeight > el.clientHeight + 1
      && getComputedStyle(el).overflowY !== 'visible' && el.clientHeight > 0)
      .map(el => [(el.className || el.tagName.toLowerCase()).toString().split(' ').join('.'), el.scrollHeight, el.clientHeight]);
    return { id: card.id, name: card.querySelector('.node-title')?.textContent, kind: viewer.dataset.viewer,
      size: [card.offsetWidth, card.offsetHeight], body: [viewer.scrollHeight, viewer.clientHeight],
      viewerBottom: Math.round(frame.bottom - cardBox.bottom), squeezed, lines };
  }) };
})()`;

export const cameraAt = zoom => `(async () => {
  const a = StudyApp;
  a.state.zoom = ${zoom}; a.state.x = 0; a.state.y = 0; a.transformWorld();
  for (const travel of document.getAnimations()) if (travel.effect?.target?.matches?.('#nodes > .node')) travel.finish();
  // The fixture daemon accepts no subscription, so every card carries the disconnected screen's note over
  // its foot. A connected screen has none: the bodies are measured, and looked at, without it.
  for (const note of document.querySelectorAll('#nodes .node-status-note')) note.remove();
  await document.fonts.ready;
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  await new Promise(resolve => setTimeout(resolve, 600));
})()`;

export const ringRows = ({ kinds, local, absolute, uint }) => kinds.flatMap((kind, i) => {
  const before = (i + kinds.length - 1) % kinds.length;
  const from = absolute(local(kinds[before])), to = absolute(local(kind));
  const body = { value: 12.375, to: 'firing', from: 'clear', text: 'The gateway dispatched a job and this sentence is longer than one line of a card.' };
  return [{ kind: 'actor_emission', actor: from, port: 'event', index: 0n, observed_at_ms: BigInt(61000 + before * 1000), body,
    at: [uint(1n), uint(0n), from, uint(0n), uint(1n)] },
  { kind: 'actor_arrival', actor: to, port: 'input', index: 0n, observed_at_ms: BigInt(61000 + i * 1000), body,
    origin: [uint(1n), uint(0n), from, uint(0n), uint(1n)],
    edge: [1n, { actor: from, port: 'event' }, { actor: to, port: 'input' }, 0n] }];
});

export async function census({ shot, zoom = 1, ...options } = {}) {
  let seen;
  await canvas(async (evaluate, call) => {
    await evaluate(cameraAt(zoom));
    seen = await evaluate(measure);
    await shot?.(call, evaluate);
  }, 'detail-census', { daemon: censusDaemon(options) });
  return seen;
}
