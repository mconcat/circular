import { viewRecords, recordPort, drawnPorts, drawnSide, recordedArrivalsText } from './arrivals.mjs';
import { kicker, objectMembers, formatReading, formatTime, writeFacts } from './view-registry.mjs';
import { reasonText } from './reasons.mjs';
const unobserved = 'READ_UNAVAILABLE';

export const declaredMembers = flow => objectMembers(flow).map(field => field.name);

export function valueReading(body, members = []) {
  if (formatReading(body)) return { value: body, member: null, detail: [] };
  if (!body || typeof body !== 'object' || Array.isArray(body) || body instanceof Uint8Array) return null;
  const index = members.findIndex(name => formatReading(body[name]) !== null);
  if (index < 0) return null;
  return {
    value: body[members[index]], member: members[index],
    detail: members.filter((name, at) => at !== index && formatReading(body[name]) !== null)
      .map(name => `${name} ${formatReading(body[name]).full}`),
  };
}

const byTime = (a, b) => a.observed_at_ms === b.observed_at_ms ? 0 : a.observed_at_ms < b.observed_at_ms ? -1 : 1;
const point = (row, value, port) => ({ at: Number(row.observed_at_ms) / 1000, ms: String(row.observed_at_ms),
  value: value.value, member: value.member, detail: value.detail, port });

export function trendInput(node) {
  const side = drawnSide(node), emissions = viewRecords(node, side, { sided: true });
  const inlets = new Set((node.in ?? []).map(([id]) => id));
  const received = viewRecords(node, 'arrivals', { sided: true }).filter(row => inlets.has(row.port));
  const ports = drawnPorts(node);
  const emitted = [...emissions].sort(byTime).flatMap(row => {
    const port = recordPort(node, row), declared = (ports ?? []).find(([id]) => id === port);
    const value = valueReading(row.body, declaredMembers(declared?.[1]));
    return value === null ? [] : [point(row, value, declared?.[3] ?? port ?? null)];
  });
  const code = node.recordedEmissions != null ? null : 'EMISSION_UNOBSERVED';
  const stamped = code ? reasonText(code) : `${node.recordedEmissions} ${String(node.recordedEmissions) === '1' ? 'emission' : 'emissions'}`;
  return { trendObservation: { emitted, sampled: received.length > 0,
    recorded: { arrivals: typeof node.recordedArrivals === 'bigint' ? recordedArrivalsText(node) : null, emissions: stamped, code } } };
}

export const plotted = (node, t) => (node.trendObservation?.emitted ?? []).filter(row => row.at <= t);
export function trendSeries(node, t) {
  const emitted = plotted(node, t);
  return { value: emitted.length ? Number(formatReading(emitted.at(-1).value).full) : null };
}

const factSlots = 2;
export function updateTrendView(card, node, t = globalThis.window?.StudyApp?.displayTime?.()
  ?? globalThis.window?.StudySource?.head ?? Infinity) {
  if (!node) return;
  const viewer = card?.querySelector('.node-viewer[data-viewer="trend"]');
  if (!viewer) return;
  const observed = node.trendObservation;
  const emitted = plotted(node, t);
  const latest = emitted[emitted.length - 1] ?? null;
  const sampleDetail = observed?.recorded.arrivals ?? reasonText(unobserved);
  const emissions = observed?.recorded.code ? reasonText(observed.recorded.code) : observed?.recorded.emissions;
  const detail = !observed ? reasonText(unobserved)
    : latest ? `${emissions} · last emission ${formatTime(latest.ms)?.detail ?? latest.ms} · ${sampleDetail}`
      : `${emissions} · ${sampleDetail}`;
  viewer.setAttribute('aria-disabled', String(!observed));
  viewer.title = detail;
  const code = !observed ? unobserved : observed.recorded.code;
  if (code) viewer.setAttribute('data-reason', code); else viewer.removeAttribute?.('data-reason');
  const value = viewer.querySelector('[data-product-live="number"]');
  const line = latest ? [node.viewConfig?.caption ?? latest.member ?? latest.port, ...latest.detail].filter(Boolean).join(' · ') : null;
  if (value) {
    const formatted = latest && formatReading(latest.value, { label: line });
    value.textContent = formatted ? formatted.text : '—';
    value.title = formatted ? formatted.title : detail;
  }
  const label = viewer.querySelector('.number-content > span');
  if (label) {
    const arrivals = observed?.recorded.arrivals ? [observed.recorded.arrivals, null] : [reasonText(unobserved), unobserved];
    writeFacts(label, !observed ? [[reasonText(unobserved), unobserved]]
      : latest ? [[line, null]]
        : observed.sampled ? [[observed.recorded.emissions, observed.recorded.code], arrivals]
          : [[reasonText('ARRIVAL_UNOBSERVED'), 'ARRIVAL_UNOBSERVED']]);
    label.title = detail;
  }
  const canvas = viewer.querySelector('.viewer-spark');
  if (canvas) {
    canvas.setAttribute('aria-disabled', String(!emitted.length));
    canvas.setAttribute('aria-label', detail);
    canvas.title = detail;
    if (!emitted.length) canvas.getContext('2d')?.clearRect(0, 0, canvas.width, canvas.height);
  }
}

function trendFacts(n, t = Infinity) {
  const observed = n.trendObservation, latest = plotted(n, t).at(-1) ?? null;
  const line = latest ? [n.viewConfig?.caption ?? latest.member ?? latest.port, ...latest.detail].filter(Boolean).join(' · ') : null;
  const formatted = latest && formatReading(latest.value, { label: line });
  const code = !observed ? unobserved : formatted ? null : observed.recorded.code ?? (observed.sampled ? null : 'ARRIVAL_UNOBSERVED');
  const none = code ? { reason: code } : { words: observed.recorded.emissions };
  return { value: formatted ? { reading: formatted.text, lead: true, title: formatted.title } : none,
    label: formatted && line ? { words: line } : null,
    reading: !formatted ? none : line ? { count: formatted.text, noun: line, title: formatted.title } : { reading: formatted.text, title: formatted.title } };
}

export default {
  kind: 'trend',
  render: n => kicker(n.viewConfig?.heading) +
    `<div class="number-content"><strong data-product-live="number">—</strong><span class="facts"><span data-fact></span>${'<span data-fact></span>'.repeat(factSlots - 1)}</span></div><canvas class="viewer-spark" width="480" height="130" aria-label="Emitted values"></canvas>`,
  glance: { cells: trendFacts, reduced: ['value', 'label'], names: ['reading'] },
  defaultFor: ['ema', 'windowed_reduce'],
  reads: [],
  size: { height: 248, min: 174 },
  input: trendInput,
  update: updateTrendView,
  tick: 'paced',
  sample: trendSeries,
};
