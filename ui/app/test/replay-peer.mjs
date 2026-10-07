import { establish, OWNER_LOCAL_RESOURCE_CEILINGS } from '@circular/client';
import { envelope, wireEnvelopeCodec } from '@circular/protocol';
import { recordedActor, recordedArrival } from './time-harness.mjs';
import { heldSession, ack, creditAck, arrivalFrame, retentionComplete, item, uint } from './records-peer.mjs';
import { healthPage, deadLetterItem, edgeDepthRow, edgeDepthsComplete } from './fixtures.mjs';

export const codec = wireEnvelopeCodec(OWNER_LOCAL_RESOURCE_CEILINGS);
export const turns = async (n = 20) => { for (let i = 0; i < n; i += 1) await new Promise(resolve => setImmediate(resolve)); };

export async function replaySession({refuse, moved} = {}) {
  const frames = [], queue = [];
  let waiting, closed = false;
  const transport = {
    incoming: {[Symbol.asyncIterator]() { return this; }, next() {
      if (queue.length) return Promise.resolve({value:queue.shift(), done:false});
      if (closed) return Promise.resolve({done:true});
      return new Promise(resolve => { waiting = resolve; });
    }},
    async send(bytes) {
      const {envelope:request} = codec.decode(bytes);
      if (request.kind.verb === 'Goodbye') return;
      let reply;
      if (request.kind.verb === 'Hello') reply = envelope('SessionMechanics', 'HelloAck', request.correlation,
        {features:request.payload.features, protocol_version:1n, roles:[], token:new Uint8Array(32), trust:1n});
      else {
        frames.push(Uint8Array.from(bytes));
        const rejected = refuse?.(request.kind.verb);
        if (!rejected) moved?.(request.kind.verb, request.payload);
        reply = envelope('ReplayControl', 'ReplayResult', request.correlation, rejected ? [2n, rejected] : 1n);
      }
      const encoded = codec.encode(reply);
      if (waiting) { const resolve = waiting; waiting = undefined; resolve({value:encoded, done:false}); } else queue.push(encoded);
    },
    async close() { closed = true; waiting?.({done:true}); },
  };
  return {session:await establish(transport, {resourceCeilings:OWNER_LOCAL_RESOURCE_CEILINGS}), frames};
}

export const ARRIVALS_MS = [1_000n, 2_000n, 3_000n];
export const oneCut = index => [{actor:recordedActor('one'), index}];
export function atValue(at_ms, epochAt = () => 2n) {
  const index = BigInt(ARRIVALS_MS.filter(at => at <= at_ms).length);
  const resolved = index === 0n ? 0n : ARRIVALS_MS[Number(index) - 1];
  return {anchor:{requested_ms:uint(at_ms), resolved_ms:uint(resolved), stream:1n, revision_epoch:uint(epochAt(index)),
    cut:oneCut(index)}, items:[], terminal:2n};
}
export function binsValue({from_ms, to_ms, bins}, marks = []) {
  const from = from_ms.value, bin = (to_ms.value - from) / bins.value;
  const counts = Array.from({length:Number(bins.value)}, (_, i) =>
    ARRIVALS_MS.filter(at => at >= from + BigInt(i) * bin && at < from + BigInt(i + 1) * bin).length);
  return {anchor:{from_ms:uint(from), to_ms:uint(to_ms.value), bin_ms:uint(bin),
    bins:counts.map(count => ({count:uint(count), incidents:uint(0n)})),
    marks:marks.filter(([, at]) => at >= from && at < to_ms.value).map(([kind, at_ms]) => ({kind, at_ms:uint(at_ms)})),
    clock_regressions:uint(0n)}, items:[], terminal:2n};
}

const deadLetter = deadLetterItem({ origin: { actor: { scope: [], local: 'one' }, port: 'event' }, reason: { code: 'destination_gone', detail: null },
  scope: [], subject: { shape: [2n, 'string'], value: 'lost' }, target: { actor: { scope: [], local: 'one' }, port: 'event' } });
export async function lensPeer({refuse, marks = [], epochAt, commands = []} = {}) {
  const ending = (reason, code) => ({kind:{verb:'SubscriptionEnded'}, payload:{reason, code, anchor:new Uint8Array([4])}});
  const replay = await replaySession({refuse, moved:(verb, body) => {
    if (verb === 'ReplayRewind' && body?.to !== undefined) endLens(ending([6n, new Uint8Array([1])], 2n));
    if (verb === 'ReplayEnd') endLens(ending(3n, 2n));
  }});
  const queues = new Map(), waiters = new Map(), sent = [], asked = new Map(),
    reads = {snapshot:0, health:0, lensHealth:0, at:[], bins:0, queries:[]};
  const binsAsked = [];
  let handles = 0, rows = [recordedArrival('one', 1n, 1_000n)], lensRows = rows, letters = 0, depths = [];
  let held = null;
  const lensOf = handle => asked.get(handle)?.lens;
  const push = (handle, env) => {
    const resolve = waiters.get(handle);
    if (resolve) { waiters.delete(handle); resolve(env); } else queues.get(handle).push(env);
  };
  const answer = value => ({kind:{verb:'QueryResult'}, payload:[1n, value]});
  const snapshot = {status:'accepted', value:{anchor:{scope:[]}, commands:[{kind:'UpsertActor', actor:{arm:'relative', value:recordedActor('one')},
    declaration:{actorType:'synthetic', config:{}, flags:{bypass:false, pause:false, mute:false}}}, ...commands], terminal:'Complete'}};
  const sdk = heldSession({
    authoringSnapshot:async () => { reads.snapshot += 1; return snapshot; },
    replay:replay.session.replay,
    hold:async name => {
      const handle = `${name}#${++handles}`;
      queues.set(handle, name === 'subscription-edge.depths' ? question(handle) : name.startsWith('subscription-') ? [ack, creditAck] : []);
      return handle;
    },
    send:async (handle, partition, verb, value) => {
      sent.push({handle, partition, verb, value});
      if (verb === 'Query' || verb === 'Subscribe') asked.set(handle, value);
      if (verb === 'Query') reads.queries.push({name:value.name, lens:value.lens});
    },
    declare:async command => { sent.push({verb:'declare', value:command}); return {status:'rejected'}; },
    exchange:async (partition, verb, payload) => {
      if (partition === 'Query') return answer({anchor:null, items:[], terminal:2n});
      sent.push({verb:'exchange', value:{partition, verb, payload}}); return {status:'rejected'};
    },
    next:async handle => {
      const name = handle.split('#')[0];
      if (name === 'actor.events') return answer({anchor:1n, items:lensOf(handle) === undefined ? rows : lensRows, terminal:2n});
      if (name === 'runtime.approvals') return answer({anchor:{producer:1n, persistence:[3n]}, items:[], terminal:2n});
      if (name === 'daemon.health') {
        if (held && held.lens === (lensOf(handle) !== undefined)) await held.until;
        reads[lensOf(handle) === undefined ? 'health' : 'lensHealth'] += 1; return answer(healthPage());
      }
      if (name === 'timeline.at') { const at = asked.get(handle).args.at_ms.value; reads.at.push(at); return answer(atValue(at, epochAt)); }
      if (name === 'timeline.bins') {
        const args = asked.get(handle).args;
        reads.bins += 1; binsAsked.push(Object.fromEntries(Object.entries(args).map(([key, value]) => [key, value.value])));
        return answer(binsValue(args, marks));
      }
      if (name === 'dead.letters') return answer({anchor:null, items:Array(letters).fill(deadLetter), terminal:2n});
      if (!name.startsWith('subscription-')) return answer({anchor:null, items:[], terminal:2n});
      const queue = queues.get(handle);
      if (queue.length) return queue.shift();
      return new Promise(resolve => waiters.set(handle, resolve));
    },
    release:async () => {},
  });
  const ended = new Set();
  const open = (prefix, lens) => [...queues.keys()].filter(handle => handle.startsWith(prefix) && !ended.has(handle)
    && (lens === undefined || (lensOf(handle) !== undefined) === lens));
  const DEPTHS = 'subscription-edge.depths#';
  function question(handle) {
    ended.add(handle);
    const frames = depths.map(edgeDepthRow).flatMap((payload, i, all) => [
      ...(i ? [creditAck] : []), {kind:{verb:'Frame'}, payload:[3n, {origin:2n, pending_after:uint(BigInt(all.length - i - 1)), payload}]}]);
    return [ack, ...(depths.length ? [creditAck] : []), ...frames, {kind:{verb:'SubscriptionEnded'}, payload:edgeDepthsComplete(depths.length)}];
  }
  const feeds = rows => rows.filter(r => !r.handle?.startsWith(DEPTHS));
  function endLens(frame) {
    for (const handle of open('subscription-', true)) { ended.add(handle); push(handle, frame); }
  }
  const newest = (target, lens) => open(`subscription-${target}`, lens).at(-1);
  return {sdk, sent, reads, replay, asked:{bins:binsAsked},
    set rows(value) { rows = value; },
    set lensRows(value) { lensRows = value; },
    set letters(value) { letters = value; },
    set depths(value) { depths = value; },
    questions:() => sent.filter(r => r.verb === 'Subscribe' && r.value.target === 'edge.depths').length,
    arrive(arrival, {lens = true} = {}) {
      const handle = newest('actor.events', lens);
      push(handle, arrivalFrame(arrival)); push(handle, null); push(handle, creditAck);
    },
    record({lens = true} = {}) {
      const handle = newest('records', lens);
      push(handle, retentionComplete); push(handle, creditAck);
      push(handle, {kind:{verb:'Frame'}, payload:[3n, {origin:lens ? 1n : 2n, payload:item, pending_after:uint(0n)}]});
      push(handle, creditAck);
    },
    lose(target) { const handle = newest(target, false); ended.add(handle); push(handle, ending(5n, 22n)); },
    reset(target) { const handle = newest(target, false); ended.add(handle); push(handle, ending([6n, new Uint8Array([1])], 2n)); },
    holdHealth({lens = true} = {}) {
      let release;
      held = {lens, until:new Promise(resolve => { release = resolve; })};
      return () => { held = null; release(); };
    },
    subscriptions:lens => open('subscription-', lens).length,
    subscribed:() => feeds(sent).filter(r => r.verb === 'Subscribe').map(r => ({target:r.value.target, lens:r.value.lens})),
    liveVerbs:(from = 0) => feeds(sent.slice(from)).filter(r => r.handle.startsWith('subscription-') && lensOf(r.handle) === undefined).map(r => r.verb),
  };
}

export const decoded = frames => frames.map(bytes => codec.decode(bytes).envelope).map(e => [e.kind.verb, e.payload]);
export const startKey = frames => BigInt(frames.map(bytes => codec.decode(bytes).envelope)
  .find(e => e.kind.verb === 'ReplayStart').correlation);
