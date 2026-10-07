import { fault } from './query.mjs';
import { inSpace } from './reasons.mjs';
import { openRecords } from './session.mjs';

export async function closed(subscription) {
  let answer;
  try { answer = await subscription.close(); }
  catch (error) { subscription.release(); throw error; }
  if (answer !== null) accepted(answer);
}

export function accepted(ack, credit = false) {
  if (credit ? Array.isArray(ack) && ack[0] === 1n : ack === 1n) return;
  throw fault(ack?.[0] === 2n ? inSpace('Subscription', ack[1].code) : 'SUBSCRIPTION_ANSWER_UNEXPECTED');
}
export async function credited(subscription, frames) {
  const ack = await subscription.credit(frames);
  if (ack !== null) accepted(ack, true);
}

export function followRecords(session, scope, onRecord, onStatus, onObserving, { target, credit = 1n, liveOnly = false, lens } = {}) {
  let stopping = false, observing = false;
  const done = (async () => {
    let subscription, opened = false, ended = false, faulted = false, pass, failure, terminal, why, draining = false, past = false;
    let woken = [];
    const drain = async () => {
      draining = true;
      try {
        while (woken.length && !stopping) {
          const batch = woken;
          woken = [];
          await onRecord(batch);
        }
      } finally { draining = false; }
    };
    const start = () => { if (!draining) pass = drain().catch(error => { failure = error; }); };
    try {
      subscription = await openRecords(session, scope, credit, target, lens);
      accepted(subscription.ack);
      opened = true; observing = !stopping;
      if (observing) onObserving?.();
      let sweeping = false, swept = 0n;
      while (!stopping && !ended) {
        const result = { frame: await subscription.receive(...(sweeping ? [0] : [])), ended: subscription.ended };
        if (result.ended) {
          ended = true; observing = false; terminal = inSpace('Subscription', result.ended.code); why = result.ended.reason;
          break;
        }
        if (!result.frame) {
          if (sweeping) {
            sweeping = false;
            const taken = swept; swept = 0n;
            if (!stopping) await credited(subscription, taken);
            start();
          }
          continue;
        }
        if (result.frame.arm === 'RetentionComplete') {
          past = true;
          if (!stopping) await credited(subscription, 1n);
          continue;
        }
        if (stopping) break;
        const held = liveOnly && result.frame.origin === 'Retained' && !past;
        if (!held) woken.push(result.frame.payload);
        if (failure) throw failure;
        if (credit > 1n) { swept += 1n; sweeping = true; continue; }
        if (!stopping) await credited(subscription, 1n);
        if (!held) start();
      }
      await pass;
      if (!stopping && woken.length) { start(); await pass; }
      if (failure) throw failure;
      if (terminal !== undefined && !stopping) onStatus(terminal, 'SubscriptionEnded', why);
    } catch (error) {
      observing = false; faulted = true;
      if (!stopping && !ended) onStatus(error.code ?? 'READ_UNAVAILABLE');
    } finally {
      observing = false;
      try { await pass; } catch {   }
      if (subscription !== undefined) {
        try {
          if (opened && !ended && !faulted) await closed(subscription);
          else subscription.release();
        } catch (error) { if (!stopping) onStatus(error.code ?? 'READ_UNAVAILABLE'); }
      }
    }
  })();
  return { done, get observing() { return observing; }, stop() { stopping = true; observing = false; return done; } };
}
