import { codeText, reasonText } from './reasons.mjs';
export function bindProductUtility(root, page, diagnostic) {
  const code = page ? '' : codeText(diagnostic ?? 'READ_UNAVAILABLE');
  const badge = root.querySelector?.('#approval-count');
  if (badge) {
    badge.textContent = page ? String(page.items.filter(row => row.state === 1n).length) : '—';
    badge.hidden = false;
    badge.title = code ? reasonText(diagnostic ?? 'READ_UNAVAILABLE') : '';
    if (code) badge.setAttribute?.('data-reason', code); else badge.removeAttribute?.('data-reason');
  }
  for (const button of root.querySelectorAll('.product-utility [data-action="approvals"]')) {
    button.disabled = Boolean(code);
    button.title = code ? reasonText(diagnostic ?? 'READ_UNAVAILABLE') : '';
    if (code) button.setAttribute?.('data-reason', code); else button.removeAttribute?.('data-reason');
  }
}
