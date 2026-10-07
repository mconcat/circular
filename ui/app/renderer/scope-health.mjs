import { lifeLabel, reason, NOT_STANDING, EMPTY } from './reasons.mjs';
import { countsText, healthReading, heldText, lastObservedText } from './health-count.mjs';

export function scopeHealth(set, page, code, seen = null) {
  const reading = healthReading(set, page, code), { state, held } = reading;
  if (reading.pause) return { state, text: reading.pause.text,
    title: held ? `${reading.pause.text} · ${heldText(reading)}` : reading.pause.text };
  if (code != null) {
    const text = lifeLabel('unobserved');
    return { state, text, code: reason(code).code, title: lastObservedText(reading, seen) ?? text };
  }
  const mixed = held && Object.values(held).filter(count => count > 0).length > 1;
  const text = mixed
    ? ['alive', 'dead', ...(held[NOT_STANDING] ? [NOT_STANDING] : [])].map(state => `${lifeLabel(state)} ${held[state]}`).join(' · ')
    : lifeLabel(state);
  return { state, text, title: state === EMPTY ? reason('CANVAS_EMPTY').label : held ? countsText(held, lifeLabel) : text };
}
