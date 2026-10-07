import { reason, reasonText } from './reasons.mjs';
import { escape, kicker, resultRows, reasonCell, formatDuration } from './view-registry.mjs';

export const assemblyCode = 'ASSEMBLY_STATE_UNREPORTED';
const declared = (config, key) => config && typeof config === 'object' && config[key] != null
  && ['bigint', 'number'].includes(typeof config[key]) ? String(config[key]) : null;

export function assemblyInput(node) {
  const config = node.declaration?.config;
  return { assembly: { capacity: declared(config, 'capacity'), window: declared(config, 'inactivity_timeout'), code: assemblyCode } };
}

const titled = (element, text) => { if (element && element.title !== text) element.title = text; };
const inactivity = declaredMs => {
  const duration = formatDuration(declaredMs);
  return duration ? `<span title="${escape(duration.title)}">${escape(duration.text)} inactivity window</span>`
    : `<span data-reason="UNDECLARED">${escape(`Inactivity window ${reasonText('UNDECLARED').toLowerCase()}`)}</span>`;
};

const capacity = declared => declared == null
  ? `<span data-reason="UNDECLARED">${escape(`Slot capacity ${reasonText('UNDECLARED').toLowerCase()}`)}</span>`
  : `<span>${escape(`— / ${declared} slots in use`)}</span>`;

export function updateAssemblyView(card, node) {
  if (!node?.assembly) return;
  const viewer = card?.querySelector('.node-viewer[data-viewer="assembly"]');
  if (!viewer) return;
  const label = reason(node.assembly.code).label;
  viewer.setAttribute('data-reason', node.assembly.code);
  for (const slot of viewer.querySelectorAll('[data-assembly-unread]')) {
    titled(slot, label);
    slot.setAttribute?.('data-reason', node.assembly.code);
  }
}

export default {
  kind: 'assembly',
  render: n => kicker(n.viewConfig?.heading) +
    resultRows(['Key', 'Parts'], [['—', n.assembly?.code ? reasonCell(n.assembly.code) : '—']]) +
    `<div class="viewer-message facts" data-assembly-unread>${n.assembly ? `${capacity(n.assembly.capacity)}${inactivity(n.assembly.window)}` : ''}</div>`,
  glance: {
    cells: n => {
      const declared = n.assembly?.capacity ?? null;
      return { open: { reason: n.assembly?.code ?? assemblyCode },
        capacity: declared === null ? { reason: 'UNDECLARED', title: `Slot capacity ${reasonText('UNDECLARED').toLowerCase()}` }
          : { count: declared, noun: declared === '1' ? 'slot' : 'slots' } };
    },
    reduced: ['capacity', 'open'],
    names: ['capacity'],
  },
  defaultFor: ['assemble'],
  reads: ['capacity', 'inactivity_timeout'],
  size: { height: 248, min: 174 },
  input: assemblyInput,
  update: updateAssemblyView,
};
