import { CircularUInt } from '@circular/protocol';
export { ack, creditAck } from './fixtures.mjs';
export const uint = value => new CircularUInt(BigInt(value));
const zero = uint(0n);
export const item = {cursor:{anchor:[new Uint8Array(32),[]],domain:'records',position:new Uint8Array([5])},fact:new Uint8Array([7,8])};
export const frame = {kind:{verb:'Frame'},payload:[3n,{origin:2n,payload:item,pending_after:zero}]};
export const arrivalFrame = arrival => ({kind:{verb:'Frame'},payload:[3n,{origin:2n,payload:arrival,pending_after:zero}]});
export const retentionComplete = {kind:{verb:'Frame'},
  payload:[4n,{anchor:new Uint8Array([4]),delivered:zero}]};
export const end = {kind:{verb:'SubscriptionEnded'},payload:{reason:5n,code:22n,anchor:new Uint8Array([4])}};

/**
 * A session whose held slots are a scripted peer's: `hold(name)` answers a handle, and
 * `send(handle, partition, verb, value)`, `next(handle, waitMs)`, `release(handle)` drive it. The
 * SDK's own subscription and the canvas's readers work over it unchanged. Every other member of the
 * peer (declare, exchange, authoringSnapshot, replay, interactions …) is the session's own.
 */
export function heldSession(peer) {
  const { hold, send, next, release, ...members } = peer;
  return { ...members,
    hold(name) {
      const held = Promise.resolve(hold(name));
      let asked = 0;
      const kept = [];
      const answer = value => value?.kind?.verb === 'SubscribeAck';
      const hand = value => { if (answer(value)) asked = Math.max(0, asked - 1); return value; };
      const step = async (waitMs, rest) => {
        const taken = { settled: false };
        taken.value = next(await held, waitMs, ...rest).then(value => { taken.settled = true; taken.result = value; return value; },
          error => { taken.settled = taken.failed = true; throw error; });
        taken.value.catch(() => {});
        return taken;
      };
      return { slot: name,
        send: async (partition, ...args) => { if (partition === 'Subscription') asked += 1; return send(await held, partition, ...args); },
        async next(waitMs, ...rest) {
          if (waitMs !== 0) return hand(await (kept.shift() ?? await step(waitMs, rest)).value);
          if (!kept.length) kept.push(await step(0, rest));
          for (let turn = 0; turn < 50 && !kept[0].settled; turn++) await Promise.resolve();
          if (!kept[0].settled || kept[0].failed || (answer(kept[0].result) && asked === 0)) return null;
          return hand(kept.shift().result);
        },
        release: () => { void held.then(handle => release(handle)); } };
    },
  };
}
