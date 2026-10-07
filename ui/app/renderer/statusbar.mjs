import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { lifecycleLabel, lifeLabel, reasonText, codeText, NOT_STANDING, EMPTY } from './reasons.mjs';
import { healthReading, lastObservation } from './health-count.mjs';
import { journalCeiling } from './journal-ceiling.mjs';
import { identity } from './query.mjs';
import { formatReading } from './view-registry.mjs';

export function deadLetterText(members, problems) {
  const count = (problems?.rows ?? []).filter(row => members.has(identity(actorIdentityFromValue(row.actor)))).length;
  if (!count) return null;
  return `${formatReading(count).text} dead ${count === 1 ? 'letter' : 'letters'}`;
}

export function statusbarText(set, page, code, problems, said = true, seen = null) {
  const unknown = reasonText('unobserved');
  const diagnostic = code == null || !said ? null : { text: reasonText(code), code: codeText(code), said: true };
  if (!page) return [diagnostic, { text: unknown, code: 'unobserved' }].filter(Boolean);
  const { held: counts, last: said_ } = healthReading(set, page, code);
  const standing = reasonText(NOT_STANDING);
  const lifecycle = page.anchor?.lifecycle;
  const ceiling = journalCeiling(page);
  const lifecycleText = lifecycle && lifecycle !== 'running' && lifecycleLabel(lifecycle);
  const letters = deadLetterText(set?.members ?? new Set(), problems);
  const details = code != null || lifecycleText || ceiling || letters || said_ !== 'alive';
  const facts = [lifecycleText,
    ceiling && { text: ceiling.text, code: ceiling.code },
    ...(!counts ? [{ text: unknown, code: 'unobserved' }] : said_ === EMPTY ? [{ text: lifeLabel(EMPTY), code: EMPTY }]
      : [`${formatReading(counts.alive).text} ${counts.alive === 1 ? 'actor' : 'actors'} alive`,
        details && `${formatReading(counts.dead).text} dead`]),
    letters,
    counts?.[NOT_STANDING] && { text: `${formatReading(counts[NOT_STANDING]).text} ${standing}`, code: NOT_STANDING },
    counts?.unobserved && { text: `${formatReading(counts.unobserved).text} ${unknown}`, code: 'unobserved' }].filter(Boolean);
  if (code == null || !counts) return [diagnostic, ...facts].filter(Boolean);
  const held = fact => typeof fact === 'string' ? { text: fact, held: true } : { ...fact, held: true };
  return [diagnostic, { text: lastObservation(seen) }, ...facts.map(held)].filter(Boolean);
}

export function updateStatusbar(root, text, workspace, health) {
  const status = root.querySelector('#footer-status');
  const dot = root.querySelector('.statusbar > div:first-child > .tiny-dot');
  if (health && dot?.dataset) dot.dataset.health = health;
  if (status) {
    if (Array.isArray(text)) {
      const facts = text.map(fact => typeof fact === 'string' ? { text: fact } : fact), doc = status.ownerDocument;
      if (doc?.createElement) status.replaceChildren(...facts.map(fact => {
        const slot = Object.assign(doc.createElement('span'), { textContent: fact.text });
        if (fact.code) slot.dataset.reason = fact.code;
        if (fact.held) slot.dataset.held = '';
        if (fact.title) slot.title = fact.title;
        return slot;
      }));
      else status.textContent = facts.map(fact => fact.text).join(' ');
      const code = facts.find(fact => fact.said)?.code;
      if (code) status.setAttribute?.('data-reason', code); else status.removeAttribute?.('data-reason');
    } else status.textContent = text;
  }
  const name = root.querySelector('.statusbar > div:first-child > span:last-child');
  if (name && workspace != null) name.textContent = workspace;
}
