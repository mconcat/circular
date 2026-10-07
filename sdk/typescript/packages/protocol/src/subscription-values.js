
import { CircularUInt } from "./value.js";
import { FrameOrigin, SubscriptionEndReason, SubscriptionFrame } from "./internal/closed-tables.js";

/** Arm names by tag, from `@circular/protocol/tables` — the tags are declared once, in Rust. */
const armsByTag = rows => new Map(rows.map(({ name, tag }) => [BigInt(tag), name]));
const FRAME_ORIGINS = armsByTag(FrameOrigin);
const FRAME_ARMS = armsByTag(SubscriptionFrame);
const END_REASONS = armsByTag(SubscriptionEndReason);

export class SubscriptionValueError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new SubscriptionValueError(code, message);
}

export function subscribeValue(request) {
  if (request === null || typeof request !== "object") fail("SUBSCRIBE_SHAPE", "a Subscribe is an object");
  for (const name of Object.keys(request)) {
    if (name !== "target" && name !== "args" && name !== "lens") {
      fail("SUBSCRIBE_UNEXPECTED", `a Subscribe carries target, args and lens, not \`${name}\``);
    }
  }
  if (typeof request.target !== "string" || request.target.length === 0) {
    fail("SUBSCRIBE_TARGET", "a target is a non-empty flat name");
  }
  if (request.args === undefined) fail("SUBSCRIBE_ARGS_MISSING", "args is the registration's data and is required");
  if (request.lens === undefined) return { args: request.args, target: request.target };
  return { args: request.args, lens: lensKey(request.lens), target: request.target };
}

/**
 * A replay lens is named by the correlation key its `ReplayStart` used — a `u32`, the same width
 * as every envelope correlation. This reads `Subscribe.lens` only, and returns it as a
 * `bigint` so the value codec writes an Int. `Query.lens` is not read here: a Query body passes
 * through unopened, so its caller writes the key as a `bigint` itself (a JS number would encode as
 * a Float, which the daemon refuses as malformed).
 */
function lensKey(value) {
  const key = typeof value === "bigint" ? value
    : Number.isSafeInteger(value) ? BigInt(value)
    : fail("LENS_KEY", "a lens is named by its correlation key, an integer");
  if (key < 0n || key > 0xffffffffn) fail("LENS_KEY", "a lens key is a correlation key: 0..=2^32-1");
  return key;
}

export function creditValue(request) {
  if (request === null || typeof request !== "object") fail("CREDIT_SHAPE", "a Credit is an object");
  for (const name of Object.keys(request)) {
    if (name !== "frames") fail("CREDIT_UNEXPECTED", `a Credit carries frames, not \`${name}\``);
  }
  const frames = typeof request.frames === "bigint" ? request.frames : BigInt(request.frames);
  if (frames <= 0n) fail("CREDIT_NOT_POSITIVE", "granting no frames is not granting");
  return { frames };
}

/**
 * `Unsubscribe` — no body.
 *
 * The absence rather than an empty object, and the distinction is the whole content of the
 * verb's shape: an empty object is a value that encodes to bytes, and this encodes to none. The
 * same position `Goodbye` holds.
 */
export const UNSUBSCRIBE_BODY = null;

function frameOrigin(value) {
  const origin = FRAME_ORIGINS.get(value);
  if (origin === undefined) return fail("FRAME_ORIGIN", "frame origin is Retained or Live");
  return origin;
}

/** Reads the shared physical frame sum without opening its target-owned payload. */
export function subscriptionFrameFromValue(value) {
  if (!Array.isArray(value) || value.length !== 2 || typeof value[0] !== "bigint") {
    fail("FRAME_SHAPE", "a subscription frame is a two-part sum");
  }
  const body = value[1];
  if (body === null || typeof body !== "object" || Array.isArray(body)) {
    fail("FRAME_SHAPE", "a subscription frame body is an object");
  }
  const arm = FRAME_ARMS.get(value[0]);
  if (arm === "RetentionComplete") {
    if (!Object.hasOwn(body, "anchor") || !(body.delivered instanceof CircularUInt)
      || Object.keys(body).some(key => !["anchor", "delivered"].includes(key))) {
      fail("FRAME_SHAPE", "RetentionComplete carries exactly anchor and delivered: UInt");
    }
    return Object.freeze({ arm: "RetentionComplete", anchor: body.anchor, delivered: body.delivered });
  }
  const origin = frameOrigin(body.origin);
  if (arm !== "Credit" && Object.hasOwn(body, "pending_after")) {
    fail("FRAME_PENDING_AFTER", "pending_after belongs only to Credit frames");
  }
  if (arm === "Lossless") {
    return Object.freeze({ arm: "Lossless", origin, payload: body.payload });
  }
  if (arm === "Conflated") {
    if (typeof body.folded !== "bigint" || body.folded < 0n) {
      fail("FRAME_FOLDED", "a conflated frame carries a non-negative fold count");
    }
    return Object.freeze({
      arm: "Conflated", origin, folded: body.folded,
      slot: body.slot ?? null, payload: body.payload,
    });
  }
  if (arm === "Credit") {
    if (!(body.pending_after instanceof CircularUInt)) {
      fail("FRAME_PENDING_AFTER", "Credit pending_after is a required UInt (u64)");
    }
    if (!Object.hasOwn(body, "payload") || Object.keys(body).some(key => !["origin", "payload", "pending_after"].includes(key))) {
      fail("FRAME_SHAPE", "Credit carries exactly origin, payload and pending_after");
    }
    return Object.freeze({ arm: "Credit", origin, payload: body.payload, pending_after: body.pending_after });
  }
  return fail("FRAME_ARM", `subscription frame arm ${String(value[0])} is unassigned`);
}

/** The end-reason arms that carry an argument — `[tag, argument]` — and where the SDK puts it. */
const END_REASON_ARGUMENT = Object.freeze({
  ResetRequired: "floorOrCursor",
  ScopeGone: "cursor",
  IncompatibleClient: "requiredEnvironment",
});

function endReason(value) {
  if (typeof value === "bigint") {
    const kind = END_REASONS.get(value);
    if (kind !== undefined && !Object.hasOwn(END_REASON_ARGUMENT, kind)) return Object.freeze({ kind });
  }
  if (!Array.isArray(value) || value.length !== 2 || typeof value[0] !== "bigint") {
    fail("SUBSCRIPTION_END_REASON", "subscription end reason is a closed sum");
  }
  const kind = END_REASONS.get(value[0]);
  if (kind !== undefined && Object.hasOwn(END_REASON_ARGUMENT, kind)) {
    return Object.freeze({ kind, [END_REASON_ARGUMENT[kind]]: value[1] });
  }
  return fail("SUBSCRIPTION_END_REASON", `subscription end reason ${String(value[0])} is unassigned`);
}

export function subscriptionEndedFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail("SUBSCRIPTION_ENDED", "SubscriptionEnded is an object");
  }
  if (!(value.anchor instanceof Uint8Array) || typeof value.code !== "bigint") {
    fail("SUBSCRIPTION_ENDED", "SubscriptionEnded carries Bytes anchor and Int code");
  }
  return Object.freeze({
    kind: "Ended",
    reason: endReason(value.reason),
    diagnostic: Object.freeze({ code: Number(value.code), message: "Subscription ended.", hint: null, at: null }),
    anchor: value.anchor,
  });
}
