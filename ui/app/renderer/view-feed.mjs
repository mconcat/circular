import { escape, kicker, empty, readViewPath } from './view-registry.mjs';
import { viewRecords, isReceived } from './arrivals.mjs';
import { valueText, rowText } from './value-text.mjs';
import { reasonText } from './reasons.mjs';

const at = row => typeof row.observed_at_ms === 'bigint' ? row.observed_at_ms : -1n;
export function feedInput(node) {
  const events = viewRecords(node, 'both', { project: (row, via, side) => ({row, via, side}) })
    .map((event, i) => ({...event, i})).filter(({row, side}) => side !== 'arrivals' || isReceived(row))
    .sort((a, b) => at(a.row) === at(b.row) ? b.i - a.i : at(a.row) > at(b.row) ? -1 : 1);
  const fields = node.viewConfig?.fields;
  const cellOf = ({text, code}) => code ? Object.freeze({reason: code, text}) : text;
  const titleOf = body => {
    const reading = readViewPath(body, fields.title);
    return cellOf(reading.code ? { text: reasonText(reading.code), code: reading.code } : valueText(reading.value));
  };
  const rows = events.map(({row, via, side}) => [
    fields?.title === undefined ? via ?? '—' : titleOf(row.body),
    cellOf(rowText(node, row, side)),
  ]);
  return {preview:rows.length ? {rows} : undefined,feedReason:rows.length ? null : 'ARRIVAL_UNOBSERVED'};
}

export function updateFeedView(card) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="feed"]');
  if (!viewer) return;
  const code = viewer.querySelector('.empty-state')?.getAttribute('data-reason') ?? null;
  viewer.setAttribute('aria-disabled',String(Boolean(code)));
  if (code) viewer.setAttribute('data-reason',code); else viewer.removeAttribute?.('data-reason');
  viewer.title = code ? reasonText(code) : '';
  const table = viewer.querySelector('.result-table');
  if (!table) return;
  const rows = [...table.querySelectorAll('[data-feed-row]')];
  for (const row of rows) row.style.display = '';
  const padding = parseFloat(table.ownerDocument.defaultView.getComputedStyle(table).paddingBottom) || 0;
  const bottom = table.clientHeight - padding;
  const firstHidden = rows.findIndex(row => row.offsetTop + row.offsetHeight > bottom);
  if (firstHidden >= 0) for (const row of rows.slice(firstHidden)) row.style.display = 'none';
}

const cellText = v => v?.reason ? v.text ?? reasonText(v.reason) : String(v ?? '');

export default {
  kind: 'feed',
  render: n => kicker(n.viewConfig?.heading) +
    (n.preview?.rows?.length
      ? `<div class="result-table" style="position:relative;min-height:0;overflow:hidden"><div><b>Source</b><b>Value</b></div>${n.preview.rows.map(r => `<div data-feed-row>${r.map(v => v?.reason
        ? `<span data-reason="${escape(v.reason)}">${escape(v.text ?? reasonText(v.reason))}</span>`
        : `<span>${escape(v)}</span>`).join('')}</div>`).join('')}</div>`
      : empty(n.feedReason ?? 'ARRIVAL_UNOBSERVED')),
  glance: {
    cells: n => ({ rows: n.preview?.rows?.length
      ? { rows: n.preview.rows.map(([source, value]) => [cellText(value), cellText(source), value?.reason ?? null]) }
      : { reason: n.feedReason ?? 'ARRIVAL_UNOBSERVED' } }),
    reduced: ['rows'],
    names: ['rows'],
  },
  defaultFor: ['otlp', 'listener'],
  reads: [],
  size: { height: 248, min: 174 },
  input: feedInput,
  update: updateFeedView,
  tick: 'replace',
};
