import { kicker, resultRows, reasonCell, formatReading, formatTime, escape, empty } from './view-registry.mjs';
import { viewRecords, recordPort, drawnPorts, outletReading } from './arrivals.mjs';
import { identity } from './query.mjs';
import { reasonText } from './reasons.mjs';

const coverage = 'Distinct emissions in the records held by this screen';

export function routingInput(node, ctx) {
  const ports = drawnPorts(node).map(([port]) => port);
  const stamps = new Map(ports.map(port => [port, new Set()]));
  for (const row of viewRecords(node, 'emitted')) stamps.get(recordPort(node, row))?.add(identity(row.at));
  const routingRows = ports.map(port => {
    const reading = outletReading(node, port, ctx.graph.edges);
    return [port, reading.code, reading.code ? null : String(stamps.get(port).size),
      reading.latest?.observed_at_ms == null ? null : String(reading.latest.observed_at_ms)];
  });
  return { routingRows, routingReason: node.portsUnavailableReason,
    routingObserved: [coverage, ...routingRows.map(([port, code, count, ms]) => code
      ? `${port}: ${reasonText(code)}`
      : `${port}: ${formatReading(count).text} ${count === '1' ? 'emission' : 'emissions'}${ms == null ? '' : `; last recorded arrival ${formatTime(ms).detail}`}`)].join(' · ') };
}

export const routingCell = ([, code, count]) => code ? reasonCell(code)
  : formatReading(count)?.text ?? reasonCell('READ_UNAVAILABLE');

const routingReason = node => node.routingReason ?? (node.routingRows ? null : 'READ_UNAVAILABLE');

export function updateRoutingView(card, node) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="routing"]');
  if (!viewer) return;
  const code = routingReason(node);
  viewer.setAttribute('aria-disabled', String(Boolean(code)));
  if (code) viewer.setAttribute('data-reason', code);
  else viewer.removeAttribute?.('data-reason');
  viewer.title = code ? reasonText(code) : node.routingObserved ?? coverage;
}

export default {
  kind: 'routing',
  render: n => {
    const code = routingReason(n);
    const body = code
      ? `<div class="viewer-message" data-reason="${escape(code)}" title="${escape(reasonText(code))}">${escape(reasonText(code))}</div>`
      : n.routingRows.length ? resultRows(['Destination', 'Emissions'], n.routingRows.map(row => [row[0], routingCell(row)]))
        : empty('OUTLETS_UNDECLARED');
    return kicker(n.viewConfig?.heading) + body;
  },
  glance: {
    cells: n => {
      const code = routingReason(n);
      return { destinations: code ? { reason: code }
        : n.routingRows.length ? { rows: n.routingRows.map(([port, reason, count]) => [port, reason ? '—' : formatReading(count)?.text ?? '—', reason]),
          title: n.routingObserved ?? coverage }
          : { reason: 'OUTLETS_UNDECLARED' } };
    },
    reduced: ['destinations'],
    names: ['destinations'],
  },
  defaultFor: ['match', 'route'],
  reads: [],
  size: { height: 248, min: 174 },
  input: routingInput,
  update: updateRoutingView,
  tick: 'replace',
};
