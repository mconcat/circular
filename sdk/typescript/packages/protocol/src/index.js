/**
 * Runtime helpers for Circular's transport-neutral protocol values.
 *
 * The wire schema itself remains the declarations in this package. These helpers
 * construct or inspect those values; they do not introduce a JSON graph model or
 * a second command representation.
 */

import { decodeEnvelopeFrame, encodeEnvelopeFrame } from "./wire.js";

export {
  BOUNDARY_PORT_ID_VERSION,
  SYNTH_BOUNDARY_LOCAL_VERSION,
  deriveBoundaryPortId,
  deriveSynthBoundaryLocal,
} from "./boundary-port.js";

/**
 * The framing facts a byte transport needs, and only those.
 *
 * A transport has to know where one frame ends before it can hand a whole frame to a codec.
 * Publishing this pair keeps the length field's position in one place; a transport reading
 * `bytes 8..12` by hand would be a second place that has to be right about the layout.
 */
export { HEADER_BYTES, frameByteLength } from "./wire.js";

/**
 * The port Flow/Shape carrier reader and writer — the TypeScript spelling of
 * `crates/protocol/src/port_type.rs`. `authoring.actor-ports` rows arrive decoded by it.
 */
export { PORT_BASE_SHAPES, decodePortShape, decodePortFlow, encodePortFlow } from "./port-type.js";

export { CircularUInt } from "./value.js";
export { valueKey, sameValue } from "./internal/value-equal.js";

export {
  DEAD_LETTER_REASONS,
  deadLetterReasonFromValue,
  deadLetterTargetFromValue,
  deadLetterItemFromValue,
  preprocessFailurePoint,
  decodeDeadLetters,
  INCARNATION_PHASES,
  instanceTransitionFromValue,
  decodeInstanceTransitions,
  projectActiveInstanceFacts,
  actorCreateInputSlotFromValue,
  actorCreateInputCatalogEntryFromValue,
  decodeActorCreateInputs,
} from "./observation-rows.js";
export { lifecycleResultFromValue } from "./lifecycle-payload.js";

export {
  TIMELINE_BINS_QUERY,
  TIMELINE_AT_QUERY,
  TIMELINE_MAX_BINS,
  TIMELINE_MARK_KINDS,
  timelineCoverage,
  timelineBinsArgsValue,
  timelineAtArgsValue,
  timelineBinsFromValue,
  timelineAtFromValue,
} from "./timeline-values.js";
import { validateResourceCeilings } from "./value.js";
import { Partition, StableVerb } from "./internal/closed-tables.js";

/** Stable protocol partitions in canonical declaration order (`Partition::ALL`, `@circular/protocol/tables`). */
export const stablePartitions = Object.freeze(Partition.map(partition => partition.name));

/**
 * Stable verbs, partition by partition in declaration order. Experimental verbs are negotiated
 * separately and are not listed. Read from `@circular/protocol/tables`.
 */
export const stableVerbs = Object.freeze(StableVerb.map(verb => verb.name));

/** Constructs the normal success channel. */
export function accepted(value) {
  return Object.freeze({ status: "accepted", value });
}

/** Constructs the normal structured rejection channel. */
export function rejected(reason, diagnostics) {
  if (!Array.isArray(diagnostics) || diagnostics.length === 0) {
    throw new TypeError("Circular rejection diagnostics must be a non-empty array");
  }
  return Object.freeze({
    status: "rejected",
    reason,
    diagnostics: Object.freeze([...diagnostics]),
  });
}

/** Returns whether a value is Circular's normal accepted-result variant. */
export function isAccepted(value) {
  return value !== null && typeof value === "object" && value.status === "accepted";
}

/** Returns whether a value is Circular's normal rejected-result variant. */
export function isRejected(value) {
  return value !== null && typeof value === "object" && value.status === "rejected";
}

/** Constructs one transport-independent envelope. */
export function envelope(partition, verb, correlation, payload) {
  return Object.freeze({
    kind: Object.freeze({ partition, verb }),
    correlation,
    payload,
  });
}

/** Defines a feature-owned query descriptor without putting type witnesses on the wire. */
export function defineQuery(descriptor) {
  if (descriptor.paging !== "none" && descriptor.paging !== "cursor") {
    throw new TypeError(`Unknown Circular query paging policy: ${String(descriptor.paging)}`);
  }
  return Object.freeze({
    name: descriptor.name,
    paging: descriptor.paging,
    anchorKind: descriptor.anchorKind,
  });
}

/** Defines a feature-owned subscription descriptor. */
export function defineSubscription(descriptor) {
  if (!["lossless", "conflated", "credit"].includes(descriptor.discipline)) {
    throw new TypeError(`Unknown Circular subscription discipline: ${String(descriptor.discipline)}`);
  }
  return Object.freeze({ name: descriptor.name, discipline: descriptor.discipline });
}

/**
 * A zero-copy codec for an in-process transport.
 *
 * This is intentionally not a JSON codec. Remote transports should provide the
 * negotiated codec (for example generated Cap'n Proto bindings) at `establish`.
 */
export const identityEnvelopeCodec = Object.freeze({
  encode(value) {
    return value;
  },
  decode(value) {
    if (
      value === null ||
      typeof value !== "object" ||
      value.kind === null ||
      typeof value.kind !== "object" ||
      typeof value.kind.partition !== "string" ||
      typeof value.kind.verb !== "string" ||
      typeof value.correlation !== "string" ||
      !("payload" in value)
    ) {
      return rejected("Malformed", [
        Object.freeze({
          code: 0,
          message: "The in-process frame is not a complete Circular envelope.",
          hint: null,
          at: null,
        }),
      ]);
    }
    return Object.freeze({ status: "complete", envelope: value });
  },
});

/**
 * The wire codec: the one `establish` has been asking for.
 *
 * `identityEnvelopeCodec` above is for an in-process transport, and its own doc says a remote
 * transport supplies the negotiated codec instead. This is that codec — a twelve-byte head, a
 * payload version tag, and one canonical `Value` — so a session over a socket has something to
 * encode with.
 *
 * The ceilings are the caller's, and deliberately have no default: three of the four have no
 * fixed value, and a codec that supplied its own would be choosing on the caller's behalf.
 *
 * `decode` returns the same shape `identityEnvelopeCodec` does, so `establish` cannot tell the
 * two apart. A malformed frame is a rejection rather than a throw, because the session loop has
 * to stay in control of a bad frame rather than unwind through it.
 */
export function wireEnvelopeCodec(resourceCeilings) {
  validateResourceCeilings(resourceCeilings);
  return Object.freeze({
    encode(value) {
      return encodeEnvelopeFrame({
        partition: value.kind.partition,
        verb: value.kind.verb,
        correlationId: value.correlation,
        payload: value.payload,
      }, resourceCeilings);
    },
    decode(frame) {
      let decoded;
      try {
        decoded = decodeEnvelopeFrame(frame, resourceCeilings);
      } catch (error) {
        return rejected("Malformed", [
          Object.freeze({
            code: 0,
            message: `The frame is not a complete Circular envelope: ${error.code ?? "unknown"}.`,
            hint: null,
            at: null,
          }),
        ]);
      }
      return Object.freeze({
        status: "complete",
        envelope: envelope(
          decoded.partition,
          decoded.verb,
          decoded.correlationId,
          decoded.payload,
        ),
      });
    },
  });
}
