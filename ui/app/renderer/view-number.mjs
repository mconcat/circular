import { escape, kicker, formatReading, formatTime, writeFacts } from './view-registry.mjs';
import { viewRecords, recordPort, drawnPorts, latestRow, recordedArrivalsText } from './arrivals.mjs';
import { valueReading, declaredMembers } from './view-trend.mjs';
import { reasonText } from './reasons.mjs';

export function acceptedArrivals(node) {
  const newest = latestRow(viewRecords(node, 'arrivals', { sided: true }));
  return {count: typeof node.recordedArrivals === 'bigint' ? node.recordedArrivals : null,
    newest: typeof newest?.observed_at_ms === 'bigint' ? String(newest.observed_at_ms) : null};
}

export const emissionUnobserved = 'EMISSION_UNOBSERVED';
export const emissionNotNumeric = 'EMISSION_NOT_NUMERIC';
const byTime = (a, b) => a.observed_at_ms === b.observed_at_ms ? 0 : a.observed_at_ms < b.observed_at_ms ? -1 : 1;
const portFlow = (ports, id) => (ports ?? []).find(([port]) => port === id)?.[1];

export function numberInput(node) {
  const emissions = [...viewRecords(node, 'emitted')].filter(row => typeof row.observed_at_ms === 'bigint').sort(byTime)
    .map(row => {
      const value = valueReading(row.body, declaredMembers(portFlow(drawnPorts(node), recordPort(node, row))));
      const at = {at: Number(row.observed_at_ms) / 1000, ms: String(row.observed_at_ms)};
      return value === null ? {...at, code: emissionNotNumeric} : {...at, value: value.value, member: value.member};
    });
  return {acceptedArrivals: acceptedArrivals(node), emittedValue: {emissions}};
}

export function numberReading(emitted, t = Infinity) {
  if (!emitted) return {code: 'READ_UNAVAILABLE'};
  const latest = emitted.emissions.filter(row => row.at <= t).at(-1);
  if (latest) return latest;
  return {code: emissionUnobserved};
}

const factSlots = 2;
export function updateNumberView(card, node, t = globalThis.window?.StudyApp?.displayTime?.() ?? Infinity) {
  if (!node) return;
  const viewer = card?.querySelector('.node-viewer[data-viewer="number"]');
  if (!viewer) return;
  const observed = node.acceptedArrivals, shown = numberReading(node.emittedValue, t);
  const formatted = !shown.code && formatReading(shown.value, { label: shown.member });
  const arrivals = recordedArrivalsText({ recordedArrivals: observed?.count == null ? null : BigInt(observed.count) });
  const page = observed?.count == null ? reasonText('READ_UNAVAILABLE')
    : `${observed.count} arrivals recorded at this actor · ${observed.newest == null ? 'newest unobserved' : `newest recorded ${formatTime(observed.newest).detail}`}`;
  const detail = shown.code ? `${reasonText(shown.code)} · ${page}` : `last emission ${formatTime(shown.ms).detail} · value ${formatted.full} · ${page}`;
  viewer.setAttribute('aria-disabled', String(Boolean(shown.code)));
  if (shown.code) viewer.setAttribute('data-reason', shown.code); else viewer.removeAttribute?.('data-reason');
  viewer.title = detail;
  const value = viewer.querySelector('[data-product-live="number"]');
  if (value) {
    value.textContent = shown.code ? '—' : formatted.text;
    value.title = shown.code ? reasonText(shown.code) : formatted.title;
  }
  const label = viewer.querySelector('.number-content > span');
  if (label) {
    const counted = observed?.count == null ? (shown.code === 'READ_UNAVAILABLE' ? null : [arrivals, 'READ_UNAVAILABLE']) : [arrivals, null];
    writeFacts(label, [shown.code ? [reasonText(shown.code), shown.code] : [node.viewConfig?.count_label ?? shown.member, null], counted ?? [null]]);
    label.title = detail;
  }
}

export function numberFacts(n, t = Infinity) {
  const shown = numberReading(n.emittedValue, t), formatted = !shown.code && formatReading(shown.value, { label: shown.member });
  const label = n.viewConfig?.count_label ?? shown.member ?? n.viewConfig?.heading ?? null;
  return { value: shown.code ? { reason: shown.code } : { reading: formatted.text, lead: true, title: formatted.title },
    label: label ? { words: label } : null,
    reading: shown.code ? (label ? { count: '—', noun: label, code: shown.code, title: reasonText(shown.code) } : { reason: shown.code })
      : label ? { count: formatted.text, noun: label, title: formatted.title } : { reading: formatted.text, title: formatted.title } };
}

export default {
  kind: 'number',
  render: n => {
    const value = formatReading(n.preview?.value, { label: n.preview?.label });
    return kicker(n.viewConfig?.heading) +
      `<div class="number-content"><strong data-product-live="number"${value ? ` title="${escape(value.title)}"` : ''}>${escape(value?.text ?? '—')}</strong><span class="facts"><span data-fact>${escape(n.viewConfig?.count_label ?? n.preview?.label ?? '')}</span>${'<span data-fact></span>'.repeat(factSlots - 1)}</span></div>`;
  },
  glance: { cells: numberFacts, reduced: ['value', 'label'], names: ['reading'] },
  defaultFor: ['counter'],
  reads: [],
  size: { height: 248, min: 174 },
  input: numberInput,
  update: updateNumberView,
  tick: 'replace',
};
