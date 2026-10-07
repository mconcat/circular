import { escape, kicker, readViewPath, formatTime } from './view-registry.mjs';
import { viewRecords, recordPort, drawnPorts, drawnSide, latestRow, outletReading } from './arrivals.mjs';
import { reasonText } from './reasons.mjs';
import { valueText } from './value-text.mjs';

const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const tabular = body => body !== null && typeof body === 'object';
const member = (value, name) => value != null && Object.hasOwn(value, name) ? value[name] : undefined;

function fields(values, shape, path = []) {
  const declared = shape?.kind === 'Object' ? shape.fields : [];
  const names = [...new Set([...declared.map(field => field.name),
    ...values.flatMap(value => object(value) ? Object.keys(value) : [])])];
  return names.flatMap(name => {
    const nested = values.map(value => member(value, name));
    const nestedShape = declared.find(field => field.name === name)?.shape;
    const nestedPath = [...path, name];
    const children = fields(nested, nestedShape, nestedPath);
    return children.length ? [...children,
      ...(nested.some(value => value !== undefined && !object(value)) ? [nestedPath] : [])] : [nestedPath];
  });
}

function rowsOf(value, shape, projection) {
  const keyed = !Array.isArray(value) && shape?.kind !== 'Object';
  const entries = keyed ? Object.entries(value) : null;
  const values = entries ? entries.map(([, item]) => item) : Array.isArray(value) ? value : [value];
  if (projection !== undefined) return { columns: projection.map(column => column.label),
    rows: values.map(item => projection.map(column => readViewPath(item, column.path))) };
  const paths = fields(values, shape?.kind === 'Array' ? shape.item : shape);
  if (paths.length && values.some(item => !object(item))) paths.push([]);
  const columns = paths.length ? paths.map(path => path.join(' · ') || 'Value') : ['Value'];
  const rows = values.map((item, index) => {
    const cells = paths.length ? paths.map(path => path.length ? path.reduce(member, item)
      : object(item) ? undefined : item) : [item];
    return entries ? [entries[index][0], ...cells] : cells;
  });
  return {columns: entries ? ['Key', ...columns] : columns, rows};
}

const cell = (value, code = value === undefined ? 'READ_UNAVAILABLE' : null) => code
  ? `<span data-reason="${escape(code)}" title="${escape(reasonText(code))}">${escape(reasonText(code))}</span>`
  : `<span>${escape(valueText(value).text)}</span>`;

export function tableInput(node, ctx) {
  const config = node.viewConfig ?? {};
  const total = config.total;
  let tableTotal;
  if (total) {
    const outlet = node.out?.find(([name]) => name === total.outlet);
    const reading = outletReading(node, total.outlet, ctx.graph.edges);
    tableTotal = { label: outlet?.[3] ?? total.outlet,
      ...(reading.latest ? readViewPath(ctx.display(reading.latest.body), total.path)
        : { code: outlet ? reading.code : 'UNDECLARED' }) };
  }
  const latest = latestRow(viewRecords(node, 'emitted').filter(row => tabular(ctx.display(row.body))));
  const port = latest ? recordPort(node, latest) : drawnPorts(node)?.[0]?.[0];
  const outlet = drawnPorts(node)?.find(([name]) => name === port);
  const tableTitle = outlet?.[3] ?? null;
  if (latest) {
    const shape = outlet?.[1]?.kind === 'Known' ? outlet[1].flow.item : undefined;
    const preview = rowsOf(ctx.display(latest.body), shape, config.columns);
    return {preview, tableReason: null, tableTitle, tableTotal,
      tableCaption: `${preview.rows.length} recorded ${preview.rows.length === 1 ? 'row' : 'rows'}.`,
      tableSource: `Last ${drawnSide(node) === 'arrivals' ? 'received' : 'emitted'}${port ? ` on ${port}` : ''} · ${formatTime(latest.observed_at_ms)?.detail ?? reasonText('TIME_UNRECORDED')}`};
  }
  return {preview: undefined, tableTitle, tableTotal,
    tableReason: port === undefined ? 'EMISSION_UNOBSERVED'
      : outletReading(node, port, ctx.graph.edges).code ?? 'READ_UNAVAILABLE'};
}

export function updateTableView(card) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="table"]');
  if (!viewer) return;
  const table = viewer.querySelector('.result-table');
  if (!table) return;
  const code = table.getAttribute('data-reason');
  viewer.setAttribute('aria-disabled',String(Boolean(code)));
  if (code) viewer.setAttribute('data-reason',code); else viewer.removeAttribute?.('data-reason');
  viewer.title = code ? reasonText(code) : '';
  const rows = [...table.querySelectorAll('[data-table-row]')];
  for (const row of rows) row.style.display = '';
  const padding = parseFloat(table.ownerDocument.defaultView.getComputedStyle(table).paddingBottom) || 0;
  const bottom = table.clientHeight - padding;
  const firstHidden = rows.findIndex(row => row.offsetTop + row.offsetHeight > bottom);
  const visible = firstHidden < 0 ? rows.length : firstHidden;
  for (const row of rows.slice(visible)) row.style.display = 'none';
  const more = viewer.querySelector('.viewer-kicker > span');
  if (more) more.textContent = visible < rows.length ? `${rows.length - visible} more` : '';
}

const cellText = (n, v) => n.viewConfig?.columns !== undefined ? (v.code ? reasonText(v.code) : valueText(v.value).text)
  : v === undefined ? reasonText('READ_UNAVAILABLE') : valueText(v).text;

export default {
  kind: 'table',
  render: n => {
    const columns = n.preview?.columns || [], data = n.preview?.rows || [];
    const grid = `grid-template-columns:repeat(${columns.length},minmax(0,1fr))`;
    const message = `<p class="viewer-message"${n.tableReason ? ` data-reason="${escape(n.tableReason)}" title="${escape(reasonText(n.tableReason))}"` : n.tableSource ? ` title="${escape(n.tableSource)}"` : ''}>${escape((n.tableReason && reasonText(n.tableReason)) || (n.viewConfig?.caption ?? n.tableCaption ?? ''))}</p>`;
    return kicker(n.viewConfig?.heading ?? n.tableTitle, '') +
      `<div class="result-table" style="position:relative;min-height:0;overflow:hidden"${n.tableReason ? ` aria-disabled="true" data-reason="${escape(n.tableReason)}"` : ''}>${columns.length ? `<div data-table-head style="${grid}">${columns.map(v => `<b>${escape(v)}</b>`).join('')}</div>` : ''}${data.length ? (columns.length ? data.map(r => `<div data-table-row style="${grid}">${r.map(v => n.viewConfig?.columns !== undefined ? cell(v.value, v.code) : cell(v)).join('')}</div>`).join('') : '') : `<div><span>—</span><span${n.tableReason ? ` data-reason="${escape(n.tableReason)}"` : ''}>${n.tableReason || !n.preview ? '' : 'No recorded rows'}</span></div>`}</div>` +
      (n.tableTotal ? `<div data-table-total><span>${escape(n.tableTotal.label)}</span> ${cell(n.tableTotal.value, n.tableTotal.code)}</div>` : '') + message;
  },
  glance: {
    cells: n => {
      const total = n.tableTotal, data = n.preview?.rows ?? [];
      return { total: total ? (total.code ? { reason: total.code } : { reading: valueText(total.value).text, caption: total.label }) : null,
        rows: n.tableReason ? { reason: n.tableReason } : data.length
          ? { rows: data.map(r => [cellText(n, r[0]), r.length > 1 ? r.slice(1).map(v => cellText(n, v)).join(' · ') : '']),
            title: n.tableSource ?? '' }
          : { words: n.preview ? 'No recorded rows' : '—' } };
    },
    reduced: ['total', 'rows'],
    names: ['total', 'rows'],
  },
  defaultFor: ['keyed_reduce', 'join'],
  reads: [],
  size: { height: 248, min: 174 },
  input: tableInput,
  update: updateTableView,
  tick: 'replace',
};
