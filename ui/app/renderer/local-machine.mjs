import { lifeLabel, reason, NOT_STANDING, EMPTY } from './reasons.mjs';
import { healthReading, heldText, lastObservedText } from './health-count.mjs';
import { journalCeiling } from './journal-ceiling.mjs';

export function localMachineStatus(set, page, code, seen = null) {
  const reading = healthReading(set, page, code);
  const held = heldText(reading);
  if (code != null) {
    const text = lifeLabel('unobserved');
    return { state: 'unobserved', text, title: lastObservedText(reading, seen) ?? text };
  }
  const ceiling = journalCeiling(page);
  const journal = status => ceiling
    ? { ...status, journal: ceiling.code, text: `${status.text} · ${ceiling.text}`, title: `${status.text} · ${ceiling.text}` }
    : status;
  if (reading.pause) return journal({ state: reading.state, text: held ? `${reading.pause.text} · ${held}` : reading.pause.text });
  if (!reading.held) {
    const value = reason('unobserved');
    return journal({ state: value.code, text: value.label });
  }
  const { state } = reading;
  if (state === EMPTY) return journal({ state, text: lifeLabel(state) });
  return journal({ state, text: held });
}

export function updateLocalMachine(root, set, page, code, seen = null) {
  const row = root?.querySelector('.local-machine > div > span');
  if (!row?.childNodes) return;
  const value = localMachineStatus(set, page, code, seen);
  const text = [...row.childNodes].find(child => child.nodeType === 3);
  if (text) text.nodeValue = ` ${value.text}`;
  row.dataset.health = value.state;
  if (code != null) row.dataset.reason = reason(code).code; else delete row.dataset.reason;
  if (value.journal) row.dataset.journal = value.journal;
  else delete row.dataset.journal;
  row.title = value.title ?? value.text;
  const dot = row.querySelector('.tiny-dot');
  const red = value.state === 'dead' || value.state === NOT_STANDING;
  dot.classList.toggle('green', value.state === 'alive' && !value.journal);
  dot.classList.toggle('amber', !red && (Boolean(value.journal) || value.state === 'paused'));
  dot.classList.toggle('red', red);
}
