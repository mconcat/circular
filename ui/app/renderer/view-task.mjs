import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { identity } from './query.mjs';
import { escape, words, writeWords, kicker, formatTime, PROSE } from './view-registry.mjs';
import { viewRecords, causingArrival, isOwnOutcome, isReceived, isEmission, primaryOutlet, recordedArrivalsText, wireSource } from './arrivals.mjs';
import { declaredEdge } from './activity.mjs';
import { rowText } from './value-text.mjs';
import { reasonText } from './reasons.mjs';
import { kindText, causedBy } from './record-words.mjs';

export const taskRows = 3;

const turnUnobserved = 'TURN_UNOBSERVED';
const turnPreprocessed = 'TURN_PREPROCESSED';
const turnWireUnread = 'TURN_WIRE_UNREAD';
const stepUnobserved = 'STEP_UNOBSERVED';
const resultUnobserved = 'RESULT_UNOBSERVED';
const meaningUndeclared = 'TASK_MEANING_UNDECLARED';
const pageUnavailable = reasonText('READ_UNAVAILABLE');

export function taskPage(node, page) {
  return { taskFacts: taskFacts(page?.items ?? [], node) };
}

function taskFacts(items, node) {
  const id = node.id, answers = primaryOutlet(node);
  const own = row => identity(actorIdentityFromValue(row.actor)) === id;
  const here = items.filter(row => row.kind === 'actor_arrival' && own(row));
  return { observed: items.some(row => (row.kind === 'actor_arrival' || isEmission(row))
    && typeof row.observed_at_ms === 'bigint'),
    turns: here.filter(row => isReceived(row) && typeof row.port === 'string'),
    steps: here.filter(isOwnOutcome),
    results: items.filter(row => isEmission(row) && own(row) && typeof row.port === 'string'
      && (answers === undefined || row.port === answers)) };
}

const seconds = row => Number(row.observed_at_ms) / 1000;
const stamp = row => ({ at: seconds(row), at_ms: String(row.observed_at_ms) });
const latest = (rows, t) => rows.filter(row => row.at <= t)
  .reduce((best, row) => !best || row.at >= best.at ? row : best, null);

const absence = { code: resultUnobserved, text: reasonText(resultUnobserved) };

function received(node, row, edges) {
  if (wireSource(row) === null) return { ...rowText(node, row, 'arrivals'), withheld: false };
  const wire = (edges ?? []).find(edge => edge.to === node.id && declaredEdge(edge) === identity(row.edge));
  const steps = wire?.attributes?.preprocess ?? [];
  if (wire && !steps.length) return { ...rowText(node, row, 'arrivals'), withheld: false };
  return { text: '', code: wire ? turnPreprocessed : turnWireUnread, withheld: true };
}

export function taskInput(node, { graph }) {
  const facts = node.taskFacts;
  const observed = facts?.observed ? recordedArrivalsText(node) : pageUnavailable;
  const turns = (facts?.turns ?? []).map(row => ({ ...stamp(row), port: row.port, ...received(node, row, graph?.edges) }));
  const results = (facts?.results ?? []).map(row => ({ ...stamp(row), port: row.port,
    ...rowText(node, row, 'emitted') }));
  const answerAt = step => !step.ok ? step.at
    : results.filter(result => result.at >= step.at).reduce((first, result) => Math.min(first, result.at), Infinity);
  const steps = (facts?.steps ?? []).map(row => ({ ...stamp(row), kind: row.body.kind, ok: row.body.ok,
    ...stepCause(row, viewRecords(node)), attempts: sequence(row.body.attempts) }))
    .map(step => ({ ...step, at: answerAt(step) })).filter(step => step.at !== Infinity);
  const turn = latest(turns, Infinity), result = latest(results, Infinity), step = latest(steps, Infinity);
  return { taskTurns: turns, taskResults: results, taskSteps: steps, taskObserved: observed,
    preview: { task: turn ? title(turn) : reasonText(turnUnobserved), text: turn ? turn.text : '',
      step: step ? settled(step) : reasonText(stepUnobserved),
      ...(result ? { result: result.text } : {}) } };
}

const sequence = value => typeof value?.value === 'bigint' ? String(value.value)
  : typeof value === 'bigint' ? String(value) : null;
const stepCause = (row, column) => {
  const cause = causingArrival(row, column);
  return { cause: cause.row ? String(cause.row.index) : null, causeCode: cause.code ?? null };
};
const recordedAt = row => `recorded ${formatTime(row.at_ms)?.title ?? reasonText('TIME_UNRECORDED')}`;
const title = turn => `${turn.port} · ${formatTime(turn.at_ms)?.text ?? reasonText('TIME_UNRECORDED')}`;
const resultTitle = result => `${result.port} · ${recordedAt(result)}`;
const settled = step => [step.ok ? 'Last step succeeded' : 'Last step failed',
  step.attempts ? `after ${step.attempts} attempts` : null].filter(Boolean).join(' ');
const settledTitle = step => [kindText(step.kind),
  step.cause !== null ? causedBy(step.cause) : step.causeCode ? reasonText(step.causeCode) : null,
  recordedAt(step)].filter(Boolean).join(' · ');

export function updateTaskView(card, node, t = globalThis.window?.StudyApp?.displayTime?.() ?? Infinity) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="task"]');
  if (!viewer) return;
  const projected = Boolean(node);
  const observed = (projected && node.taskObserved) || pageUnavailable;
  const turn = projected ? latest(node.taskTurns ?? [], t) : null;
  const result = projected ? latest(node.taskResults ?? [], t) : null;
  const step = projected ? latest(node.taskSteps ?? [], t) : null;
  const detail = code => code ? `${reasonText(code)} · ${observed}` : observed;
  viewer.setAttribute('aria-disabled', String(!turn));
  viewer.title = detail(turn ? null : turnUnobserved);
  if (!turn) viewer.setAttribute('data-reason', turnUnobserved); else viewer.removeAttribute?.('data-reason');
  const write = (selector, text, code, unobserved = Boolean(code), recorded = null, asWords = false) => {
    const target = viewer.querySelector(selector);
    if (!target) return;
    if (asWords) writeWords(target, text); else target.textContent = text;
    target.setAttribute('aria-disabled', String(unobserved));
    target.title = [detail(code), recorded].filter(Boolean).join(' · ');
    if (code) target.setAttribute('data-reason', code); else target.removeAttribute?.('data-reason');
  };
  write('.task-content h3', turn ? title(turn) : reasonText(turnUnobserved),
    turn ? meaningUndeclared : turnUnobserved, !turn, turn ? recordedAt(turn) : null);
  const heading = viewer.querySelector('.task-content h3');
  if (heading && turn?.withheld) heading.title = `${reasonText(turn.code)} · ${heading.title}`;
  write('.task-content p', turn ? turn.text : '', turn ? turn.code : turnUnobserved, !turn || turn.withheld, null, true);
  write('.task-content blockquote', result ? result.text : absence.text,
    result ? result.code : absence.code, !result, result ? resultTitle(result) : null, true);
  write('.task-evidence', step ? settled(step) : reasonText(stepUnobserved),
    step ? null : stepUnobserved, !step, step ? settledTitle(step) : null);
}

export default {
  kind: 'task',
  render: n => kicker(n.viewConfig?.heading) +
    `<div class="task-content"><h3>${escape(n.preview?.task || reasonText(turnUnobserved))}</h3><p style="${PROSE}">${words(n.preview?.text ?? '')}</p>${n.preview?.result ? `<blockquote style="${PROSE}">${words(n.preview.result)}</blockquote>` : ''}<div class="task-evidence">${escape(n.preview?.step ?? reasonText(stepUnobserved))}</div></div>`,
  glance: {
    cells: (n, t = Infinity) => {
      const turn = latest(n.taskTurns ?? [], t), result = latest(n.taskResults ?? [], t);
      return { turn: turn ? { reading: title(turn), title: recordedAt(turn) } : { reason: turnUnobserved },
        result: result ? { words: result.text, title: resultTitle(result), code: result.code } : { reason: absence.code } };
    },
    reduced: ['turn', 'result'],
    names: ['result'],
  },
  defaultFor: ['agent'],
  reads: [],
  size: { height: 248, min: 174 },
  rows: taskRows,
  input: taskInput,
  update: updateTaskView,
  tick: 'replace',
  page: taskPage,
};
