import { escape, spark, formatRate, kicker } from './view-registry.mjs';
import { reasonText } from './reasons.mjs';
import { viewRecords, distribute, isReceived } from './arrivals.mjs';
import { rowText } from './value-text.mjs';
import { readActorEvents } from './session.mjs';
import { RATE_SECONDS, RATE_UNIT } from './activity.mjs';

export function agentInput(node) {
  return {agentArrivals: viewRecords(node).filter(isReceived).map(r => ({at: Number(r.observed_at_ms) / 1000,
    index: r.index, body: rowText(node, r, 'arrivals').text}))};
}

export async function readAgentArrivals(session, graph, observedPage, mine) {
  const drawn = new Set(graph.nodes.filter(mine).map(node => node.id));
  if (observedPage || !drawn.size) return graph;
  return distribute(graph, await readActorEvents(session), drawn);
}

export function updateAgentView(element, node, t = globalThis.window?.StudyApp?.displayTime?.() ?? globalThis.window?.StudySource?.head ?? 0,
  source = globalThis.window?.StudySource) {
  if (!element) return;
  const set = (selector, text, diagnostic, title = '', code) => {
    const target = element.querySelector(selector);
    if (!target) return;
    target.textContent = text;
    target.setAttribute('aria-disabled', String(Boolean(diagnostic)));
    target.title = diagnostic ?? title;
    if (code) target.setAttribute('data-reason', code); else target.removeAttribute?.('data-reason');
  };
  const counted = source?.arrivalsIn?.(node.id, t - RATE_SECONDS, t);
  const rate = formatRate(counted, counted ? null : 'READ_UNAVAILABLE');
  set('[data-live="rate"]', rate.text, rate.code ? rate.title : null, rate.title, rate.code);
  const latest = (node.agentArrivals ?? []).filter(r => r.at <= t)
    .reduce((last, r) => !last || r.index > last.index ? r : last, null);
  set('.viewer-message', latest?.body ?? reasonText('ARRIVAL_UNOBSERVED'), null, latest?.body ?? '', latest ? undefined : 'ARRIVAL_UNOBSERVED');
  element.querySelector('.viewer-message')?.toggleAttribute?.('data-recorded', Boolean(latest));
  const canvas = element.querySelector('.viewer-spark');
  if (canvas) {
    canvas.setAttribute('aria-disabled', 'false');
    canvas.title = '';
  }
}

export default {
  kind: 'agent',
  render: n => {
    const rate = formatRate(null, 'READ_UNAVAILABLE');
    return `${kicker(n.viewConfig?.heading)}<div class="viewer-reading"><strong data-live="rate" title="${escape(rate.title)}">${rate.text}</strong><span>${RATE_UNIT}</span></div>${spark}${n.agentArrivals || n.preview?.text ? `<div class="viewer-message">${escape(n.agentArrivals ? '' : n.preview.text)}</div>`
      : `<div class="viewer-message" data-reason="ARRIVAL_UNOBSERVED">${escape(reasonText('ARRIVAL_UNOBSERVED'))}</div>`}`;
  },
  glance: {
    cells: () => ({ rate: { count: '—', unit: RATE_UNIT, live: 'rate' }, spark: { spark: 'Recent arrival density' } }),
    reduced: ['rate', 'spark'],
    names: ['rate'],
  },
  defaultFor: [],
  reads: [],
  size: { height: 318, min: 244 },
  input: agentInput,
  update: updateAgentView,
  tick: 'paced',
  read: readAgentArrivals,
};
