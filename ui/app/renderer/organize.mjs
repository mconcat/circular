import { organized } from './layout.mjs';
import { reason, codeText } from './reasons.mjs';

export function organizeTargets(nodes, wires, origin, metrics) {
  return organized(nodes, wires, origin, metrics, new Map(nodes.map(node => [node.id, { x: node.x, y: node.y }])));
}

export function bindOrganize(doc, app, { action, availability }) {
  function refresh() {
    const control = doc.querySelector('[data-action="organize"]');
    if (!control) return;
    const code = availability();
    control.disabled = code !== undefined;
    control.title = code === undefined ? 'Organize' : reason(code).label;
    if (code === undefined) delete control.dataset.reason;
    else control.dataset.reason = codeText(code);
  }
  doc.addEventListener('keydown', event => {
    if (event.key?.toLowerCase() !== 'l' || event.metaKey || event.ctrlKey || event.altKey) return;
    const target = event.target;
    if (/INPUT|TEXTAREA|SELECT/.test(target?.tagName ?? '') || target?.isContentEditable) return;
    if (doc.querySelector('dialog[open]') || app.state.view !== 'canvas') return;
    event.preventDefault();
    action();
  });
  return { refresh };
}
