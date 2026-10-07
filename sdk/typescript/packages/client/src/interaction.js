/**
 * `injector.inject(mount, payload)` — the authoring side of an interaction. `mount` is the export
 * mount's address `{ scope, local }` (the root scope is `[]`).
 *
 * v1's buttons and inputs sent `Bang` and `ExportInput`; v2 folds both into `Inject`, and the
 * verb has been on the wire for a while. What was missing is the part a caller cannot be asked
 * to get right on its own: the retry identity.
 *
 * # The idempotency key is not derived from the session token
 *
 * The obvious source for a per-session unique prefix is the token the engine issues at
 * establishment, and it is the wrong one, for two reasons that are written down in the token's
 * own definition rather than invented here.
 *
 *  - **It has no projection rule.** `identity.rs` says the token is not authority evidence and
 *    carries neither role nor trust, so there is no rule for splitting it into anything —
 *    inventing one here would be the first.
 *  - **The key travels, and the token must not.** Every `Inject` carries its key, and an
 *    injection is recorded as a boundary arrival, so a key derived from the token would write
 *    the token into the log. The token's own `Debug` refuses to print it for exactly this
 *    reason: knowing a live token means being able to name that session's subscriptions.
 *
 * So the origin is **the caller's and required**. This module keeps the counter and guarantees
 * that a key is never reused within an injector; what it will not do is invent the uniqueness it
 * cannot guarantee across two processes that both chose to be called "ui".
 *
 * # The key is Bytes
 *
 * `decode_inject` reads it with `bytes_of` and the published vectors carry `Bytes(1 bytes: [01])`
 * and `[02]`. It was built here as an array of numbers, which encodes as an `Array` of Floats and
 * is refused as `WrongCarrier { key: "idempotency" }`.
 *
 * The offline tests passed anyway, and that is the part worth keeping: they drove this encoder
 * against a fake far end, so they agreed with whatever shape this side chose. The live probe
 * found it in one run. It is the same lesson the verb-tag re-pin recorded — a round trip cannot
 * disagree with itself — and it is why `cross-check-t2-injection.test.mjs` now compares against
 * the daemon's bytes rather than against this module's idea of them.
 *
 * # A retry is explicit
 *
 * `inject` never retries by itself. The key exists so that a resend *can* be recognised as the
 * same injection, but whether the far end honours it is the far end's contract: the daemon's
 * injection custody recognises a replayed key only within its own process lifetime. An automatic
 * retry would therefore risk turning one timeout into two injections, silently, in exactly the
 * case the key was added to prevent.
 * `receipt.retry()` resends the same bytes when a caller decides to.
 *
 * # A rejection is an answer, not a failure
 *
 * A refused injection returns a receipt whose outcome is `rejected`, carrying the mount and the
 * key that was refused. It does not throw. v1's lesson is the reason: a rejection belongs at the
 * input that caused it, not in a global toast, and a caller can only put it there if the answer
 * says which injection it is about. Transport failures still throw — those are not answers.
 */

import { declarationAddressValue, declarationResultFromValue } from "@circular/protocol/declaration";

export class InteractionError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new InteractionError(code, message);
}

/**
 * Encodes a counter as the shortest big-endian byte sequence that represents it.
 *
 * Shortest rather than fixed-width so the key's length carries no accidental meaning, and
 * big-endian so that keys from one origin sort in issue order — which costs nothing here and is
 * the property anyone reading a log will assume.
 */
function counterBytes(value) {
  const bytes = [];
  let remaining = value;
  do {
    bytes.unshift(Number(remaining & 0xffn));
    remaining >>= 8n;
  } while (remaining > 0n);
  return bytes;
}

/** The key as it rides: a byte string, which is what the far end reads it as. */
function keyBytes(origin, counter) {
  return Uint8Array.from([...origin, ...counterBytes(counter)]);
}

function originBytes(origin) {
  if (origin instanceof Uint8Array) {
    if (origin.length === 0) fail("INJECT_ORIGIN_EMPTY", "an injection origin is a non-empty byte string");
    return [...origin];
  }
  if (typeof origin === "string") {
    if (origin.length === 0) fail("INJECT_ORIGIN_EMPTY", "an injection origin is a non-empty byte string");
    return [...new TextEncoder().encode(origin.normalize("NFC"))];
  }
  return fail(
    "INJECT_ORIGIN_REQUIRED",
    "inject() needs an origin this SDK cannot derive: the session token has no projection rule and "
    + "must not travel in an injection key",
  );
}

/**
 * The mount an injection names, as it rides: the export mount's declaration key
 * `{ local, scope }` — the identity inside an `UpsertExportMount` address, with no address arm
 * (`crates/protocol/src/injection_payload.rs`, `PlanExportKey`).
 *
 * The caller spells it `{ scope: [...segments], local }`, the same shape a declaration address
 * carries as its `value`. A root-scope mount is `scope: []`. A bare name is not an address: mount
 * names are unique per scope, so a name alone cannot say which scope's mount it means, and the
 * daemon refuses the bare-name envelope rather than guessing the root.
 */
function mountValue(mount) {
  if (mount === null || typeof mount !== "object" || Array.isArray(mount) || mount instanceof Uint8Array
    || typeof mount.local !== "string" || mount.local.length === 0 || !Array.isArray(mount.scope)) {
    fail("INJECT_MOUNT_REQUIRED",
      "inject() names a declared export surface by its mount address { scope, local } — its authored "
      + "scope (the root is []) and its name in that scope — not a bare name, an actor, or a port");
  }
  return declarationAddressValue({ arm: "absolute", value: mount }, "exportMount", "mutation")[1];
}

export async function inject(session, request) {
  const { mount, payload, idempotency } = request;
  const answer = await session.exchange("EventInjection", "Inject", { mount: mountValue(mount), payload, idempotency });
  if (answer.kind.verb !== "InjectAck") {
    fail("INJECT_ANSWER_UNEXPECTED", `Inject was answered with ${answer.kind.verb}`);
  }
  return declarationResultFromValue(answer.payload, "Inject");
}

/**
 * Opens an injector over any session that can `exchange`.
 *
 * Deliberately not a method on one session class. `exchange(partition, verb, payload)` already
 * sends an arbitrary verb, so anything that has it can inject, and binding this to one client
 * would make the other one grow its own copy.
 */
export function openInjector(session, options = {}) {
  if (session === null || typeof session !== "object" || typeof session.exchange !== "function") {
    fail("INJECT_SESSION_INVALID", "an injector needs a session with exchange(partition, verb, payload)");
  }
  const origin = originBytes(options.origin);
  let issued = 0n;
  const keys = new Set();

  const send = (mount, payload, idempotency) => inject(session, { mount, payload, idempotency });

  function receiptFor(mount, payload, idempotency, result) {
    return Object.freeze({
      mount,
      idempotency: Uint8Array.from(idempotency),
      outcome: result.status,
      /** The `Result<InjectionAccepted>` the one injection path read. */
      answer: result,
      /** Resends the same injection under the same key. */
      async retry() {
        return receiptFor(mount, payload, idempotency, await send(mount, payload, idempotency));
      },
    });
  }

  return Object.freeze({
    /** How many keys this injector has issued. */
    get issued() {
      return Number(issued);
    },

    /**
     * Sends one injection and returns its receipt.
     *
     * The key is minted here rather than accepted from the caller: a caller that supplied its
     * own would be free to reuse one by accident, and the reuse would look like a retry of an
     * injection it had never made.
     */
    async inject(mount, payload) {
      mountValue(mount);
      issued += 1n;
      const idempotency = keyBytes(origin, issued);
      const fingerprint = idempotency.join(",");
      if (keys.has(fingerprint)) {
        fail("INJECT_KEY_REUSED", `the injector minted ${fingerprint} twice`);
      }
      keys.add(fingerprint);
      return receiptFor(mount, payload, idempotency, await send(mount, payload, idempotency));
    },
  });
}
