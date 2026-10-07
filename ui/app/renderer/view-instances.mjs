import { scopeIdentityValue } from '@circular/protocol/establishment';
import { identity, domId } from './query.mjs';
import { codeText, reasonText } from './reasons.mjs';
import { escape, kicker, formatReading, formAt, places, readingAt } from './view-registry.mjs';
import { DETAIL } from './tier.mjs';
import { decodeInstanceTransitions, projectActiveInstanceFacts } from '@circular/protocol';
import { decoded, readInstanceTransitions } from './session.mjs';

const containerOf = address => scopeIdentityValue([...address.scope, { name: address.local }]);

export async function readInstances(session, limit, lens) {
  try {
    const page = await readInstanceTransitions(session, limit, lens);
    const rows = decoded(() => decodeInstanceTransitions(page.items));
    const containers = new Map(rows.filter(row => row.container).map(row => [identity(row.container), row.container]));
    const tables = new Map();
    for (const [id, container] of containers)
      tables.set(id, decoded(() => projectActiveInstanceFacts(page.anchor, page.items, container)).rows.map(row => row.key));
    return { rows, tables, diagnostic: null };
  } catch (error) {
    return { rows: [], tables: new Map(), diagnostic: error.code ?? 'READ_UNAVAILABLE' };
  }
}

export function instanceFacts(address, instances) {
  if (!instances || instances.diagnostic) return { code: instances?.diagnostic ?? 'READ_UNAVAILABLE' };
  const container = identity(containerOf(address)), base = scopeIdentityValue(address.scope);
  const current = instances.tables.get(container) ?? [];
  const history = instances.rows.filter(row => row.container && identity(row.container) === container);
  const retired = history.filter(row => row.kind === 'Retired').length;
  const phases = new Map();
  for (const row of instances.rows) {
    if (row.container && identity(row.container) === container) phases.delete(identity(row.key));
    if (!row.phase) continue;
    const segment = row.actor.scope[base.length];
    if (segment?.[0] === 2n && segment[1] === address.local && identity(row.actor.scope.slice(0, base.length)) === identity(base))
      phases.set(identity(segment[2]), row.phase);
  }
  return { current: current.map(key => ({ key: String(key), phase: phases.get(identity(key)) ?? null,
    scope: domId(identity([...address.scope, { of: address.local, key }])) })), retired,
    history: history.map(row => ({ key: String(row.key), kind: row.kind })) };
}

export function instancesInput(node, { instances }) {
  return { instancesView: instanceFacts(node.address, instances) };
}
export function instancesLine(view) {
  if (readCode(view)) return reasonText(readCode(view));
  return `${formatReading(view.retired).text} retired`;
}
const readCode = view => view?.code ?? (view?.current ? null : 'READ_UNAVAILABLE');
const countReading = view => readCode(view) ? null : formatReading(view.current.length);
const currentTitle = (view, label) => readCode(view) ? reasonText(readCode(view))
  : [countReading(view).full, label].filter(Boolean).join(' ');
const reasonAttribute = code => code ? ` data-reason="${escape(codeText(code))}"` : '';

const write = (element, text) => { if (element && element.textContent !== text) element.textContent = text; };
const titled = (element, text) => { if (element && element.title !== text) element.title = text; };
const attributed = (element, code) => {
  if (code) element?.setAttribute('data-reason', codeText(code));
  else element?.removeAttribute('data-reason');
};

export function updateInstancesView(card, node) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="instances"]');
  if (!viewer) return;
  const view = node?.instancesView;
  const count = viewer.querySelector('[data-instances-current]');
  write(count, countReading(view)?.text ?? '—');
  titled(count, currentTitle(view, node.viewConfig?.count_label));
  attributed(count, readCode(view));
  const status = viewer.querySelector('[data-instances-status]');
  write(status, instancesLine(view));
  titled(status, instancesLine(view));
  attributed(status, readCode(view));
}

export default {
  kind: 'instances',
  render: (n, own, tier, accepts = true) => {
    const view = n.instancesView, line = readingAt(tier, true), label = n.viewConfig?.count_label;
    const summary = `<div class="instance-summary${formAt(tier)}"${line.block}><strong data-instances-current${reasonAttribute(readCode(view))}${line.value} title="${escape(currentTitle(view, label))}">${escape(countReading(view)?.text ?? '—')}</strong>${label ? `<span${line.label}>${escape(label)}</span>` : ''}</div>`;
    const show = face => `<button class="viewer-action" data-show-instances="${escape(n.id)}"${face}</button>`;
    return tier === DETAIL
      ? kicker(n.viewConfig?.heading) + summary + `<div class="viewer-message" data-instances-status${reasonAttribute(readCode(view))} title="${escape(instancesLine(view))}">${escape(instancesLine(view))}</div>` + show('>View instances ↗')
      : accepts ? places(summary + show(' style="align-self:flex-start" aria-label="View instances">↗')) : summary;
  },
  defaultFor: ['replicator'],
  size: { height: 248, min: 174 },
  input: instancesInput,
  update: updateInstancesView,
};
