
import { creditValue, subscribeValue, subscriptionFrameFromValue, UNSUBSCRIBE_BODY } from "@circular/protocol/subscription";
import { FrameOrigin, RejectionReason, SubscriptionEndReason, SubscriptionFrame } from "@circular/protocol/tables";

import { recordRegistrations } from '../../protocol/src/internal/record-values.js';
import { subscriptionChannel, creditAccepted } from "./internal/subscription-channel.js";

export class SubscriptionError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new SubscriptionError(code, message);
}

/** Arm names by tag, from `@circular/protocol/tables` — so a tag never appears bare below. */
const byTag = rows => Object.freeze(Object.fromEntries(rows.map(({ name, tag }) => [tag, name])));
const FRAME_ARMS = byTag(SubscriptionFrame);
const FRAME_ORIGINS = byTag(FrameOrigin);
/** End reasons. Three carry an argument. */
const END_REASONS = byTag(SubscriptionEndReason);

const UNRESOLVED = BigInt(RejectionReason.find(reason => reason.name === 'Unresolved').numbers[0]);
const refusedAsUnresolved = (payload) => {
  if (!Array.isArray(payload) || payload[0] !== 2n) return false;
  const code = payload[1]?.code;
  return (typeof code === "bigint" ? code : typeof code?.value === "bigint" ? code.value : undefined) === UNRESOLVED;
};

export function readFrame(body) {
  if (!Array.isArray(body) || body.length !== 2) fail("FRAME_SHAPE", "a frame is [arm, body]");
  const arm = FRAME_ARMS[String(body[0])];
  if (arm === undefined) fail("FRAME_ARM_UNKNOWN", `frame arm ${body[0]} is unassigned`);
  if (arm === "RetentionComplete") return subscriptionFrameFromValue(body);
  const fields = body[1];
  const origin = FRAME_ORIGINS[String(fields.origin)];
  if (origin === undefined) fail("FRAME_ORIGIN_UNKNOWN", `frame origin ${fields.origin} is unassigned`);

  if (arm === "Credit") {
    return Object.freeze({ ...subscriptionFrameFromValue(body), folded: null, slot: null });
  }
  if (Object.hasOwn(fields, "pending_after")) fail("FRAME_PENDING_AFTER", "pending_after belongs only to Credit frames");
  if (arm !== "Conflated") {
    return Object.freeze({ arm, origin, payload: fields.payload, folded: null, slot: null });
  }
  if (fields.folded === undefined) {
    fail("FRAME_FOLDED_MISSING", "a conflated frame always carries its folded count, zero included");
  }
  return Object.freeze({
    arm,
    origin,
    payload: fields.payload,
    folded: fields.folded,
    slot: fields.slot === undefined ? null : fields.slot,
  });
}

/** Reads a terminal frame's reason, keeping the argument the three carrying arms have. */
export function readEnding(body) {
  if (body === null || typeof body !== "object" || Array.isArray(body)) {
    fail("ENDING_SHAPE", "an ending is an object of reason, code and anchor");
  }
  const raw = body.reason;
  const tag = Array.isArray(raw) ? raw[0] : raw;
  const reason = END_REASONS[String(tag)];
  if (reason === undefined) fail("ENDING_REASON_UNKNOWN", `end reason ${tag} is unassigned`);
  return Object.freeze({
    reason,
    argument: Array.isArray(raw) ? raw[1] : null,
    code: body.code,
    anchor: body.anchor,
  });
}

/** The key a read names to go through a replay lens — the lens handle's own correlation. */
function lensCorrelation(lens) {
  if (lens === null || typeof lens !== "object" || !Number.isSafeInteger(lens.correlation)) {
    fail("SUBSCRIPTION_LENS", "a lens is the handle session.replay.start returned");
  }
  return lens.correlation;
}

/**
 * Opens a subscription over any session that can `exchange`.
 *
 * `initialCredit` is required. There is no default, because a default would be this file
 * choosing a consumer's speed — the one thing the wire leaves to the consumer — and a client
 * that opened with an invented balance would look like it was keeping up when it was being fed.
 */
export async function openSubscription(session, options) {
  if (session === null || typeof session !== "object" || typeof session.hold !== "function") {
    fail(
      "SUBSCRIPTION_SESSION_INVALID",
      "a subscription needs a session that can hold one correlation open. Its "
      + "three requests on one key, and the engine answers that key with a stream — a client built "
      + "out of one-shot exchanges would allocate a slot per request and lose every envelope after "
      + "the first on each",
    );
  }
  if (!Number.isInteger(Number(options?.initialCredit)) || Number(options.initialCredit) <= 0) {
    fail("SUBSCRIPTION_CREDIT_REQUIRED", "opening states an initial credit; there is no default speed to choose");
  }

  const registration=Object.hasOwn(recordRegistrations,options.target) ? recordRegistrations[options.target] : undefined;
  if (registration?.subscription) registration.args(options.args);

  const channel = subscriptionChannel(session.hold(`subscription-${options.target}`, "SubscriptionEnded"));
  let granted = 0n;
  let received = 0n;
  let ended = null;
  let ack = null;

  /** Reads envelopes until one answers a request, keeping any frames that overtake it. */
  async function untilAnswer(what, waitMs) {
    const envelope = await channel.untilAnswer(waitMs);
    if (envelope === null) fail("SUBSCRIPTION_UNANSWERED", `${what} was not answered`);
    return envelope;
  }

  const lens = options.lens === undefined ? {} : { lens: lensCorrelation(options.lens) };
  let opened;
  try {
    await channel.send("Subscription", "Subscribe", subscribeValue({ target: options.target, args: options.args, ...lens }));
    opened = await untilAnswer("Subscribe", options.openTimeoutMs ?? 5000);
    if (opened.kind.verb !== "SubscribeAck") {
      fail("SUBSCRIPTION_ANSWER_UNEXPECTED", `Subscribe was answered with ${opened.kind.verb}`);
    }
  } catch (error) {
    channel.release();
    throw error;
  }
  ack = opened.payload;

  const handle = {
    /** The engine's answer to the open, whichever arm it carried. */
    get ack() {
      return ack;
    },
    /** The correlation all three requests and every frame share. */
    get correlation() {
      return channel.slot;
    },
    /** Frames granted minus frames received — what the consumer still owes itself. */
    get balance() {
      return granted - received;
    },
    get ended() {
      return ended;
    },

    async credit(frames) {
      if (ended !== null) fail("SUBSCRIPTION_ENDED", "a closed subscription takes no more credit");
      const body = creditValue({ frames });
      if (await channel.over()) return null;
      await channel.send("Subscription", "Credit", body);
      const answer = await untilAnswer("Credit", options.creditTimeoutMs ?? 5000);
      if (answer.kind.verb !== "SubscribeAck") fail("SUBSCRIPTION_ANSWER_UNEXPECTED", `Credit was answered with ${answer.kind.verb}`);
      if (creditAccepted(answer.payload, fail)) granted += body.frames;
      else if (refusedAsUnresolved(answer.payload) && channel.ended) return null;
      return answer.payload;
    },

    /**
     * The next frame, or `null` if none arrives.
     *
     * `null` rather than a throw: a credited stream with nothing to say is an ordinary state, and
     * only the caller knows whether it is a fault here.
     */
    async receive(waitMs = 2000) {
      if (ended !== null) fail("SUBSCRIPTION_ENDED", "a closed subscription receives no more frames");
      const envelope = await channel.next(waitMs);
      if (envelope === null) return null;
      if (envelope.kind.verb === "SubscriptionEnded") {
        ended = readEnding(envelope.payload);
        return null;
      }
      if (envelope.kind.verb !== "Frame") {
        fail("SUBSCRIPTION_ANSWER_UNEXPECTED", `a frame slot carried ${envelope.kind.verb}`);
      }
      received += 1n;
      const frame=readFrame(envelope.payload);
      if (registration && frame.arm!=="RetentionComplete") {
        if (registration.subscriptionItem) {
          return { ...frame, payload: registration.subscriptionItem(frame.payload) };
        }
        registration.item(frame.payload);
      }
      return frame;
    },

    /** Delivers a terminal frame body directly, for callers holding their own transport. */
    end(body) {
      ended = readEnding(body);
      return ended;
    },

    /**
     * Asks the engine to close.
     *
     * Resolves on the reply. The ending arrives separately as a frame, and this deliberately
     * does not wait for it: folding the two would let the reply overtake frames still queued and
     * claim a stream had ended while it was still delivering.
     *
     * `null` when the stream's ending has already reached this side — the stream is over,
     * so no `Unsubscribe` is sent to a dead key, the slot is let go, and `receive` still gives the
     * ending.
     */
    async close() {
      if (await channel.over()) {
        channel.release();
        return null;
      }
      await channel.send("Subscription", "Unsubscribe", UNSUBSCRIBE_BODY);
      const answer = await untilAnswer("Unsubscribe", options.closeTimeoutMs ?? 5000);
      if (answer.kind.verb !== "SubscribeAck") {
        fail("SUBSCRIPTION_ANSWER_UNEXPECTED", `Unsubscribe was answered with ${answer.kind.verb}`);
      }
      channel.release();
      return answer.payload;
    },

    /**
     * Lets the stream go without waiting. The session sends the one `Unsubscribe` on
     * this key — unless `close` already did, or the engine already ended the stream — and hands
     * the slot back when the engine has answered, so no frame follows a released stream. A stream
     * the engine already ended has given its slot back on its own; releasing it does nothing.
     */
    release() {
      channel.release();
    },
  };

  try {
    await handle.credit(options.initialCredit);
  } catch (error) {
    channel.release();
    throw error;
  }
  return handle;
}
