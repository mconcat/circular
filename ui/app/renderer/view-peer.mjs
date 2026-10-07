import { reasonText } from './reasons.mjs';
import { escape, kicker, formatReading, formatTime } from './view-registry.mjs';
import { outletReading, primaryOutletReading } from './arrivals.mjs';
import { rowText } from './value-text.mjs';
import { valueMarkup } from './view-output.mjs';

const declaredLine = (config, key) => config?.[key] == null
  ? { code: 'UNDECLARED', text: reasonText('UNDECLARED'), title: reasonText('UNDECLARED') }
  : { text: String(config[key]), title: '' };

export function peerInput(node, ctx) {
  const primary = primaryOutletReading(node, ctx.graph.edges);
  const status = node.viewConfig?.fields?.status;
  const line = reading => {
    const port = node.out?.find(([id]) => id === reading.port);
    return { label: port?.[3] ?? reading.port,
      ...(status === undefined ? {} : { status: reading.latest
        ? { value: rowText(node, reading.latest, 'emitted', { value: status }) } : { code: reading.code } }),
      ...(reading.latest ? { value: rowText(node, reading.latest, 'emitted'),
        ms: formatReading(reading.latest.observed_at_ms)?.full } : { code: reading.code }) };
  };
  return { peer: {
    message: primary ? line(primary) : { code: node.portsUnavailableReason ?? 'UNDECLARED' },
    outputs: (node.out ?? []).filter(([id]) => id !== primary?.port)
      .map(([id]) => line(outletReading(node, id, ctx.graph.edges))),
  } };
}

const unread = { message: { code: 'READ_UNAVAILABLE' }, outputs: [] };
const readingAttributes = reading => reading.code
  ? ` data-reason="${escape(reading.code)}" title="${escape(reasonText(reading.code))}"`
  : reading.ms == null ? '' : ` title="${escape(`Recorded ${formatTime(reading.ms)?.detail ?? reading.ms}`)}"`;
const readingMarkup = reading => reading.code ? escape(reasonText(reading.code)) : valueMarkup(reading.value);
const statusMarkup = reading => reading.status
  ? `<div data-peer-status${readingAttributes(reading.status)}>${readingMarkup(reading.status)}</div>` : '';
const peerCode = peer => [peer.message, ...peer.outputs].every(reading => reading.code) ? peer.message.code : null;

export function updatePeerView(card, node) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="peer"]');
  if (!viewer) return;
  const code = peerCode(node.peer ?? unread);
  viewer.setAttribute('aria-disabled', String(Boolean(code)));
  if (code) viewer.setAttribute('data-reason', code); else viewer.removeAttribute?.('data-reason');
}

export default {
  kind: 'peer',
  render: n => {
    const name = declaredLine(n.config, 'name'), realm = declaredLine(n.config, 'realm');
    const peer = n.peer ?? unread;
    return kicker(n.viewConfig?.heading) +
      `<div class="peer-content response-content" style="overflow-wrap:anywhere;pointer-events:auto"><strong${readingAttributes(name)} style="color:var(--ink)">${escape(name.text)}</strong>` +
      peer.outputs.map(reading => `<details class="peer-output"${readingAttributes(reading)}><summary>${escape(reading.label)}${statusMarkup(reading)}</summary><div>${readingMarkup(reading)}</div></details>`).join('') +
      `<section class="peer-message"${readingAttributes(peer.message)}>${peer.message.label == null ? '' : `<small>${escape(peer.message.label)}</small>`}${statusMarkup(peer.message)}<div style="color:var(--ink)">${readingMarkup(peer.message)}</div></section>` +
      `<small class="peer-realm"${readingAttributes(realm)} style="border-top:1px solid var(--line-soft);padding-top:6px">${escape(realm.text)}</small></div>`;
  },
  glance: {
    cells: n => {
      const name = declaredLine(n.config, 'name'), message = (n.peer ?? unread).message;
      const time = message.ms == null ? '' : `Recorded ${formatTime(message.ms)?.detail ?? message.ms}`;
      return { name: name.code ? { reason: name.code } : { reading: name.text },
        message: message.code ? { reason: message.code }
          : message.status && !message.status.code ? { words: message.status.value.text, title: time }
            : { words: message.value.text, code: message.value.code, title: time } };
    },
    reduced: ['name', 'message'],
    names: ['name'],
  },
  defaultFor: ['peer'],
  reads: ['name', 'realm'],
  size: { height: 248, min: 174 },
  input: peerInput,
  update: updatePeerView,
  tick: 'replace',
};
