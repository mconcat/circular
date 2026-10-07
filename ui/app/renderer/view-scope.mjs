import { reasonText } from './reasons.mjs';
import { identity } from './query.mjs';
import { escape, kicker, formatReading, formAt, places, readingAt, empty } from './view-registry.mjs';
import { DETAIL, NAMES } from './tier.mjs';

export function scopeSummaries(graph) {
  const summaries = new Map(graph.scopes.map(scope => [scope.id, { count: 0, attention: 0 }]));
  for (const child of graph.nodes) {
    const summary = summaries.get(child.scope);
    summary.count++;
    if (!child.health || child.health.state === 'unobserved') summary.attention = null;
    else if (summary.attention !== null && child.health.state === 'failed') summary.attention++;
  }
  return summaries;
}

export function scopeInput(node, { graph }) {
  const child = identity([...node.address.scope, { name: node.address.local }]);
  return { scopeSummary: scopeSummaries(graph).get(child) ?? { count: null, attention: null },
    scopeBoundary: { code: node.portsUnavailableReason,
      ports: ['in', 'out'].flatMap(side => (node[side] ?? []).map(([id, , , label]) =>
        ({ direction: side === 'in' ? 'input' : 'output', label: label ?? id }))) } };
}

const label = reasonText;
const countTitle = (count, words) => count == null ? label('READ_UNAVAILABLE') : [formatReading(count).full, words].filter(Boolean).join(' ');
const attentionText = (attention, words) => attention == null ? label('unobserved')
  : [words, attention ? `${formatReading(attention).text} need attention` : 'no issues observed'].filter(Boolean).join(' · ');

function boundaryMarkup(boundary) {
  const code = boundary?.code ?? (boundary ? null : 'READ_UNAVAILABLE');
  if (code) return `<p class="scope-boundary" data-reason="${escape(code)}" style="margin:0">${escape(reasonText(code))}</p>`;
  if (!boundary.ports.length) return empty('BOUNDARY_PORTS_UNDECLARED');
  return `<ul class="scope-boundary" aria-label="Boundary ports" style="list-style:none;margin:0;padding:0">${boundary.ports.map(port =>
    `<li style="display:flex;align-items:baseline;gap:8px;padding:2px 0"><span aria-hidden="true">•</span><span style="flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;color:var(--ink)">${escape(port.label)}</span><small>${port.direction}</small></li>`).join('')}</ul>`;
}

export function updateScopeView(element, node) {
  const summary = node.scopeSummary;
  const count = element?.querySelector('.scope-summary strong');
  const attention = element?.querySelector('.scope-summary span');
  if (count) {
    count.textContent = formatReading(summary?.count)?.text ?? '—';
    count.setAttribute('aria-disabled', String(summary?.count == null));
    count.title = countTitle(summary?.count, node.viewConfig?.count_label);
    if (summary?.count == null) count.setAttribute('data-reason', 'READ_UNAVAILABLE'); else count.removeAttribute?.('data-reason');
  }
  if (attention) {
    attention.textContent = attentionText(summary?.attention, node.viewConfig?.count_label);
    attention.setAttribute('aria-disabled', String(summary?.attention == null));
    attention.title = summary?.attention == null ? label('unobserved') : '';
    if (summary?.attention == null) attention.setAttribute('data-reason', 'unobserved'); else attention.removeAttribute?.('data-reason');
  }
}

export default {
  kind: 'scope',
  render: (n, own, tier, accepts = true) => {
    const s = n.scopeSummary, line = readingAt(tier, true), words = n.viewConfig?.count_label;
    const summary = `<div class="scope-summary${formAt(tier)}"${line.block}><strong${s?.count == null ? ' data-reason="READ_UNAVAILABLE"' : ''}${line.value} title="${escape(countTitle(s?.count, words))}">${escape(formatReading(s?.count)?.text ?? '—')}</strong><span${s?.attention == null ? ' data-reason="unobserved"' : ''}${line.label}>${escape(attentionText(s?.attention, words))}</span></div>`;
    const boundary = `<div class="scope-description" style="margin:0;display:flex;flex-direction:column;gap:8px;pointer-events:auto">${boundaryMarkup(n.scopeBoundary)}</div>`;
    const enter = face => `<button class="scope-enter" data-enter="${escape(n.scope ?? '')}"${n.scope ? '' : ` disabled data-reason="READ_UNAVAILABLE" title="${escape(label('READ_UNAVAILABLE'))}"`}${face}<span>↗</span></button>`;
    return tier === DETAIL ? kicker(n.viewConfig?.heading) + summary + boundary + enter('>Enter scope ')
      : (accepts ? places(summary + enter(' style="align-self:flex-start" aria-label="Enter scope">')) : summary) + (tier === NAMES ? '' : boundary);
  },
  defaultFor: ['pipeline_actor'],
  traits: { inlineSettings: false },
  size: { height: 250, min: 220 },
  input: scopeInput,
  update: updateScopeView,
};
