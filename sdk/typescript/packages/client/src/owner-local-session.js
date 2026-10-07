
import { wireEnvelopeCodec } from "@circular/protocol";
import { Partition } from "@circular/protocol/tables";
import { lifecyclePayloadValue } from '../../protocol/src/lifecycle-payload.js';
import {
  REPLAY_END_BODY,
  replayRewindValue,
  replayStartValue,
  replayTargetFromCheckpoint,
} from '../../protocol/src/replay-values.js';
import {
  declarationPayloadValue,
  declarationResultFromValue,
} from "@circular/protocol/declaration";
import {
  authoringCommitArgumentsValue,
  authoringSnapshotArgumentsValue,
  authoringSnapshotQueryResultFromValue,
} from "@circular/protocol/authoring-query";
import {
  creditValue,
  subscribeValue,
  subscriptionEndedFromValue,
  subscriptionFrameFromValue,
} from "@circular/protocol/subscription";

import { CorrelationPool, terminatesExchange } from "./correlation.js";
import { inject } from "./interaction.js";
import { recordRegistrations } from "../../protocol/src/internal/record-values.js";
import { subscriptionChannel, creditAccepted } from "./internal/subscription-channel.js";

/** Visible finite default used when a caller does not provide `maxInFlight`. */
export const DEFAULT_MAX_IN_FLIGHT = 256;

export class SessionError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new SessionError(code, message);
}

function tokenBytes(value) {
  if (!(value instanceof Uint8Array) || value.length !== 32) {
    fail("ESTABLISHED_SHAPE", "a session token is exactly 32 raw bytes");
  }
  return Uint8Array.from(value);
}

/** The partitions in declaration order (`Partition::ALL`, `@circular/protocol/tables`). A `Hello` names its minor for each. */
export const PARTITION_SPELLINGS = Object.freeze(Partition.map(({ name }) => name));

export function helloBody(options = {}) {
  const protocolVersion = options.protocolVersion ?? 1;
  const minor = options.minor ?? 0;
  const requestedRoles = options.requestedRoles ?? [];
  if (!Number.isSafeInteger(minor) || minor < 0 || minor > 0xff) {
    fail("MINOR_WIDTH", "a feature minor is a u8");
  }
  return {
    features: Object.fromEntries(PARTITION_SPELLINGS.map((name) => [name, BigInt(minor)])),
    protocol_version: BigInt(protocolVersion),
    requested_roles: [...requestedRoles],
  };
}

export function readEstablished(body) {
  if (body === null || typeof body !== "object" || Array.isArray(body) || body instanceof Uint8Array) {
    fail("ESTABLISHED_SHAPE", "an Established is an object of five members");
  }
  for (const name of ["features", "protocol_version", "roles", "token", "trust"]) {
    if (!(name in body)) fail("ESTABLISHED_INCOMPLETE", `an Established body requires ${name}`);
  }
  return Object.freeze({
    features: body.features,
    protocolVersion: body.protocol_version,
    roles: body.roles,
    token: tokenBytes(body.token),
    trust: body.trust,
  });
}

export async function establish(transport, options) {
  const { resourceCeilings } = options;
  if (resourceCeilings === undefined) {
    fail("CEILINGS_MISSING", "the value ceilings have no default; a caller must choose them");
  }
  const codec = wireEnvelopeCodec(resourceCeilings);
  const maxInFlight = options.maxInFlight ?? DEFAULT_MAX_IN_FLIGHT;
  if (!Number.isSafeInteger(maxInFlight) || maxInFlight <= 0) {
    throw new RangeError("maxInFlight must be a positive safe integer");
  }
  const pool = new CorrelationPool({ maximumLive: maxInFlight });
  const requestTimeoutMs = options.requestTimeoutMs;

  const waiters = new Map();
  const unmatched = [];
  /** Slots held open across many envelopes, keyed by slot. See `hold` below. */
  const held = new Map();
  let readerFault = null;
  let ended = false;
  let issued = 0;

  const settleAll = (error) => {
    readerFault = error;
    for (const waiter of waiters.values()) waiter.reject(error);
    waiters.clear();
    for (const stream of [...held.values()]) {
      stream.end(error);
      stream.abandon();
    }
  };

  const reading = (async () => {
    try {
      for await (const frame of transport.incoming) {
        const decoded = codec.decode(frame);
        if (decoded.status !== "complete") {
          settleAll(new SessionError("ENVELOPE_MALFORMED", decoded.diagnostics?.[0]?.message ?? "the envelope did not decode"));
          return;
        }
        const { envelope } = decoded;
        const stream = held.get(envelope.correlation);
        if (stream !== undefined) {
          stream.deliver(envelope);
          continue;
        }
        const waiter = waiters.get(envelope.correlation);
        if (waiter === undefined) {
          unmatched.push(envelope);
          continue;
        }
        if (terminatesExchange(envelope.kind.verb)) {
          waiters.delete(envelope.correlation);
          if (!leavesPagesOpen(envelope) && !leavesLensOpen(waiter.verb, envelope)) {
            pool.release(envelope.correlation);
          }
        }
        waiter.resolve(envelope);
      }
      ended = true;
      settleAll(new SessionError("SESSION_CLOSED", "the transport ended before the answer arrived"));
    } catch (error) {
      settleAll(error instanceof SessionError ? error : new SessionError("TRANSPORT_FAILED", String(error?.message ?? error)));
    }
  })();

  /** Sends one envelope that expects an answer, and waits for the answer on its slot. */
  async function exchange(partition, verb, payload) {
    if (readerFault !== null) throw readerFault;
    if (ended) fail("SESSION_CLOSED", "the session has ended");
    if (partition === 'Lifecycle') payload = lifecyclePayloadValue(verb, payload);
    if (partition === 'ReplayControl') payload = replayPayloadValue(verb, payload, resourceCeilings);
    issued += 1;
    const correlationId = `${verb}-${issued}`;
    const slot = pool.allocate(correlationId);

    let timer = null;
    const answer = new Promise((resolve, reject) => {
      waiters.set(slot, { resolve, reject, verb });
      if (requestTimeoutMs !== undefined) {
        timer = setTimeout(() => {
          waiters.delete(slot);
          if (pool.correlationFor(slot) !== undefined) pool.release(slot);
          reject(new SessionError("REQUEST_TIMED_OUT", `${verb} was not answered within ${requestTimeoutMs}ms`));
        }, requestTimeoutMs);
        timer.unref?.();
      }
    });

    try {
      await transport.send(codec.encode({
        kind: { partition, verb },
        correlation: slot,
        payload,
      }));
    } catch (error) {
      waiters.delete(slot);
      if (pool.correlationFor(slot) !== undefined) pool.release(slot);
      if (timer !== null) clearTimeout(timer);
      throw error;
    }

    let answered;
    try {
      answered = await answer;
    } finally {
      if (timer !== null) clearTimeout(timer);
    }
    if (leavesPagesOpen(answered)) closePages(slot);
    if (leavesLensOpen(verb, answered)) closeLens(slot);
    return answered;
  }

  /**
   * Ends the lens an accepted one-shot `ReplayStart` left open on `slot`, with one
   * `ReplayEnd` on that key. The slot returns when the end's `ReplayResult` arrives — never on this
   * side's send — so a new request cannot reuse the number while the daemon still holds the lens.
   */
  function closeLens(slot) {
    const free = () => {
      waiters.delete(slot);
      if (pool.correlationFor(slot) !== undefined) pool.release(slot);
    };
    if (readerFault !== null || ended) {
      free();
      return;
    }
    waiters.set(slot, { resolve: () => {}, reject: () => {}, verb: "ReplayEnd" });
    transport.send(codec.encode({
      kind: { partition: "ReplayControl", verb: "ReplayEnd" },
      correlation: slot,
      payload: REPLAY_END_BODY,
    })).catch(free);
  }

  function closePages(slot) {
    const free = () => {
      waiters.delete(slot);
      if (pool.correlationFor(slot) !== undefined) pool.release(slot);
    };
    if (readerFault !== null || ended) {
      free();
      return;
    }
    waiters.set(slot, {
      resolve: (answer) => { if (!closesPages(answer)) unmatched.push(answer); },
      reject: () => {},
    });
    transport.send(codec.encode({
      kind: { partition: "Query", verb: "QueryClose" },
      correlation: slot,
      payload: null,
    })).catch(free);
  }

  function hold(label, releaseOn = null) {
    issued += 1;
    const slot = pool.allocate(`${label}-${issued}`);
    const queue = [];
    const waiting = [];
    let pagesOpen = false;
    let unanswered = 0;
    let closing = null;
    let subscribed = false;
    let opening = false;
    let unsubscribing = false;
    let requests = 0;
    let leaving = false;
    const free = () => {
      held.delete(slot);
      if (pool.correlationFor(slot) !== undefined) pool.release(slot);
    };
    const letGo = () => {
      if (requests === 0 && (unsubscribing || !subscribed)) free();
    };
    const stream = {
      releaseOn,
      deliver(envelope) {
        const verb = envelope.kind.verb;
        if (verb === "QueryResult") {
          unanswered = Math.max(0, unanswered - 1);
          pagesOpen = leavesPagesOpen(envelope);
          if (closing !== null) {
            closing(envelope);
            return;
          }
        }
        if (verb === "SubscribeAck") {
          requests = Math.max(0, requests - 1);
          if (opening) {
            opening = false;
            subscribed = envelope.payload === 1n;
          }
        }
        if (verb === "SubscriptionEnded") subscribed = false;
        const next = waiting.shift();
        if (next === undefined) queue.push(envelope);
        else next(envelope);
        if (verb === releaseOn) leaving = true;
        if (leaving || unsubscribing) letGo();
      },
      end(error) {
        for (const next of waiting.splice(0)) next(undefined, error);
      },
      abandon() {
        if (closing !== null || leaving) free();
      },
    };
    held.set(slot, stream);
    return {
      slot,
      /** Sends one envelope on this slot. Nothing is awaited: answers arrive through `next`. */
      async send(partition, verb, payload) {
        if (partition === 'Lifecycle') payload = lifecyclePayloadValue(verb, payload);
        if (readerFault !== null) throw readerFault;
        if (ended) fail("SESSION_CLOSED", "the session has ended");
        const bytes = codec.encode({ kind: { partition, verb }, correlation: slot, payload });
        if (partition === "Query" && verb === "Query") unanswered += 1;
        if (partition === "Subscription") {
          requests += 1;
          if (verb === "Subscribe") {
            subscribed = true;
            opening = true;
          }
          if (verb === "Unsubscribe") unsubscribing = true;
        }
        await transport.send(bytes);
      },
      /**
       * The next envelope on this slot, or `null` if none arrives within `waitMs`.
       * Infinity waits for an envelope or session closure without a timer.
       *
       * A timeout answers `null` rather than throwing, because "no frame yet" is an ordinary
       * state of a credited stream and not a fault — the caller is what knows whether it is one.
       */
      next(waitMs = 5000) {
        if (queue.length > 0) return Promise.resolve(queue.shift());
        if (readerFault !== null) return Promise.reject(readerFault);
        if (waitMs <= 0) return Promise.resolve(null);
        return new Promise((resolve, reject) => {
          let timer = null;
          const deliver = (envelope, error) => {
            if (timer !== null) clearTimeout(timer);
            if (error !== undefined) reject(error);
            else resolve(envelope);
          };
          waiting.push(deliver);
          if (waitMs !== Infinity) timer = setTimeout(() => {
            const at = waiting.indexOf(deliver);
            if (at !== -1) waiting.splice(at, 1);
            resolve(null);
          }, waitMs);
          timer?.unref?.();
        });
      },
      release() {
        if (!held.has(slot) || closing !== null || leaving) return;
        if (readerFault !== null || ended) {
          free();
          return;
        }
        if (subscribed || requests > 0) {
          leaving = true;
          if (subscribed && !unsubscribing) {
            unsubscribing = true;
            requests += 1;
            transport.send(codec.encode({
              kind: { partition: "Subscription", verb: "Unsubscribe" },
              correlation: slot,
              payload: null,
            })).catch(free);
          }
          letGo();
          return;
        }
        if (!pagesOpen && unanswered === 0) {
          free();
          return;
        }
        let sent = false;
        closing = (answer) => {
          if (unanswered > 0) return;
          if (!sent) {
            if (!pagesOpen) {
              free();
              return;
            }
            sent = true;
            unanswered += 1;
            transport.send(codec.encode({
              kind: { partition: "Query", verb: "QueryClose" },
              correlation: slot,
              payload: null,
            })).catch(free);
            return;
          }
          if (!closesPages(answer)) unmatched.push(answer);
          free();
        };
        closing(null);
      },
    };
  }

  const helloAck = await exchange("SessionMechanics", "Hello", helloBody(options.hello));
  if (helloAck.kind.verb !== "HelloAck") {
    fail("ESTABLISHMENT_UNEXPECTED", `establishment expected HelloAck and received ${helloAck.kind.verb}`);
  }
  if (Array.isArray(helloAck.payload) && helloAck.payload[0] === 2n) {
    const { code, message, hint } = declarationResultFromValue(helloAck.payload).diagnostics[0];
    const error = new SessionError("ESTABLISHMENT_REJECTED",
      `[${code}] ${message}${hint === null ? "" : `\n${hint}`}`);
    error.cause = helloAck.payload[1];
    try { await transport.close(); } catch {   }
    throw error;
  }
  const established = readEstablished(helloAck.payload);

  return Object.freeze({
    established,

    /** Envelopes that arrived on a slot nobody held. */
    get unmatched() {
      return [...unmatched];
    },

    /** How many exchanges are live. */
    get liveCorrelations() {
      return pool.live;
    },

    async declare(command) {
      if (command === null || typeof command !== "object") {
        fail("COMMAND_UNEXPECTED", "declare takes one DeclarationCommand; there is no raw verb arm");
      }
      const verb = command.kind;
      const payload = declarationPayloadValue(command, { context: "mutation" });
      const result = await exchange("Declaration", verb, payload);
      if (result.kind.verb !== "CommandResult") {
        fail("RESULT_UNEXPECTED", `${verb} was answered with ${result.kind.verb}`);
      }
      return declarationResultFromValue(result.payload, verb);
    },

    /** Reads one complete immutable authoring snapshot over a held correlation. */
    async authoringSnapshot(scope, pageLimit, upto) {
      if (!Number.isSafeInteger(pageLimit) || pageLimit <= 0) {
        fail("PAGE_LIMIT", "authoring snapshot pageLimit must be a positive safe integer");
      }
      const stream = hold("authoring-snapshot");
      const args = authoringSnapshotArgumentsValue(scope);
      const items = [];
      let anchor = null;
      let cursor;
      try {
        for (;;) {
          const page = { limit: BigInt(pageLimit) };
          if (cursor !== undefined) page.cursor = cursor;
          await stream.send("Query", "Query", {
            name: "authoring-snapshot",
            args,
            page,
            ...(upto === undefined ? {} : { upto }),
          });
          const envelope = await stream.next(requestTimeoutMs ?? 5000);
          if (envelope === null) fail("QUERY_TIMED_OUT", "authoring-snapshot produced no page");
          if (envelope.kind.verb !== "QueryResult") {
            fail("RESULT_UNEXPECTED", `authoring-snapshot was answered with ${envelope.kind.verb}`);
          }
          const decoded = authoringSnapshotQueryResultFromValue(envelope.payload);
          if (decoded.status === "rejected") return decoded;
          if (anchor === null) anchor = decoded.value.anchor;
          else if (!sameAuthoringSnapshotAnchor(anchor, decoded.value.anchor)) {
            fail("QUERY_ANCHOR_MIXED", "authoring-snapshot changed anchor between pages");
          }
          items.push(...decoded.value.items);
          if (decoded.value.terminal === "Complete") {
            return Object.freeze({
              status: "accepted",
              value: Object.freeze({ anchor, commands: Object.freeze(items) }),
            });
          }
          if (decoded.value.terminal === "Diagnostic") {
            return Object.freeze({
              status: "partial",
              anchor,
              items: Object.freeze(items),
              diagnostic: decoded.value.diagnostic,
            });
          }
          cursor = decoded.value.next;
        }
      } finally {
        stream.release();
      }
    },

    /** Opens the credit-controlled semantic commit stream after one snapshot cursor. */
    async authoringCommits(scope, after) {
      const stream = subscriptionChannel(hold("authoring-commits", "SubscriptionEnded"));
      const args = authoringCommitArgumentsValue(scope, after);
      await stream.send("Subscription", "Subscribe", subscribeValue({
        target: "authoring-commits",
        args,
      }));
      const opening = await stream.untilAnswer(requestTimeoutMs ?? 5000);
      if (opening === null) {
        stream.release();
        fail("SUBSCRIPTION_TIMED_OUT", "authoring-commits produced no SubscribeAck");
      }
      if (opening.kind.verb !== "SubscribeAck") {
        stream.release();
        fail("RESULT_UNEXPECTED", `authoring-commits was answered with ${opening.kind.verb}`);
      }
      if (opening.payload !== 1n) {
        stream.release();
        const message = Array.isArray(opening.payload)
          ? opening.payload[1]?.message ?? "authoring-commits open was rejected"
          : "authoring-commits open did not carry Accepted";
        fail("SUBSCRIPTION_REJECTED", message);
      }

      let closed = false;
      const handle = {
        scope,
        after,
        async grant(frames) {
          if (closed) fail("SUBSCRIPTION_CLOSED", "authoring-commits is closed");
          await stream.send("Subscription", "Credit", creditValue({ frames }));
        },
        async next(waitMs = requestTimeoutMs ?? 5000) {
          for (;;) {
            const envelope = await stream.next(waitMs);
            if (envelope === null) return null;
            if (envelope.kind.verb === "SubscribeAck") {
              if (!creditAccepted(envelope.payload, fail)) {
                fail("SUBSCRIPTION_CREDIT_REJECTED", "authoring-commits credit was rejected");
              }
              continue;
            }
            if (envelope.kind.verb === "SubscriptionEnded") {
              closed = true;
              return subscriptionEndedFromValue(envelope.payload);
            }
            if (envelope.kind.verb !== "Frame") {
              fail("RESULT_UNEXPECTED", `authoring-commits received ${envelope.kind.verb}`);
            }
            const frame = subscriptionFrameFromValue(envelope.payload);
            if (frame.arm !== "Credit") {
              fail("SUBSCRIPTION_DISCIPLINE", "authoring-commits requires Credit frames");
            }
            return Object.freeze({
              kind: "Commit",
              origin: frame.origin,
              value: recordRegistrations["authoring-commits"].item(frame.payload),
            });
          }
        },
        async unsubscribe() {
          if (closed) return;
          await stream.send("Subscription", "Unsubscribe", null);
          closed = true;
          stream.release();
        },
        release() {
          closed = true;
          stream.release();
        },
      };
      return Object.freeze(handle);
    },

    /** Sends any verb and returns the answering envelope, for exchanges with no shorthand. */
    exchange,

    /** Holds one correlation open for a stream of envelopes — what a subscription needs. */
    hold,

    /**
     * The event-injection partition: `inject(request)` sends one `Inject {mount, payload,
     * idempotency}` and returns `Result<InjectionAccepted>`. The target is an export mount with a
     * bound request role, never an actor or a port; see `interaction.js` `inject`, the one path.
     */
    interactions: Object.freeze({
      inject: (request) => inject({ exchange }, request),
    }),

    replay: Object.freeze({
      async start({ from, pace }) {
        const stream = hold("replay-lens");
        const ask = async (verb, payload) => {
          await stream.send("ReplayControl", verb, replayPayloadValue(verb, payload, resourceCeilings));
          const answer = await stream.next(requestTimeoutMs ?? 5000);
          if (answer === null) fail("REQUEST_TIMED_OUT", `${verb} was not answered`);
          return replayResult(answer, verb);
        };
        let opened;
        try {
          opened = await ask("ReplayStart", {
            arrangement: { kind: "Observational", from: replayTargetFromCheckpoint(from) },
            pace: replayPace(pace),
          });
        } catch (error) {
          stream.release();
          throw error;
        }
        if (opened.status !== "accepted") {
          stream.release();
          return opened;
        }
        let turn = Promise.resolve();
        let ended = false;
        const inTurn = (work) => {
          const next = turn.then(work);
          turn = next.catch(() => {});
          return next;
        };
        const lens = Object.freeze({
          /** The lens's correlation key — what a read names to go through this lens. */
          correlation: stream.slot,
          rewind: ({ to, pace }) => inTurn(async () => {
            if (ended) fail("REPLAY_LENS_ENDED", "an ended lens takes no rewind");
            return ask("ReplayRewind", to === undefined
              ? { pace: replayPace(pace) }
              : { to: replayTargetFromCheckpoint(to), pace: replayPace(pace) });
          }),
          end: () => inTurn(async () => {
            if (ended) fail("REPLAY_LENS_ENDED", "the lens has ended");
            const result = await ask("ReplayEnd", REPLAY_END_BODY);
            ended = true;
            stream.release();
            return result;
          }),
        });
        return Object.freeze({ status: "accepted", value: lens });
      },
    }),

    /**
     * Says goodbye and closes.
     *
     * `Goodbye` is one-way, so it takes no slot and there is nothing
     * to wait for. A slot taken for it would have no moment of return and would leak for the
     * session's life.
     */
    async goodbye() {
      if (!ended) {
        await transport.send(codec.encode({
          kind: { partition: "SessionMechanics", verb: "Goodbye" },
          correlation: 0,
          payload: null,
        }));
      }
      await transport.close();
      await reading;
    },

    async close() {
      await transport.close();
      await reading;
    },
  });
}

function replayPayloadValue(verb, payload, ceilings) {
  if (verb === "ReplayStart") return replayStartValue(payload, ceilings);
  if (verb === "ReplayRewind") return replayRewindValue(payload, ceilings);
  if (verb === "ReplayEnd") {
    if (payload !== REPLAY_END_BODY) fail("REPLAY_END_BODY", "ReplayEnd has no body");
    return payload;
  }
  return fail("REPLAY_VERB", `${String(verb)} is not a ReplayControl request`);
}

/** A client pace names checkpoints where the codec names coordinates. */
function replayPace(pace) {
  if (pace !== null && typeof pace === "object" && pace.kind === "Step") {
    return { kind: "Step", upto: replayTargetFromCheckpoint(pace.upto) };
  }
  return pace;
}

/** ReplayResult is the Declaration partition's CommandResult sum. */
function replayResult(answer, verb) {
  if (answer.kind.verb !== "ReplayResult") {
    fail("RESULT_UNEXPECTED", `${verb} was answered with ${answer.kind.verb}`);
  }
  return declarationResultFromValue(answer.payload, verb);
}

/**
 * Whether an arriving answer leaves its page sequence open on the daemon.
 *
 * `More` is the one terminal after which the daemon still holds the sequence and its key
 * `Complete`, a `Diagnostic` and a rejection all end it.
 */
function leavesPagesOpen(envelope) {
  const body = envelope?.payload;
  return envelope?.kind?.verb === "QueryResult" && Array.isArray(body) && body[0] === 1n
    && Array.isArray(body[1]?.terminal) && body[1].terminal[0] === 1n;
}

/**
 * Whether an answer leaves a replay lens open on its key: an accepted `ReplayResult`
 * answering a `ReplayStart`. Every other `ReplayResult` on the one-shot path is the last envelope on
 * its key — the key is fresh, so a rewind or an end sent there finds no lens and is refused.
 */
function leavesLensOpen(verb, envelope) {
  if (verb !== "ReplayStart" || envelope?.kind?.verb !== "ReplayResult") return false;
  const body = envelope.payload;
  return body === 1n || (Array.isArray(body) && body[0] === 1n);
}

/** Whether an answer is the terminal page a `QueryClose` is answered with: a `Diagnostic`. */
function closesPages(envelope) {
  const body = envelope?.payload;
  return envelope?.kind?.verb === "QueryResult" && Array.isArray(body) && body[0] === 1n
    && Array.isArray(body[1]?.terminal) && body[1].terminal[0] === 3n;
}

function bytesEqual(left, right) {
  if (!(left instanceof Uint8Array) || !(right instanceof Uint8Array) || left.length !== right.length) {
    return false;
  }
  return left.every((byte, index) => byte === right[index]);
}

function sameRevision(left, right) {
  return left?.kind === right?.kind
    && (left?.kind === "Absent" || bytesEqual(left?.revision, right?.revision));
}

function sameScope(left, right) {
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
  return left.every((segment, index) => {
    const other = right[index];
    return segment?.name === other?.name
      && segment?.of === other?.of
      && segment?.key === other?.key;
  });
}

function sameEnvironment(left, right) {
  return bytesEqual(left?.declarationSchema, right?.declarationSchema)
    && bytesEqual(left?.specSet, right?.specSet);
}

function sameAuthoringSnapshotAnchor(left, right) {
  return sameScope(left?.scope, right?.scope)
    && sameRevision(left?.authoringRevision, right?.authoringRevision)
    && sameRevision(left?.topologyRevision, right?.topologyRevision)
    && left?.cursor === right?.cursor
    && sameEnvironment(left?.environment, right?.environment);
}
