import { escape, ONE_LINE } from './view-registry.mjs';
import { viewRecords, isReceived, recordedArrivalsText } from './arrivals.mjs';
import { rowText } from './value-text.mjs';
import { reasonText } from './reasons.mjs';

const unobserved = 'WRITE_UNOBSERVED';
const completionUnrecorded = 'WRITE_COMPLETION_UNRECORDED';
const portUnrecorded = 'PORT_UNRECORDED';
const undeclared = 'UNDECLARED';
const clock = seconds => [Math.floor(seconds / 3600) % 24, Math.floor(seconds / 60) % 60, Math.floor(seconds) % 60]
  .map(part => String(part).padStart(2, '0')).join(':');
const byteText = bytes => bytes >= 1024 ? `${(bytes / 1024).toFixed(1)} kB` : `${bytes} B`;
const headingOf = config => {
  const path = config?.path;
  if (typeof path !== 'string' || !path) return reasonText(undeclared);
  return path.split(/[\\/]/).at(-1) || path;
};
export function notebookInput(node) {
  const arrivals = viewRecords(node).filter(isReceived)
    .sort((a, b) => a.observed_at_ms === b.observed_at_ms ? 0 : a.observed_at_ms < b.observed_at_ms ? -1 : 1);
  const latest = arrivals.at(-1);
  const heading = headingOf(node.declaration?.config ?? node.config);
  const text = latest === undefined ? null : rowText(node, latest, 'arrivals').text;
  const bytes = text === null ? null : latest.body instanceof Uint8Array ? latest.body.length
    : new TextEncoder().encode(typeof latest.body === 'string' ? latest.body : text).length;
  const at = latest === undefined ? null : Number(latest.observed_at_ms ?? 0) / 1000;
  const notebook = {
    observed: latest !== undefined,
    heading, bytes,
    lines: text === null ? [reasonText(unobserved)] : text.split(/\r?\n/),
    bytesText: bytes === null ? '—' : byteText(bytes),
    detail: latest === undefined
      ? reasonText(unobserved)
      : `${clock(at)} · ${latest.port ?? reasonText(portUnrecorded)} · ${recordedArrivalsText(node)} · ${reasonText(completionUnrecorded)}`,
  };
  return {notebook, preview: {heading, lines: notebook.lines}};
}

function bindPath(element, path) {
  if (!element) return;
  const full = typeof path === 'string' ? path : '—';
  element.title = full;
  element.setAttribute('aria-label', full);
  element.textContent = full;
  if (!element.clientWidth || element.scrollWidth <= element.clientWidth) return;
  const split = Math.max(full.lastIndexOf('/'), full.lastIndexOf('\\'));
  if (split < 0) return;
  const prefix = Array.from(full.slice(0, split));
  const suffix = full.slice(split);
  let low = 0, high = prefix.length;
  while (low < high) {
    const middle = Math.ceil((low + high) / 2);
    element.textContent = prefix.slice(0, middle).join('') + '…' + suffix;
    if (element.scrollWidth <= element.clientWidth) low = middle;
    else high = middle - 1;
  }
  element.textContent = prefix.slice(0, low).join('') + '…' + suffix;
}

export function updateNotebookView(card, node) {
  if (!node) return;
  const viewer = card?.querySelector('.node-viewer[data-viewer="notebook"]');
  if (!viewer) return;
  const observation = node.notebook;
  const observed = Boolean(observation?.observed);
  viewer.setAttribute('aria-disabled', String(!observed));
  viewer.title = observation?.detail ?? reasonText(unobserved);
  viewer.setAttribute('data-reason', observed ? completionUnrecorded : unobserved);
  const bytes = viewer.querySelector('[data-live="bytes"]');
  if (bytes) {
    bytes.textContent = observation?.bytesText ?? '—';
    bytes.title = observed
      ? `${observation.bytes} B · UTF-8 length of the last observed arrival body · ${reasonText(completionUnrecorded)}`
      : reasonText(unobserved);
    bytes.setAttribute('aria-disabled', String(!observed));
    bytes.setAttribute('data-reason', observed ? completionUnrecorded : unobserved);
  }
  const heading = viewer.querySelector('.viewer-document > strong');
  if (heading) {
    heading.textContent = observation?.heading ?? headingOf(node.config);
    heading.title = typeof node.config?.path === 'string' ? node.config.path : reasonText(undeclared);
    if (typeof node.config?.path !== 'string') heading.setAttribute?.('data-reason', undeclared); else heading.removeAttribute?.('data-reason');
  }
  const rows = [...(viewer.querySelectorAll?.('.viewer-document > div') ?? [])];
  const lines = observation?.lines ?? [reasonText(unobserved)];
  for (const [i, row] of rows.entries()) {
    row.style.display = i < lines.length ? '' : 'none';
    if (i < lines.length) row.lastElementChild.textContent = lines[i];
  }
  const path = viewer.querySelector('.viewer-file-path');
  bindPath(path, node.config?.path);
  const bottom = viewer.getBoundingClientRect?.().bottom ?? 0;
  if (bottom && path?.getBoundingClientRect) {
    for (let i = rows.length - 1; i >= 0 && path.getBoundingClientRect().bottom > bottom; i--) {
      rows[i].style.display = 'none';
    }
  }
}

export default {
  kind: 'notebook',
  render: n => `<div class="viewer-kicker">${escape(n.viewConfig?.heading)}<span><span data-live="bytes">${escape(n.notebook?.bytesText ?? '—')}</span></span></div><div class="viewer-document"><strong style="${ONE_LINE}">${escape(n.preview?.heading || headingOf(n.config))}</strong>${(n.preview?.lines || [reasonText(unobserved)]).map((s, i) => `<div><small>${String(i + 1).padStart(2, '0')}</small><span>${escape(s)}</span></div>`).join('')}</div><div class="viewer-file-path">${escape(n.config?.path)}</div>`,
  glance: {
    cells: n => {
      const observation = n.notebook, path = typeof n.config?.path === 'string' ? n.config.path : null;
      return { file: path === null ? { reason: undeclared } : { reading: observation?.heading ?? headingOf(n.config), title: path },
        lines: observation?.observed ? { words: observation.lines.join('\n'), title: observation.detail }
          : { reason: unobserved } };
    },
    reduced: ['file', 'lines'],
    names: ['file'],
  },
  defaultFor: ['file'],
  reads: ['path'],
  size: { height: 248, min: 210 },
  input: notebookInput,
  update: updateNotebookView,
  tick: 'replace',
};
