import { reasonText } from './reasons.mjs';
import { escape, words, writeWords, kicker, formatTime, places, PROSE } from './view-registry.mjs';
import { DETAIL, NAMES } from './tier.mjs';
import { rowText, valueText } from './value-text.mjs';

export const foldLines = 40;

function fold(text) {
  const lines = text.split('\n');
  return lines.length <= foldLines ? { text, folded: 0 }
    : { text: lines.slice(0, foldLines).join('\n'), folded: lines.length - foldLines };
}

export function valueInput(node, ctx) {
  const emitted = ctx.emitted(node);
  const last = [...emitted].filter(row => typeof row.observed_at_ms === 'bigint')
    .reduce((a, b) => !a || b.observed_at_ms > a.observed_at_ms ? b : a, null);
  if (last) {
    const shown = fold(rowText(node, last, 'emitted').text);
    const value = { source: 'emitted', ...shown, at: String(last.observed_at_ms) };
    return { storedValue: { ...value, caption: valueCaption(value) } };
  }
  const config = node.declaration?.config;
  const declared = config && typeof config === 'object' && Object.hasOwn(config, 'initial');
  const shown = declared ? fold(valueText(config.initial).text) : { text: '—', folded: 0 };
  const value = { source: declared ? 'initial' : 'undeclared', ...shown, at: null, code: 'EMISSION_UNOBSERVED' };
  return { storedValue: { ...value, caption: valueCaption(value) } };
}

export function valueCaption(value) {
  const folded = value.folded ? `${value.folded} more lines folded` : null;
  if (value.source === 'emitted') return [`Recorded ${formatTime(value.at)?.text ?? value.at}`, folded].filter(Boolean);
  const code = value.code ?? 'EMISSION_UNOBSERVED';
  return [value.source === 'initial' ? null : 'No initial value declared', { text: reasonText(code), code }, folded].filter(Boolean);
}

const captionTitle = value => value.source === 'emitted' ? `Recorded ${formatTime(value.at)?.title ?? value.at}` : '';
const write = (element, text) => { if (element && element.textContent !== text) element.textContent = text; };
const facts = list => list.map(fact => typeof fact === 'string' ? `<span>${escape(fact)}</span>`
  : `<span data-reason="${escape(fact.code)}">${escape(fact.text)}</span>`).join('');
const kickerLive = value => value?.source === 'emitted' ? 'LAST EMITTED' : value?.source === 'initial' ? 'INITIAL' : '';

export function updateValueView(card, node) {
  if (!node?.storedValue) return;
  const viewer = card?.querySelector('.node-viewer[data-viewer="value"]');
  if (!viewer) return;
  const value = viewer.querySelector('.value-content');
  if (value) writeWords(value, node.storedValue.text);
  write(viewer.querySelector('.viewer-kicker > span'), kickerLive(node.storedValue));
  const caption = viewer.querySelector('[data-value-caption]');
  const shown = facts(valueCaption(node.storedValue));
  if (caption && caption.innerHTML !== shown) caption.innerHTML = shown;
  if (caption && node.storedValue.source !== 'emitted') {
    const label = reasonText(node.storedValue.code ?? 'EMISSION_UNOBSERVED');
    if (caption.title !== label) caption.title = label;
    caption.setAttribute?.('data-reason', node.storedValue.code ?? 'EMISSION_UNOBSERVED');
  } else if (caption) { caption.title = captionTitle(node.storedValue); caption.removeAttribute?.('data-reason'); }
}

export default {
  kind: 'value',
  render: (n, own, tier, accepts = true) => {
    const lineAt = tier => `;overflow-wrap:normal${tier === NAMES ? ';white-space:nowrap;overflow:hidden' : ';overflow-x:hidden'};text-overflow:ellipsis`;
    const value = (form = '') => `<pre class="value-content" style="${PROSE}${form}"${tier === NAMES ? ` title="${escape(n.storedValue?.text ?? '—')}"` : ''}>${tier === NAMES ? escape(n.storedValue?.text ?? '—') : words(n.storedValue?.text ?? '—')}</pre>`;
    const edit = face => `<button class="viewer-action" data-edit-value="${n.id}"${face}</button>`;
    return tier === DETAIL
      ? kicker(n.viewConfig?.heading, kickerLive(n.storedValue)) + value() +
        `<p class="viewer-message facts" data-value-caption${n.storedValue?.source === 'emitted' ? ` title="${escape(captionTitle(n.storedValue))}"` : ''}>${facts(n.storedValue?.caption ?? [])}</p>` + edit('>Edit initial value')
      : accepts ? places(value(lineAt(tier)) + edit(' aria-label="Edit initial value">✎'), true)
        : value(`;flex:1 1 auto;min-height:0;margin:0${lineAt(tier)}`);
  },
  defaultFor: ['json'],
  reads: ['initial'],
  size: { height: 248, min: 174 },
  input: valueInput,
  update: updateValueView,
};
