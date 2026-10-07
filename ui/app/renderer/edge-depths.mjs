import { edgeDepths, edgeDepthsItemFromValue } from '@circular/client';
import { openSubscription } from '@circular/client/subscription';
import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { identity, domId } from './query.mjs';
import { accepted, credited } from './records.mjs';
import { declaredEdge } from './activity.mjs';
import { inSpace } from './reasons.mjs';

const depthRow = value => {
  const row = edgeDepthsItemFromValue(value);
  return row.edge === undefined
    ? { actor: identity(actorIdentityFromValue(row.actor)), code: inSpace('Subscription', row.code) }
    : { edge: identity(row.edge), depth: row.depth.value, queued: row.queued.value, capacity: row.capacity?.value ?? null };
};

let held;
const GONE = Symbol('superseded');
export const depthsOf = session => held?.session === session ? held.value : undefined;
export function askDepths(session, answered) {
  if (!session) return;
  if (held?.session !== session) held = { session, value: undefined, reading: false, again: false, letGo: undefined };
  const reading = held;
  reading.answered = answered;
  if (reading.reading) { reading.again = true; reading.letGo?.(); return; }
  reading.reading = true;
  void (async () => {
    try {
      do { reading.again = false; await ask(reading); } while (reading.again && held === reading);
    } finally { reading.reading = false; }
  })();
}

async function ask(reading) {
  const answer = { rows: new Map(), ended: new Map(), open: true };
  const show = () => { if (held === reading) { reading.value = answer; reading.answered?.(answer); } };
  let subscription, drawn = false, superseded;
  const gone = new Promise(resolve => { superseded = () => resolve(GONE); });
  reading.letGo = force => { if (force || drawn) superseded(); };
  try {
    subscription = await openSubscription(reading.session, { target: edgeDepths.name, args: null, initialCredit: 1n });
    accepted(subscription.ack);
    for (;;) {
      const next = subscription.receive(Infinity);
      next.catch(() => {});
      const frame = await Promise.race([next, gone]);
      if (frame === GONE) break;
      const ending = subscription.ended;
      if (ending) {
        if (ending.reason !== 'Complete') answer.code = inSpace('Subscription', ending.code);
        break;
      }
      if (!frame || frame.arm === 'RetentionComplete') continue;
      const row = depthRow(frame.payload);
      if (row.edge === undefined) answer.ended.set(row.actor, row.code); else answer.rows.set(row.edge, row);
      show();
      drawn = frame.pending_after?.value === 0n;
      if (drawn && reading.again) break;
      await credited(subscription, 1n);
    }
  } catch (error) {
    answer.code = error.code ?? 'READ_UNAVAILABLE';
  } finally {
    reading.letGo = undefined;
    if (subscription && !subscription.ended) subscription.release();
    answer.open = false;
    show();
  }
}
export function forgetDepths() {
  held?.letGo?.(true);
  held = undefined;
}

function wireAnswer(value, edge) {
  const row = value.rows.get(declaredEdge(edge));
  if (row) return row;
  const ended = value.ended.get(edge.to);
  if (ended !== undefined) return { code: ended };
  return { code: (!value.open && value.code) || 'MAILBOX_DEPTH_UNANSWERED' };
}

export function pools(value, edges) {
  if (!value) return [];
  return (edges ?? []).flatMap(edge => {
    const row = wireAnswer(value, edge);
    return row.code === undefined && row.depth + row.queued > 0n
      ? [{ wire: domId(edge.id), depth: Number(row.depth), queued: Number(row.queued), capacity: row.capacity === null ? null : Number(row.capacity) }]
      : [];
  });
}

export function inletDepths(value, edges, actor, { past = false } = {}) {
  const into = (edges ?? []).filter(edge => edge.to === actor);
  if (!into.length) return { rows: [] };
  if (past) return { code: 'MAILBOX_DEPTH_UNRECORDED' };
  if (!value) return { code: 'MAILBOX_DEPTH_UNOBSERVED' };
  return { rows: into.map(edge => {
    const shown = wireAnswer(value, edge);
    return { wire: domId(edge.id), from: edge.from, out: edge.out, in: edge.in,
      ...(shown.code === undefined ? { depth: shown.depth, queued: shown.queued, capacity: shown.capacity } : { code: shown.code }) };
  }) };
}
