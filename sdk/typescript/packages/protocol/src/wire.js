
import { decodeValueBeta, encodeValueBeta } from "./value.js";
import { Partition, StableVerb, RESERVED_CAPABILITY_PARTITION_TAG as RESERVED_PARTITION, RESERVED_CAPABILITY_VERB_TAG_FIRST as RESERVED_FIRST, RESERVED_CAPABILITY_VERB_TAG_COUNT as RESERVED_COUNT } from "./internal/closed-tables.js";

export const HEADER_BYTES = 12;
export const INITIAL_PROTOCOL_VERSION = 1;
export const PAYLOAD_VERSION_TAG_BYTES = 2;
export const INITIAL_PAYLOAD_VERSION_TAG = 1;
export const MAXIMUM_LIVE_CORRELATIONS = 4096;

export const RESERVED_CAPABILITY_PARTITION_TAG = RESERVED_PARTITION;
export const RESERVED_CAPABILITY_VERB_TAG_FIRST = RESERVED_FIRST;
export const RESERVED_CAPABILITY_VERB_TAG_COUNT = RESERVED_COUNT;

const MAX_U16 = 0xffff;
const MAX_U32 = 0xffff_ffff;

export const PARTITION_TAGS = Object.freeze(
  Object.fromEntries(Partition.map(({ name, tag }) => [name, tag])),
);

/** Verb tags. One global space across all partitions; zero is excluded. In tag order. */
export const VERB_TAGS = Object.freeze(
  Object.fromEntries([...StableVerb].sort((left, right) => left.tag - right.tag).map(({ name, tag }) => [name, tag])),
);

/** The one partition each verb belongs to, which the verb tag alone already determines. */
export const VERB_PARTITION = Object.freeze(
  Object.fromEntries(StableVerb.map(({ name, partition }) => [name, partition])),
);

const VERB_BY_TAG = Object.freeze(
  Object.fromEntries(Object.entries(VERB_TAGS).map(([verb, tag]) => [tag, verb])),
);
const PARTITION_BY_TAG = Object.freeze(
  Object.fromEntries(Object.entries(PARTITION_TAGS).map(([partition, tag]) => [tag, partition])),
);

export class WireError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new WireError(code, message);
}

/** The forty-eight row table, materialised for inspection. */
export function tagTable() {
  return Object.entries(VERB_TAGS).map(([verb, verbTag]) => ({
    partition: VERB_PARTITION[verb],
    partitionTag: PARTITION_TAGS[VERB_PARTITION[verb]],
    verb,
    verbTag,
  }));
}

/** Resolves a partition and verb pair to its two head tags. */
export function tagsFor(partition, verb) {
  const partitionTag = PARTITION_TAGS[partition];
  if (partitionTag === undefined) fail("PARTITION_UNKNOWN", `${partition} is not a declared partition`);
  const verbTag = VERB_TAGS[verb];
  if (verbTag === undefined) fail("VERB_UNKNOWN", `${verb} is not a declared verb`);
  if (VERB_PARTITION[verb] !== partition) {
    fail("PARTITION_VERB_MISMATCH", `${verb} belongs to ${VERB_PARTITION[verb]}, not to ${partition}`);
  }
  return { partitionTag, verbTag };
}

/**
 * The inverse of `tagsFor`.
 *
 * Reserved and unassigned tags land in the same rejection:
 * a reader must not learn from the refusal which of the two it was.
 */
export function namesFor(partitionTag, verbTag) {
  if (partitionTag === 0) fail("PARTITION_TAG_RESERVED", "partition tag 0 is reserved");
  if (verbTag === 0) fail("VERB_TAG_RESERVED", "verb tag 0 is reserved");
  const partition = PARTITION_BY_TAG[partitionTag];
  if (partition === undefined) fail("PARTITION_TAG_UNASSIGNED", `partition tag ${partitionTag} is unassigned`);
  const verb = VERB_BY_TAG[verbTag];
  if (verb === undefined) fail("VERB_TAG_UNASSIGNED", `verb tag ${verbTag} is unassigned`);
  if (VERB_PARTITION[verb] !== partition) {
    fail("PARTITION_VERB_MISMATCH", `verb tag ${verbTag} belongs to ${VERB_PARTITION[verb]}, not to ${partition}`);
  }
  return { partition, verb };
}

function assertUnsigned(value, maximum, code, label) {
  if (!Number.isSafeInteger(value) || value < 0 || value > maximum) {
    fail(code, `${label} must be an unsigned integer within its width`);
  }
}

/**
 * Encodes the twelve-byte head.
 *
 * Zero is excluded from all three tag positions rather than merely unassigned: a zero-filled or
 * truncated buffer must reject rather than select a live value, and that only holds if zero is
 * invalid at every position zero-filling can reach.
 */
export function encodeEnvelopeHeader(header) {
  const { protocolVersion, partition, verb, correlationId, payloadLength } = header;
  assertUnsigned(protocolVersion, MAX_U16, "PROTOCOL_VERSION_INVALID", "protocolVersion");
  if (protocolVersion === 0) fail("PROTOCOL_VERSION_RESERVED", "protocol version 0 is reserved");
  if (typeof correlationId !== "number") {
    fail(
      "CORRELATION_NOT_INTEGER",
      "the head carries a u32 correlation; the SDK's string CorrelationId has no mapping onto it yet",
    );
  }
  assertUnsigned(correlationId, MAX_U32, "CORRELATION_INVALID", "correlationId");
  assertUnsigned(payloadLength, MAX_U32, "PAYLOAD_LENGTH_INVALID", "payloadLength");

  const { partitionTag, verbTag } = tagsFor(partition, verb);
  const bytes = new Uint8Array(HEADER_BYTES);
  const view = new DataView(bytes.buffer);
  view.setUint16(0, protocolVersion, false);
  view.setUint8(2, partitionTag);
  view.setUint8(3, verbTag);
  view.setUint32(4, correlationId, false);
  view.setUint32(8, payloadLength, false);
  return bytes;
}

/**
 * Decodes the twelve-byte head.
 *
 * Boundary-decodable by construction: every position has a width fixed by its value space
 * rather than by the value it carries, so this needs no lookahead and cannot be ambiguous about
 * where the payload starts.
 */
export function decodeEnvelopeHeader(input) {
  if (!(input instanceof Uint8Array)) fail("INPUT_NOT_BYTES", "input must be a Uint8Array");
  if (input.length < HEADER_BYTES) fail("HEADER_TRUNCATED", "fewer than twelve header bytes");

  const view = new DataView(input.buffer, input.byteOffset, input.byteLength);
  const protocolVersion = view.getUint16(0, false);
  if (protocolVersion === 0) fail("PROTOCOL_VERSION_RESERVED", "protocol version 0 is reserved");
  const { partition, verb } = namesFor(view.getUint8(2), view.getUint8(3));

  return {
    protocolVersion,
    partition,
    verb,
    correlationId: view.getUint32(4, false),
    payloadLength: view.getUint32(8, false),
    headerBytes: HEADER_BYTES,
  };
}

/**
 * Splits a frame into its head and its payload bytes.
 *
 * A frame that declares more than it carries is truncated, and one that carries more leaves
 * bytes with no owner. Both are settled here, before anything reads from the payload.
 */
export function splitEnvelopeFrame(frame) {
  const header = decodeEnvelopeHeader(frame);
  const payload = frame.subarray(HEADER_BYTES);
  if (payload.length < header.payloadLength) fail("PAYLOAD_TRUNCATED", "frame is shorter than its declared payload");
  if (payload.length > header.payloadLength) {
    fail("PAYLOAD_TRAILING_BYTES", "frame carries bytes past its declared payload");
  }
  return { header, payload };
}

export const BODYLESS_VERBS = new Set(StableVerb.filter(verb => verb.body === "Absent").map(verb => verb.name));

/**
 * Encodes a complete frame: the twelve-byte head followed by one `Value`.
 *
 * Nothing stands between them. An envelope body is not automatically an
 * `EncodedPayload`, so the payload version tag is not an envelope-level prefix; a payload that
 * carries one carries it inside its own encoding, put there by the construct that owns it.
 *
 * The payload is encoded first, because the head has to state its length and cannot be written
 * until that length is known — which is what makes the head fixed-width rather than streamed.
 */
export function encodeEnvelopeFrame(envelope, resourceCeilings) {
  const {
    protocolVersion = INITIAL_PROTOCOL_VERSION,
    partition,
    verb,
    correlationId,
    payload,
  } = envelope;

  const valueBytes = BODYLESS_VERBS.has(verb) && payload === null
    ? new Uint8Array(0)
    : encodeValueBeta(payload, resourceCeilings);
  const header = encodeEnvelopeHeader({
    protocolVersion,
    partition,
    verb,
    correlationId,
    payloadLength: valueBytes.length,
  });

  const frame = new Uint8Array(HEADER_BYTES + valueBytes.length);
  frame.set(header, 0);
  frame.set(valueBytes, HEADER_BYTES);
  return frame;
}

/** Decodes a complete frame. */
export function decodeEnvelopeFrame(frame, resourceCeilings) {
  const { header, payload: payloadBytes } = splitEnvelopeFrame(frame);
  return {
    protocolVersion: header.protocolVersion,
    partition: header.partition,
    verb: header.verb,
    correlationId: header.correlationId,
    payload: BODYLESS_VERBS.has(header.verb) && payloadBytes.length === 0
      ? null
      : decodeValueBeta(payloadBytes, resourceCeilings),
  };
}

/**
 * The total byte length of the frame this head begins.
 *
 * A byte transport has to know where one frame ends before it can hand a whole frame to a
 * codec, and that is the only thing about the head it needs to know. Exposing it as a function
 * keeps the length field's position here rather than copied into every transport — a transport
 * reading `bytes 8..12` by hand is a second place that has to be right about the layout.
 *
 * The declared length is attacker-controlled and `u32` wide, so a caller must bound it before
 * allocating. That bound is the caller's, for the same reason the value ceilings are.
 */
export function frameByteLength(head) {
  if (!(head instanceof Uint8Array)) fail("INPUT_NOT_BYTES", "input must be a Uint8Array");
  if (head.length < HEADER_BYTES) fail("HEADER_TRUNCATED", "fewer than twelve header bytes");
  const view = new DataView(head.buffer, head.byteOffset, head.byteLength);
  return HEADER_BYTES + view.getUint32(8, false);
}

/**
 * Reads only what routing needs, without decoding the payload.
 *
 * A router picks a handler from the partition and verb, and paying for a payload it may be
 * about to refuse is wasted work. The fixed-width head is what makes skipping it possible.
 */
export function peekEnvelopeRoute(frame) {
  const header = decodeEnvelopeHeader(frame);
  return {
    partition: header.partition,
    verb: header.verb,
    correlationId: header.correlationId,
    payloadLength: header.payloadLength,
  };
}
