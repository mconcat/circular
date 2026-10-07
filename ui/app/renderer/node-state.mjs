import { reason, reasonText, lifeLabel, healthLabel, healthState, healthText, NOT_STANDING } from './reasons.mjs';
import { rateText, rateTitle } from './activity.mjs';

const SLOT_LENGTH = 21;

export const flagWords = (flags) =>
  [flags?.pause && 'Paused', flags?.bypass && 'Bypassed', flags?.mute && 'Muted'].filter(Boolean);

export function nodeState(node, recorded = false, { rate = null, ended = null, pause = null } = {}) {
  if (ended && !recorded) {
    const last = nodeState(node, false, { rate });
    return { state: 'unobserved', code: 'Unobserved', text: [`last observation ${ended.at}`, last.text].join(' · '),
      reasons: last.reasons, color: 'var(--faint)', unavailable: true };
  }
  const flags = node.flags ?? {};
  const labels = flagWords(flags);
  const unknown = !['alive', 'dead', 'running', 'waiting', 'failed', 'backpressure', NOT_STANDING].includes(node.health);
  let state, label, color;
  if (node.health === 'alive' || node.health === 'dead')
    [state, label, color] = [node.health, lifeLabel(node.health), node.health === 'dead' ? 'red' : 'green'];
  else if (node.health === 'waiting') [state, label, color] = ['waiting', 'Waiting', 'amber'];
  else if (node.health === 'failed') [state, label, color] = ['failed', 'Failed', 'red'];
  else if (node.health === 'backpressure') [state, label, color] = ['backpressure', 'Backpressure', 'amber'];
  else if (node.health === NOT_STANDING) [state, label, color] = [NOT_STANDING, lifeLabel(NOT_STANDING), 'red'];
  else if (flags.pause) [state, label, color] = ['paused', 'Paused', 'amber'];
  else if (flags.bypass) [state, label, color] = ['bypassed', 'Bypassed', 'blue'];
  else if (flags.mute) [state, label, color] = ['muted', 'Muted', 'muted'];
  else if (node.health === 'running' && node.activity === 'Idle') [state, label, color] = ['idle', 'Idle', 'faint'];
  else if (node.health === 'running') [state, label, color] = ['alive', 'Alive', 'green'];
  else [state, label, color] = ['unobserved', 'Health unobserved', 'faint'];
  const text = [label, ...labels.filter(value => value !== label)], reasons = [];
  if (node.issue?.code != null) { text.push(reasonText(node.issue.code)); reasons.push(String(node.issue.code)); }
  if (node.health === 'running' && node.activity && node.activity !== label) text.push(node.activity);
  if (unknown) {
    const health = reason('unobserved'), activity = reason('ARRIVAL_UNOBSERVED');
    text.push(health.label); reasons.push(health.code);
    if (!rate?.count && !(node.recordedArrivals > 0n)) { text.push(activity.label); reasons.push(activity.code); }
    if (label === health.label) text.shift();
  }
  const observed = rateText(rate);
  const paused = recorded ? null : pause;
  const phrase = paused ? paused.label
    : observed ?? (node.health === 'running' && node.activity && node.activity !== label ? node.activity : null);
  if (observed && !text.includes(observed)) text.push(observed);
  if (paused) { text.unshift(paused.text); if (color === 'green') color = 'amber'; }
  if (recorded) { state = 'recorded'; color = 'history'; text.unshift('Recorded'); }
  const word = (state[0].toUpperCase() + state.slice(1)).replaceAll('_', ' ');
  const fits = phrase && `${word} · ${phrase}`.length <= SLOT_LENGTH;
  const measured = rateTitle(rate, globalThis.window?.studyTimeFormat);
  if (rate && !rate.whole) reasons.push('DENSITY_UNREAD');
  return { state, code: word, ...(fits ? { phrase } : {}), text: [...text, ...(measured ? [measured] : [])].join(' · '),
    reasons, color: `var(--${color})`, unavailable: unknown };
}

const tint = color => ['green', 'amber', 'red'].find(name => color === `var(--${name})`) ?? null;
const tones = { alive: 'green', dead: 'red', [NOT_STANDING]: 'red', waiting: 'amber', paused: 'amber' };
export function cardFace(node, recorded = false, { rate = null, ended = null, pause = null } = {}) {
  const life = ended ? 'unobserved' : node.health, standing = healthState(node.health);
  const health = ended ? 'unobserved' : node.health === 'waiting' && node.issue ? 'waiting'
    : standing === 'alive' && pause && !recorded ? 'paused' : standing;
  const title = ended ? `last observation ${ended.at} · ${healthText(node)}` : healthText(node);
  const row = nodeState(ended || standing === 'unobserved' ? node : { ...node, health: health === 'waiting' ? health : standing },
    recorded, { rate, ended, pause });
  const tone = tones[health] ?? null;
  return {
    life,
    flags: flagWords(node.flags).join(' · '),
    dot: { color: tone, health, title, unavailable: health === 'unobserved' },
    row: { code: row.code, ...(row.phrase ? { phrase: row.phrase } : {}), text: row.text, reasons: row.reasons, state: row.state, fill: row.color, color: tint(row.color), unavailable: row.unavailable, health,
      band: node.issue && !ended ? tone : null },
  };
}
