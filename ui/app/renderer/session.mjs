import { establish, OWNER_LOCAL_RESOURCE_CEILINGS, daemonHealth, daemonHealthPageFromValue, actorEvents,
  actorEventsPageFromValue, runtimeApprovals, runtimeApprovalsPageFromValue,
  queryCatalog, queryCatalogPageFromValue, deadLetters, deadLettersPageFromValue, instanceTransitions,
  instanceTransitionsPageFromValue, decideApproval, recordsSubscription, actorEventsSubscription,
  authoringCommits } from '@circular/client';
import { TIMELINE_BINS_QUERY, TIMELINE_AT_QUERY, timelineBinsArgsValue, timelineAtArgsValue, timelineBinsFromValue,
  timelineAtFromValue } from '@circular/protocol';
import { openSubscription } from '@circular/client/subscription';
import { sessionRoleValue, scopeIdentityValue } from '@circular/protocol/establishment';
import { authoringCommitArgumentsValue, authoringCommitFrameFromValue } from '@circular/protocol/authoring-query';
import { fault, readFirstPage, readCompleteAnswer, journalRows } from './query.mjs';
import { inSpace } from './reasons.mjs';

const routes = new WeakMap();
function route(frames) {
  let takes = routes.get(frames);
  if (!takes) {
    takes = new Map();
    frames.onFrame((attachment, bytes) => takes.get(attachment)?.(bytes));
    routes.set(frames, takes);
  }
  return takes;
}

/**
 * The `{incoming, send, close}` transport `establish` takes, over the shell's frames bridge
 * (`preload.mjs` `circularFrames`: `send(attachment, bytes)`, `onFrame(handler)`, `close(attachment)`)
 * on one attachment — the one the shell's connection answer named. Its frames are that attachment's
 * alone, both ways. The shell hands `null` when that attachment's socket ended; the incoming frames
 * end there.
 */
export function framesTransport(frames, attachment) {
  const takes = route(frames), ready = [];
  let ended = false, waiting = null;
  const take = bytes => {
    if (bytes === null) {
      ended = true;
      if (takes.get(attachment) === take) takes.delete(attachment);
    } else ready.push(bytes);
    const resume = waiting;
    waiting = null;
    resume?.();
  };
  takes.set(attachment, take);
  return {
    incoming: {
      async *[Symbol.asyncIterator]() {
        for (;;) {
          while (ready.length) yield ready.shift();
          if (ended) return;
          await new Promise(resolve => { waiting = resolve; });
        }
      },
    },
    async send(bytes) {
      const refused = await frames.send(attachment, bytes);
      if (refused?.code) throw fault(refused.code);
    },
    async close() {
      take(null);
      await frames.close(attachment);
    },
  };
}

export const canvasHello = () => ({ requestedRoles: [sessionRoleValue('Reader'), sessionRoleValue('Operator'),
  sessionRoleValue('Writer', scopeIdentityValue([]))] });

/** One session of this document over `attachment`, the one the shell's connection answer named. */
export async function openSession(frames, attachment, { hello = canvasHello() } = {}) {
  const transport = framesTransport(frames, attachment);
  try {
    return await establish(transport, { hello, resourceCeilings: OWNER_LOCAL_RESOURCE_CEILINGS });
  } catch (error) {
    await transport.close().catch(() => {});
    throw error;
  }
}

export function decoded(read, code = 'QUERY_PAGE_INVALID') {
  try { return read(); } catch (error) { if (error instanceof TypeError) throw fault(code); throw error; }
}
const page = async (session, name, reader, limit, named = true, lens) => {
  const value = await readFirstPage(session, name, null, limit, undefined, lens);
  return named ? decoded(() => reader(value)) : reader(value);
};
const complete = async (session, name, reader, limit, lens) => {
  const value = await readCompleteAnswer(session, name, null, limit, lens);
  return decoded(() => reader(value));
};

export async function readSnapshot(session, scope, limit, upto) {
  const result = await session.authoringSnapshot(scope, limit, ...(upto === undefined ? [] : [upto]));
  if (result.status !== 'accepted') return result;
  return { status: 'accepted', value: { anchor: result.value.anchor, commands: result.value.commands, terminal: 'Complete' } };
}
export const readDaemonHealth = (session, lens) => page(session, daemonHealth.name, daemonHealthPageFromValue, undefined, false, lens);
export const readApprovalQueue = session => page(session, runtimeApprovals.name, runtimeApprovalsPageFromValue, undefined, false);
export const readDeadLetters = (session, limit, lens) => complete(session, deadLetters.name, deadLettersPageFromValue, limit, lens);
export const readInstanceTransitions = (session, limit, lens) => complete(session, instanceTransitions.name, instanceTransitionsPageFromValue, limit, lens);
export const readQueryCatalog = session => page(session, queryCatalog.name, queryCatalogPageFromValue);

export async function readActorEvents(session, limit = journalRows, since) {
  const events = actorEventsPageFromValue(await readFirstPage(session, actorEvents.name, null, limit, since));
  if (Array.isArray(events.terminal) && events.terminal[0] === 3n) throw fault(inSpace('Query', events.terminal[1]));
  return events;
}
export async function readTimelineBins(session, args) {
  const page = await readFirstPage(session, TIMELINE_BINS_QUERY, timelineBinsArgsValue(args));
  return decoded(() => timelineBinsFromValue(page.anchor));
}
export async function readTimelineAt(session, atMs) {
  const page = await readFirstPage(session, TIMELINE_AT_QUERY, timelineAtArgsValue({ at_ms: atMs }));
  return decoded(() => timelineAtFromValue(page.anchor));
}

export async function decide(session, request) {
  try { return await decideApproval(session, request); }
  catch (error) { if (error instanceof TypeError) throw fault('RESULT_UNEXPECTED'); throw error; }
}

export function openRecords(session, scope, initialCredit, target = recordsSubscription.name, lens) {
  return openSubscription(session, { target, initialCredit,
    args: target === actorEventsSubscription.name ? null : { scope: scopeIdentityValue(scope) },
    ...(lens === undefined ? {} : { lens }) });
}
export async function openCommits(session, scope, after, initialCredit) {
  const subscription = await openSubscription(session, { target: authoringCommits.name,
    args: authoringCommitArgumentsValue(scope, after), initialCredit });
  return {
    get ack() { return subscription.ack; },
    get ended() { return subscription.ended; },
    credit: frames => subscription.credit(frames),
    async receive(...wait) {
      const frame = await subscription.receive(...wait);
      return frame && frame.arm !== 'RetentionComplete' ? { ...frame, payload: authoringCommitFrameFromValue(frame.payload) } : frame;
    },
    close: () => subscription.close(),
    release: () => subscription.release(),
  };
}
