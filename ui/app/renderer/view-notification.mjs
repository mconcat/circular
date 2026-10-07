import { kicker, escape, words, writeWords, formatDuration, formatTime, PROSE } from './view-registry.mjs';
import { viewRecords, isReceived, isOwnOutcome, recordedArrivalsText } from './arrivals.mjs';
import { rowText } from './value-text.mjs';
import { reasonText } from './reasons.mjs';

const unobserved = 'NOTIFICATION_UNOBSERVED';
const suppressionUnrecorded = 'SUPPRESSION_UNRECORDED';
const portUnrecorded = 'PORT_UNRECORDED';
const undeclared = 'UNDECLARED';

const notificationOf = (record, node) => ({
  at: Number(record.observed_at_ms ?? 0) / 1000,
  time: formatTime(record.observed_at_ms)?.detail ?? reasonText('TIME_UNRECORDED'),
  port: record.port ?? null,
  message: rowText(node, record, 'arrivals').text,
});

const byTime = (a, b) => a.observed_at_ms === b.observed_at_ms ? 0 : a.observed_at_ms < b.observed_at_ms ? -1 : 1;
const outcomeText = record => `${record.body.kind} · ${record.body.ok ? 'ok' : 'failed'}`;
export function notificationInput(node) {
  const arrivals = [...viewRecords(node)].sort(byTime);
  const notifications = arrivals.filter(record => typeof record.port === 'string' && isReceived(record))
    .map(record => notificationOf(record, node));
  const latest = new Map(arrivals.filter(isOwnOutcome).map(record => [record.body.kind, record]));
  const outcome = [...latest.values()].reverse().map(outcomeText).join(', ') || null;
  return {notifications, outcome, notificationsRecorded: typeof node.recordedArrivals === 'bigint' ? recordedArrivalsText(node) : null,
    preview: {message: notifications.at(-1)?.message ?? reasonText(unobserved)}};
}

const interval = config => config?.minimum_interval == null ? null : formatDuration(config.minimum_interval);
const fact = (name, text, { code, title } = {}) =>
  `<span data-fact="${name}"${code ? ` data-reason="${escape(code)}"` : ''}${title ? ` title="${escape(title)}"` : ''}>${escape(text)}</span>`;
const declaredFacts = config => fact('channel', `Channel: ${config?.channel == null ? reasonText(undeclared) : String(config.channel)}`,
  { code: config?.channel == null ? undeclared : null })
  + fact('interval', `Minimum interval ${interval(config)?.text ?? (config?.minimum_interval == null ? reasonText(undeclared) : String(config.minimum_interval))}`,
    { code: config?.minimum_interval == null ? undeclared : null, title: interval(config)?.title });

export function updateNotificationView(card, node, t, source, drawn) {
  if (!node) return;
  const viewer = card?.querySelector('.node-viewer[data-viewer="notification"]');
  if (!viewer) return;
  const notifications = node.notifications ?? [];
  const latest = notifications.at(-1);
  viewer.setAttribute('aria-disabled', String(!latest));
  if (latest) viewer.removeAttribute('data-reason');
  else viewer.setAttribute('data-reason', unobserved);
  const title = viewer.querySelector('[data-notification-message]');
  const detail = viewer.querySelector('[data-notification-facts]');
  const message = latest ? latest.message : reasonText(unobserved);
  if (title) writeWords(title, message);
  if (detail) {
    const html = declaredFacts(node.config) + fact('arrivals', node.notificationsRecorded ?? reasonText('READ_UNAVAILABLE'),
      { code: node.notificationsRecorded ? null : 'READ_UNAVAILABLE' });
    if (detail.innerHTML !== html) detail.innerHTML = html;
  }
  if (node.outcome) viewer.setAttribute('data-outcome', node.outcome); else viewer.removeAttribute?.('data-outcome');
  viewer.setAttribute('data-reason', latest ? suppressionUnrecorded : unobserved);
  viewer.title = latest
    ? notifications.map(row => `${row.time} · ${row.port ?? reasonText(portUnrecorded)} · ${row.message}`).join('\n')
    : `${reasonText(unobserved)} · ${reasonText(suppressionUnrecorded)}`;
}

export default {
  kind: 'notification',
  render: n => kicker(n.viewConfig?.heading) +
    `<div class="notification-content"><strong data-notification-message style="${PROSE}">${words(n.preview?.message ?? '')}</strong><span class="facts" data-notification-facts>${declaredFacts(n.config)}</span></div>`,
  glance: {
    cells: n => {
      const latest = n.notifications?.at(-1);
      return { message: latest ? { words: latest.message } : { reason: unobserved },
        arrivals: n.notificationsRecorded ? { words: n.notificationsRecorded }
          : { reason: 'READ_UNAVAILABLE' } };
    },
    reduced: ['message', 'arrivals'],
    names: ['message'],
  },
  defaultFor: ['notify'],
  reads: ['channel', 'minimum_interval'],
  size: { height: 248, min: 174 },
  input: notificationInput,
  update: updateNotificationView,
  tick: 'replace',
};
