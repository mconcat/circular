import { reasonText } from './reasons.mjs';
import { escape, kicker, formatTime } from './view-registry.mjs';
import { viewRecords, latestRow, isEmission, wireSource } from './arrivals.mjs';
import { rowText } from './value-text.mjs';
import { valueMarkup, contentRoles, recordedRoles, contentFacts } from './view-output.mjs';

const leftBy = row => isEmission(row) ? row.port : wireSource(row)?.port ?? null;
export function responseInput(node) {
  const latest = latestRow(viewRecords(node, 'emitted'));
  if (latest) return {response: {outlet: leftBy(latest), ms: String(latest.observed_at_ms),
    content: rowText(node, latest, 'emitted'), roles: contentRoles(node, latest, 'emitted')}};
  return {response: {code: 'EMISSION_UNOBSERVED'}};
}

export default {
  kind: 'response',
  render: n => {
    const response = n.response ?? {code: 'READ_UNAVAILABLE'};
    const time = formatTime(response.ms);
    return kicker(n.viewConfig?.heading) + (response.code
      ? `<div class="response-content" aria-disabled="true" data-reason="${escape(response.code)}" title="${escape(reasonText(response.code))}"><strong>${escape(reasonText(response.code))}</strong></div>`
      : `<div class="response-content">${recordedRoles(response.roles)}${valueMarkup(response.content)}<p class="response-source">${response.outlet ? `${escape(response.outlet)} · ` : ''}${time
        ? `<span title="${escape(time.title)}">${escape(time.text)}</span>`
        : `<span data-reason="READ_UNAVAILABLE">${escape(reasonText('READ_UNAVAILABLE'))}</span>`}</p></div>`);
  },
  glance: { cells: n => contentFacts(n.response ?? {code: 'READ_UNAVAILABLE'}), reduced: ['lead', 'value'], names: ['lead'] },
  defaultFor: ['request'],
  reads: [],
  size: { height: 248, min: 174 },
  input: responseInput,
};
