import { escape, words, writeWords, formatReading, formatTime, kicker, empty } from './view-registry.mjs';
import { viewRecords, isReceived, isEmission } from './arrivals.mjs';
import { rowText } from './value-text.mjs';
import { reasonText } from './reasons.mjs';
export const transcriptRows = 3;
const unobserved = 'ARRIVAL_UNOBSERVED';
const reasonAttribute = code => code ? ` data-reason="${escape(code)}"` : '';
const rowDescription = row => [row.localTitle, row.timeTitle, row.description].filter(Boolean).join(' · ');
const rowDetail = row => [rowDescription(row), row.detail].filter(Boolean).join('\n');
const rowHTML = row => `<div class="viewer-table-row" data-has-kind="${Boolean(row.kind)}"${reasonAttribute(row.code)}>
  <span class="record-local" title="${escape(row.localTitle)}"${reasonAttribute(row.localCode)}>${escape(row.local)}</span>
  <span class="record-kind"${row.kind ? '' : ' hidden'}>${escape(row.kind)}</span>
  <small class="record-time" title="${escape(row.timeTitle)}"${reasonAttribute(row.timeCode)}>${escape(row.time)}</small>
  <span class="record-value" title="${escape(rowDetail(row))}"${reasonAttribute(row.valueCode)}>${words(row.value)}</span></div>`;

export function transcriptInput(node, { display }) {
  const transcriptArrivals = viewRecords(node).filter(isReceived).toReversed().map(record => {
    const index = formatReading(record.index), time = formatTime(record.observed_at_ms);
    const kind = typeof record.body?.kind === 'string' && record.body.kind.length ? record.body.kind : null;
    const value = rowText(node, record, 'arrivals');
    return {
      at: time ? Number(time.full) / 1000 : null,
      local: index ? `#${index.text}` : reasonText('LOCAL_INDEX_UNRECORDED'),
      localTitle: index ? `Actor-local index ${index.full}` : reasonText('LOCAL_INDEX_UNRECORDED'),
      localCode: index ? null : 'LOCAL_INDEX_UNRECORDED',
      time: time ? time.text : reasonText('TIME_UNRECORDED'),
      timeTitle: time ? `Recorded at ${time.detail}` : reasonText('TIME_UNRECORDED'),
      timeCode: time ? null : 'TIME_UNRECORDED',
      kind,
      value: value.text,
      valueCode: value.code,
      code: record.port == null ? 'PORT_UNRECORDED' : null,
      description: record.port == null ? reasonText('PORT_UNRECORDED')
        : `${isEmission(record) ? 'Outlet' : 'Receiving inlet'}: ${record.port}`,
      detail: JSON.stringify(display({actor: record.actor, index: record.index, port: record.port,
        observed_at_ms: record.observed_at_ms, body: record.body})),
    };
  });
  return {
    transcriptArrivals,
    preview: {rows: transcriptArrivals.map(row => [row.local, row.kind ?? row.value])},
  };
}

export function updateTranscriptView(card, node, t = globalThis.window?.StudyApp?.displayTime?.() ?? Infinity, source, drawn) {
  if (!node) return;
  const viewer = card?.querySelector('.node-viewer[data-viewer="transcript"]');
  if (!viewer) return;
  viewer.setAttribute('aria-disabled', 'false');
  viewer.title = '';
  const table = viewer.querySelector('.records-table');
  table?.setAttribute('aria-label', 'actor-local arrival records');
  const head = viewer.querySelector('.viewer-table-head span');
  if (head) head.title = 'Receiving actor-local index';
  const arrivals = node.transcriptArrivals.filter(record => record.at === null ? !Number.isFinite(t) : record.at <= t);
  const reference = Number.isFinite(t) ? t : Math.max(0, ...arrivals.map(record => record.at));
  const rows = [...viewer.querySelectorAll('.records-table .viewer-table-row')];
  const shown = Math.min(rows.length, arrivals.length);
  if (table) {
    const none = !arrivals.length, line = viewer.querySelector(':scope > .empty-state');
    table.hidden = none;
    if (none && !line) table.insertAdjacentHTML('afterend', empty(unobserved));
    if (!none) line?.remove();
  }
  let held = '';
  for (const [i, row] of rows.entries()) {
    const arrival = arrivals[i];
    row.style.display = i < shown ? '' : 'none';
    if (!arrival) { row.setAttribute('data-hf-08-age', 'none'); continue; }
    for (const [element, value, title, code, asWords] of [
      [row.firstElementChild, arrival.local, arrival.localTitle, arrival.localCode],
      [row.querySelector('.record-time'), arrival.time, arrival.timeTitle, arrival.timeCode],
      [row.lastElementChild, arrival.value, arrival.detail, arrival.valueCode, true],
    ]) {
      if (!element) continue;
      if (asWords) writeWords(element, value ?? ''); else element.textContent = value ?? '';
      element.title = title ?? '';
      if (code) element.setAttribute('data-reason', code); else element.removeAttribute('data-reason');
    }
    const kind = row.querySelector('.record-kind');
    if (kind) { kind.textContent = arrival.kind ?? ''; kind.hidden = !arrival.kind; }
    row.setAttribute('data-has-kind', String(Boolean(arrival.kind)));
    if (arrival.code) row.setAttribute('data-reason', arrival.code); else row.removeAttribute('data-reason');
    row.lastElementChild.title = rowDetail(arrival);
    row.title = arrival.detail ?? '';
    row.setAttribute('aria-label', arrival.detail ?? '');
    row.setAttribute('aria-description', rowDescription(arrival));
    const age = arrival.at != null ? Math.max(0, reference - arrival.at) : null;
    row.setAttribute('data-hf-08-age', age === null ? 'none' : age < 1 ? '0' : age < 5 ? '1' : age < 30 ? '2' : '3');
    held += `${arrival.local}\t${arrival.time}\t${arrival.kind}\t${arrival.value}\n`;
  }
  const key = `${node.width}|${node.height}|${drawn?.scale ?? 1}|${drawn?.tier}|${held}`;
  let fit = fitted.get(viewer);
  if (fit?.key !== key) {
    const bottom = viewer.getBoundingClientRect().bottom;
    fit = {key, firstHidden:rows.findIndex(row => row.style.display !== 'none' && row.getBoundingClientRect().bottom > bottom)};
    fitted.set(viewer, fit);
  }
  if (fit.firstHidden >= 0) for (const row of rows.slice(fit.firstHidden)) row.style.display = 'none';
}
const fitted = new WeakMap();

const arrivalsAt = (n, t) => (n.transcriptArrivals ?? []).filter(record => record.at === null ? !Number.isFinite(t) : record.at <= t);

export default {
  kind: 'transcript',
  render: n => {
    const rows = n.transcriptArrivals ?? (n.preview?.rows ?? []).map(([local, value]) => ({local, value}));
    const table = rows.length ? `<div class="viewer-table records-table" aria-label="actor-local arrival records"><div class="viewer-table-head"><span>LOCAL</span><span>KIND / VALUE</span></div>${rows.map(rowHTML).join('')}</div>` : '';
    return `${kicker(n.viewConfig?.heading, '\u00a0')}${table || empty(unobserved)}`;
  },
  glance: {
    cells: (n, t = Infinity) => {
      const arrivals = arrivalsAt(n, t);
      if (!arrivals.length) return { rows: { reason: unobserved }, latest: { reason: unobserved } };
      return { rows: { rows: arrivals.map(row => [row.kind ?? row.value, '', row.valueCode, rowDetail(row)]) },
        latest: { words: arrivals[0].kind ?? arrivals[0].value, title: rowDetail(arrivals[0]) } };
    },
    reduced: ['rows'],
    names: ['latest'],
  },
  defaultFor: ['tap'],
  reads: [],
  size: { height: 236, min: 200 },
  rows: transcriptRows,
  input: transcriptInput,
  update: updateTranscriptView,
  tick: 'replace',
};
