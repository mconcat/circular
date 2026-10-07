import { escape, kicker, PROSE } from './view-registry.mjs';
import { viewRecords, distribute, isReceived } from './arrivals.mjs';
import { rowText } from './value-text.mjs';
import { observeActors } from './scene.mjs';
import { codeText, inSpace, reasonText } from './reasons.mjs';
import { readActorEvents } from './session.mjs';

export async function readBoundaryArrivals(session, graph, observedPage, mine) {
  const drawn = new Set(graph.nodes.filter(mine).map(node => node.id));
  if (!drawn.size) return graph;
  const mark = (read, boundaryDiagnostic) => observeActors(read, () => ({boundaryDiagnostic}), drawn);
  if (observedPage) return mark(graph, null);
  try { return mark(distribute(graph, await readActorEvents(session), drawn), null); }
  catch (error) { return mark(distribute(graph, {items: []}, drawn), inSpace('Query', error.code ?? 'READ_UNAVAILABLE')); }
}

export function boundaryInput(node) {
  const latest = viewRecords(node).filter(isReceived)
    .reduce((last, row) => !last || row.index > last.index ? row : last, null);
  return {boundaryDiagnostic: node.boundaryDiagnostic ?? null, boundaryRecorded: node.boundaryDiagnostic == null && Boolean(latest),
    boundaryMessage: node.boundaryDiagnostic != null ? reasonText(node.boundaryDiagnostic) : (latest
      ? rowText(node, latest, 'arrivals').text : reasonText('ARRIVAL_UNOBSERVED'))};
}

export function updateBoundaryView(card, node) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="boundary"]');
  if (!viewer) return;
  const label = viewer.querySelector('.viewer-boundary');
  const declared = node.config?.label;
  if (label) {
    const named = typeof declared === 'string' && declared.trim();
    label.textContent = named ? declared : reasonText('UNDECLARED');
    if (named) label.removeAttribute?.('data-reason'); else label.setAttribute?.('data-reason', 'UNDECLARED');
  }
  const message = viewer.querySelector('.viewer-message');
  if (message) {
    message.textContent = node.boundaryMessage;
    const code = node.boundaryDiagnostic ?? null;
    message.setAttribute('aria-disabled', String(code != null));
    const reason = code != null ? codeText(code) : node.boundaryRecorded ? null : 'ARRIVAL_UNOBSERVED';
    if (reason) message.setAttribute('data-reason', reason); else message.removeAttribute?.('data-reason');
    message.title = code != null ? reasonText(code) : node.boundaryRecorded ? node.boundaryMessage : '';
    message.toggleAttribute?.('data-recorded', Boolean(node.boundaryRecorded));
  }
}

export default {
  kind: 'boundary',
  render: n => {
    const message = `<div class="viewer-message"${n.boundaryRecorded ? ` data-recorded title="${escape(n.boundaryMessage)}"` : ''}>${escape(n.boundaryMessage ?? reasonText('ARRIVAL_UNOBSERVED'))}</div>`;
    return `${kicker(n.viewConfig?.heading)}<div class="viewer-boundary" style="${PROSE}">${escape(n.config?.label)}</div>${message}`;
  },
  glance: {
    cells: n => {
      const label = typeof n.config?.label === 'string' && n.config.label.trim() ? n.config.label : null;
      return { label: label === null ? { reason: 'UNDECLARED' } : { words: label },
        refused: n.boundaryDiagnostic != null ? { reason: n.boundaryDiagnostic } : null };
    },
    reduced: ['label', 'refused'],
    names: ['label'],
  },
  defaultFor: [],
  reads: ['label'],
  size: { height: 208, min: 180 },
  input: boundaryInput,
  update: updateBoundaryView,
  tick: 'replace',
  read: readBoundaryArrivals,
  page: node => node.boundaryDiagnostic ? { boundaryDiagnostic: null } : undefined,
};
