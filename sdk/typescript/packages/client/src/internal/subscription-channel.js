import { CircularUInt } from "../../../protocol/src/value.js";

/**
 * One reader owns the held correlation. What is read before its reader asks keeps its arrival
 * order: the frames (and the ending) that overtake a request's answer, and whatever
 * had already arrived when this side looked whether the stream is over. A frame is taken by
 * `next`, an answer by `untilAnswer`; neither takes the other's.
 */
export function subscriptionChannel(channel) {
  const ahead = [];
  const framed = (envelope) => envelope.kind.verb === "Frame" || envelope.kind.verb === "SubscriptionEnded";
  const take = (kind) => {
    const at = ahead.findIndex((envelope) => framed(envelope) === kind);
    return at === -1 ? undefined : ahead.splice(at, 1)[0];
  };
  return {
    get slot() { return channel.slot; },
    send: (...args) => channel.send(...args),
    release: () => channel.release(),
    async next(waitMs) { return take(true) ?? await channel.next(waitMs); },
    async untilAnswer(waitMs) {
      const read = take(false);
      if (read !== undefined) return read;
      for (;;) {
        const envelope = await channel.next(waitMs);
        if (envelope === null) return null;
        if (framed(envelope)) {
          ahead.push(envelope);
        } else {
          return envelope;
        }
      }
    },
    async over() {
      for (let envelope = await channel.next(0); envelope !== null; envelope = await channel.next(0)) ahead.push(envelope);
      return this.ended;
    },
    /** Whether the stream's ending is among what has been read from the key — nothing more is read. */
    get ended() {
      return ahead.some((envelope) => envelope.kind.verb === "SubscriptionEnded");
    },
  };
}

/** Validate the existing Credit ACK without erasing its pending UInt wrapper. */
export function creditAccepted(payload, fail) {
  if (!Array.isArray(payload) || payload[0] !== 1n) return false;
  const accepted = payload[1];
  if (payload.length !== 2 || !(accepted?.pending_after instanceof CircularUInt)
    || Object.keys(accepted).length !== 1) {
    fail("CREDIT_PENDING_AFTER", "accepted Credit carries exactly pending_after: UInt (u64)");
  }
  return true;
}
