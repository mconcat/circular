import { reason } from './reasons.mjs';
import { escape, words, kicker, empty, formatTime, PROSE } from './view-registry.mjs';
import { viewRecords, isReceived, latestRow } from './arrivals.mjs';
import { rowText } from './value-text.mjs';

export const valueMarkup = ({ text, code }) => `<p class="record-value" style="${PROSE}"${code ? ` data-reason="${escape(code)}"` : ''}>${words(text)}</p>`;

export function contentRoles(node, row, side) {
  const fields = node.viewConfig?.fields;
  return Object.fromEntries(['title', 'status'].filter(role => fields?.[role] !== undefined)
    .map(role => [role, rowText(node, row, side, { value: fields[role] })]));
}

export function contentFacts({ code, content, roles, outlet, ms }) {
  if (code) return { lead: { reason: code }, value: null };
  const time = formatTime(ms), title = [outlet, time?.text].filter(Boolean).join(' · ');
  const role = roles?.status ?? roles?.title;
  const value = { words: content.text, code: content.code, title };
  return role ? { lead: { reading: role.text, code: role.code, title }, value } : { lead: value, value: null };
}

export const recordedRoles = roles => Object.entries(roles ?? {}).map(([role, reading]) =>
  `<div data-content-role="${role}"${role === 'title' ? ' style="font-weight:600"' : ''}>${valueMarkup(reading)}</div>`).join('');

export function outputInput(node) {
  const latest = latestRow(viewRecords(node).filter(isReceived));
  if (!latest) return {outputContent: undefined, outputRoles: undefined, outputReason: 'ARRIVAL_UNOBSERVED'};
  return {outputContent: rowText(node, latest, 'arrivals'), outputRoles: contentRoles(node, latest, 'arrivals'),
    outputReason: null};
}

export function updateOutputView(card, node) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="output"]');
  if (!viewer) return;
  viewer.setAttribute('aria-disabled', String(Boolean(node.outputReason)));
  if (!node.outputReason) viewer.removeAttribute?.('data-reason');
  if (node.outputReason) {
    const diagnostic = reason(node.outputReason);
    viewer.setAttribute('aria-disabled', 'true');
    viewer.setAttribute('data-reason', diagnostic.code);
    viewer.title = diagnostic.label;
    const line = viewer.querySelector('.empty-state');
    if (line) {
      line.setAttribute('data-reason', diagnostic.code);
      const sentence = line.firstElementChild ?? line.querySelector?.('span');
      if (sentence) sentence.textContent = diagnostic.label;
    }
    return;
  }
  viewer.title = '';
}

export default {
  kind: 'output',
  render: n => kicker(n.viewConfig?.heading ?? n.config?.label) +
    (n.outputContent
      ? `<div class="response-content">${recordedRoles(n.outputRoles)}${valueMarkup(n.outputContent)}</div>`
      : empty(n.outputReason ?? 'ARRIVAL_UNOBSERVED')),
  glance: {
    cells: n => contentFacts(n.outputContent ? { content: n.outputContent, roles: n.outputRoles } : { code: n.outputReason ?? 'ARRIVAL_UNOBSERVED' }),
    reduced: ['lead', 'value'],
    names: ['lead'],
  },
  defaultFor: ['output'],
  size: { height: 248, min: 174 },
  input: outputInput,
  update: updateOutputView,
  tick: 'replace',
};
