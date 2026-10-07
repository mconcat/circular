import { inSpace } from './reasons.mjs';
import { identity } from './query.mjs';
import { readTimelineAt } from './session.mjs';

const seconds = ms => Number(ms) / 1000;

export function realtimePace(value) {
  const [whole, fraction = ''] = String(value).split('.');
  let num = BigInt(whole + fraction), den = 10n ** BigInt(fraction.length);
  const gcd = (a, b) => (b === 0n ? a : gcd(b, a % b));
  const g = gcd(num, den);
  num /= g; den /= g;
  return { kind: 'Realtime', num, den };
}

export const askedMs = t => BigInt(Math.max(0, Math.round(t * 1000)));

const refused = result => {
  const answered = result?.diagnostic ?? result?.diagnostics?.[0], code = inSpace('ReplayControl', answered?.code);
  return code === undefined ? undefined : { ...answered, code };
};
const same = (a, b) => a != null && b != null && identity(a) === identity(b);

export function lensTransport(session, source) {
  let lens = null;
  let code = null;
  let turn = Promise.resolve(), waiting = null;
  const reached = () => Math.max(0, ...(globalThis.window?.STUDY?.journal ?? []).map(row => row.at));
  const position = machine => lens === null ? machine.head
    : lens.named ? lens.resolved
    : Math.max(lens.resolved ?? 0, reached());
  const asked = machine => lens?.named && lens.asked !== undefined ? lens.asked : position(machine);

  const unavailable = () => source.observedHead > 0 ? null : 'TIME_UNRECORDED';

  const refuse = (machine, refusal) => {
    code = refusal.code;
    machine.endDrag();
    if (waiting?.drag) waiting.dropped = true;
    source.refuse(refusal);
    machine.refresh();
    return false;
  };
  const ended = machine => {
    const held = lens;
    if (held === null) return;
    lens = null;
    machine?.show({ mode: 'live' });
    source.lensEnded?.(held.handle);
  };
  const verbs = {
    'replay.start': body => session().replay.start(body),
    'replay.rewind': body => lens.handle.rewind(body),
    'replay.end': async (_body, machine) => {
      const result = await lens.handle.end();
      ended(machine);
      return result;
    },
  };
  const request = async (machine, planned) => {
    let step;
    try { step = await planned(); } catch (error) { return refuse(machine, { code: error?.code ?? 'READ_UNAVAILABLE' }); }
    if (!step) return false;
    const [verb, request, next] = step;
    let result;
    try { result = await verbs[verb](request, machine); }
    catch (error) { result = { status: 'rejected', diagnostic: { code: error.code ?? 'READ_UNAVAILABLE' } }; }
    if (result?.status !== 'accepted') return refuse(machine, refused(result) ?? { code: 'QUERY_RESULT_REQUIRED' });
    code = null;
    next(result.value);
    return true;
  };
  const ask = (machine, planned) => (turn = turn.then(() => request(machine, planned)));
  const settle = (machine, held, seek = true, horizon) => {
    lens = { handle: lens?.handle, ...held };
    machine.show({ mode: held.pace?.kind === 'Realtime' ? 'replay' : 'history', seek, ...(horizon === undefined ? {} : { horizon }) });
  };
  const paced = (held, pace) => ({ ...held, pace, named: false });

  const seekTo = (machine, t) => async () => {
    if (t >= machine.head && lens !== null) return ['replay.rewind', { pace: 'Free' }, () => { settle(machine, paced(lens, 'Free')); source.lensMoved?.(lens.handle); }];
    if (lens?.named && lens.asked !== undefined && askedMs(t) === askedMs(lens.asked)) return null;
    const answer = await readTimelineAt(session(), askedMs(t));
    const held = { at: answer.target, resolved: seconds(answer.resolved_ms), asked: Math.max(0, t), pace: 'Paused', named: true };
    const opened = (placed, pace, named) => handle => {
      settle(machine, { ...placed, handle, pace, named }, true, machine.head);
      source.lensOpened?.(handle);
    };
    if (t >= machine.head) return ['replay.start', { from: answer.target, pace: 'Free' }, opened(held, 'Free', false)];
    if (lens === null) return ['replay.start', { from: answer.target, pace: 'Paused' }, opened(held, 'Paused', true)];
    if (same(answer.target, lens.at) && lens.named) { lens = { ...lens, asked: held.asked }; return null; }
    if (held.resolved > position(machine)) return ['replay.rewind', { pace: { kind: 'Step', upto: answer.target } }, () => { settle(machine, held); source.lensMoved?.(lens.handle); }];
    return ['replay.rewind', { to: answer.target, pace: 'Paused' }, () => settle(machine, held)];
  };
  return {
    attach(machine) {
      machine.archive.frames = [];
      machine.archive.markers = [];
    },
    position,
    asked,
    seek(machine, t) {
      const refusal = unavailable();
      if (refusal) { source.refuse({ code: refusal }); return undefined; }
      const drag = machine.dragging === true;
      if (waiting) { waiting.t = t; waiting.drag ||= drag; return waiting.answered; }
      const entry = waiting = { t, drag };
      entry.answered = turn = turn.then(() => {
        if (waiting === entry) waiting = null;
        return entry.dropped ? false : request(machine, seekTo(machine, entry.t));
      });
      return entry.answered;
    },
    resume(machine) {
      if (lens === null) return;
      return ask(machine, () => {
        if (lens === null) return null;
        if (machine.mode === 'replay') return ['replay.rewind', { pace: 'Paused' }, () => settle(machine, paced(lens, 'Paused'))];
        const pace = realtimePace(machine.speed);
        return ['replay.rewind', { pace }, () => settle(machine, paced(lens, pace), false)];
      });
    },
    speedChanged(machine) {
      if (lens === null || machine.mode !== 'replay') return machine.refresh();
      return ask(machine, () => {
        if (lens === null || machine.mode !== 'replay') return null;
        const pace = realtimePace(machine.speed);
        return ['replay.rewind', { pace }, () => settle(machine, paced(lens, pace), false)];
      });
    },
    live(machine) {
      if (lens === null) return machine.show({ mode: 'live' });
      return ask(machine, () => lens === null ? null : ['replay.end', undefined, () => {}]);
    },
    forget(machine) { if (lens !== null) ended(machine); },
    get lens() { return lens; },
    get code() { return code; },
    unavailable,
  };
}
