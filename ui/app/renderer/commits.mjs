import { key, joined } from './scene.mjs';
import { foldEpoch } from './fold.mjs';
import { LENS, keysOf } from './verbs.mjs';
import { accepted, closed, credited } from './records.mjs';
import { openCommits } from './session.mjs';
import { inSpace } from './reasons.mjs';

export function followCommits(session, scope, after, onCommit, onStatus, onObserving) {
  let stopping = false, observing = false;
  const done = (async () => {
    let subscription, opened = false, ended = false, faulted = false;
    try {
      subscription = await openCommits(session, scope, after, 1n);
      accepted(subscription.ack);
      opened = true; observing = !stopping;
      if (observing) onObserving?.();
      while (!stopping && !ended) {
        const result = { frame: await subscription.receive(), ended: subscription.ended };
        if (result.ended) {
          ended = true; observing = false;
          if (!stopping) onStatus(inSpace('Subscription', result.ended.code), 'SubscriptionEnded');
          break;
        }
        if (!result.frame || result.frame.arm === 'RetentionComplete') continue;
        if (stopping) break;
        await onCommit(result.frame.payload);
        if (!stopping) await credited(subscription, 1n);
      }
    } catch (error) {
      observing = false; faulted = true;
      if (!stopping && !ended) onStatus(error.code ?? 'READ_UNAVAILABLE');
    } finally {
      observing = false;
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

export const revisionShort = revision => revision?.kind === 'At'
  ? Array.from(revision.revision.slice(0, 6), b => b.toString(16).padStart(2, '0')).join('')
  : revision?.kind;

export function applyCommit(graph, commit, { own = false } = {}) {
  const before = scope => [...commit.metadata.revisions].find(([s]) => key(s) === key(scope))?.[1]?.authoringBefore;
  const replaced = own ? [] : commit.epoch.content.flatMap(keysOf)
    .filter(([kind, actor, value]) => kind === 'actor' && value !== undefined && LENS.actor.view(graph.declared, actor) !== undefined)
    .map(([, actor]) => ({ actor, before: before(actor.scope) }));
  const declared = foldEpoch(graph.declared, commit);
  const changed = declared !== graph.declared;
  const transition = [...commit.metadata.revisions].find(([scope]) => key(scope) === key(graph.anchor.scope))?.[1];
  const anchor = { ...graph.anchor, cursor: commit.metadata.cursor,
    environment: commit.metadata.afterEnvironment,
    ...(transition ? { authoringRevision: transition.authoringAfter } : {}) };
  return { graph: changed ? joined({ ...graph, anchor, declared }) : { ...graph, anchor }, changed, replaced };
}
