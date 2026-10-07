import { escape, words, kicker, formatReading, formatDuration, formatTime, PROSE } from './view-registry.mjs';
import { viewRecords, latestRow } from './arrivals.mjs';
import { reasonText } from './reasons.mjs';

const stateOf = body => body && typeof body === 'object' && typeof body.to === 'string' ? body : null;

export function alertingInput(node) {
  const latest = latestRow(viewRecords(node, 'emitted').filter(row => stateOf(row.body)));
  if (latest) return {condition: {state: latest.body.to, from: typeof latest.body.from === 'string' ? latest.body.from : null,
    ms: formatReading(latest.observed_at_ms)?.full ?? null}};
  return {condition: {code: 'EMISSION_UNOBSERVED'}};
}

const unread = code => `<span data-reason="${escape(code)}">${escape(reasonText(code))}</span>`;

function renderCondition(n) {
  const condition = n.condition, code = condition?.code ?? (!condition ? 'READ_UNAVAILABLE' : null);
  const state = code ? { tone: 'unobserved', glyph: '—', text: reasonText(code) }
    : { tone: 'recorded', glyph: '—', text: `Condition state: ${condition.state}` };
  const time = formatTime(condition?.ms), recovery = formatDuration(n.config?.recovery_delay);
  const transition = code ? '' : `<small class="condition-transition facts">${condition.from ? `<span>From ${escape(condition.from)}</span>` : ''}${time
    ? `<span>Recorded at <span title="${escape(time.title)}">${escape(time.text)}</span></span>` : unread('TIME_UNRECORDED')}</small>`;
  return `<div class="alert-content ${state.tone}"${code ? ` data-reason="${escape(code)}"` : ''}>
    <span class="condition-mark" aria-hidden="true">${state.glyph}</span>
    <strong>${escape(state.text)}</strong>
    ${predicateHTML(n)}
    <small>Recovery after ${recovery ? `<span title="${escape(recovery.title)}">${escape(recovery.text)}</span>` : unread('UNDECLARED')}</small>
    ${transition}</div>`;
}
const declaredPredicate = n => typeof n.config?.predicate === 'string' && n.config.predicate.length ? n.config.predicate : null;
const predicateHTML = n => `<p class="condition-predicate" data-words="predicate" style="${PROSE}">${declaredPredicate(n) === null ? unread('UNDECLARED') : words(declaredPredicate(n))}</p>`;

export function updateAlertingView(card, node) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="alerting"]');
  if (!viewer) return;
  const condition = node?.condition, code = condition ? condition.code ?? null : 'READ_UNAVAILABLE';
  viewer.setAttribute('aria-disabled', String(Boolean(code)));
  if (code) viewer.setAttribute('data-reason', code); else viewer.removeAttribute?.('data-reason');
  const time = formatTime(condition?.ms);
  viewer.title = code ? reasonText(code) : time ? `Recorded transition at ${time.detail}` : reasonText('TIME_UNRECORDED');
}

export default {
  kind: 'alerting',
  render: n => kicker(n.viewConfig?.heading) + renderCondition(n),
  glance: {
    cells: n => {
      const condition = n.condition, code = condition?.code ?? (!condition ? 'READ_UNAVAILABLE' : null), predicate = declaredPredicate(n);
      return { state: code ? { reason: code } : { reading: condition.state, title: `Condition state: ${condition.state}` },
        predicate: predicate === null ? { reason: 'UNDECLARED', declared: 'predicate' }
          : { words: predicate, declared: 'predicate' } };
    },
    reduced: ['state', 'predicate'],
    names: ['state'],
  },
  defaultFor: ['alert'],
  reads: ['predicate', 'recovery_delay'],
  size: { height: 248, min: 174, words: 'predicate' },
  input: alertingInput,
  update: updateAlertingView,
  tick: 'replace',
};
