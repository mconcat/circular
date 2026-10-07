/**
 * The OwnerLocal transport's canonical frame profile.
 *
 * A byte stream has no boundaries in it, so something has to say where one message ends. This
 * is that something, and it is a **separate layer from the envelope**: the envelope's head
 * routes a message and the frame's head only carries it.
 *
 *   version:u8 · channel:u32be · operation:u8 · segment:u8 · body_len:u32be   = 11 bytes
 *
 * `crates/transport/src/canonical_frame.rs` is the counterpart and the source of every value
 * here. It was missing on this side entirely — the transport read envelopes straight off the
 * socket on the strength of the envelope head's own length, which works only if nothing wraps
 * them. Something does, so nothing this side sent could have been read.
 *
 * ## Why the envelope's own length is not enough
 *
 * It would be, for one envelope at a time on a stream nothing else uses. The frame layer buys
 * two things the envelope cannot give:
 *
 *   - **Segmentation.** One envelope may be larger than one frame, so it arrives in pieces.
 *     The envelope head declares the whole length, which tells a reader how much to wait for
 *     but not where the pieces begin.
 *   - **A channel.** Frames carry a connection-local channel so more than one exchange can
 *     share a stream. The daemon does not multiplex yet — it answers on channel `1` and does
 *     not read the channel back — so this side sends on channel `0`, the first identifier the
 *     Rust source issues, and does not depend on which one comes back.
 *
 * ## The two bounds are the profile's, not the caller's
 *
 * `maximumFrameBytes` had no default here once, on the reading that a limit the caller did not
 * choose is a limit the caller cannot reason about. That reading held while nothing published
 * one. The profile publishes both bounds now, so inheriting them is reading a contract rather
 * than accepting an arbitrary default — and a caller cannot raise them, because a message above
 * them is refused at the other end and so is not a message this transport can carry. Lowering is
 * still the caller's to do.
 */

import { MAX_REASSEMBLED_BODY_BYTES, MAX_SEGMENT_BODY_BYTES } from '@circular/protocol/tables';

export const OWNER_LOCAL_FRAMING_VERSION = 1;

/** `version:u8 · channel:u32be · operation:u8 · segment:u8 · body_len:u32be`. */
export const OWNER_LOCAL_FRAME_HEADER_BYTES = 11;

/** The largest opaque body one frame carries: 64 KiB (`MAX_SEGMENT_BODY_BYTES`, `@circular/protocol/tables`). */
export const OWNER_LOCAL_MAX_FRAME_BODY_BYTES = MAX_SEGMENT_BODY_BYTES;

/** The largest reassembled message: 16 MiB (`MAX_REASSEMBLED_BODY_BYTES`, `@circular/protocol/tables`). */
export const OWNER_LOCAL_MAX_MESSAGE_BYTES = MAX_REASSEMBLED_BODY_BYTES;

/** Frame operations. Only `data` is fragmentable. */
export const FRAME_OPERATIONS = Object.freeze({ data: 1, open: 2, close: 3 });

/** Segment tags. `whole` is one frame; the other three are a sequence. */
export const FRAME_SEGMENTS = Object.freeze({ whole: 1, first: 2, middle: 3, last: 4 });

const OPERATION_BY_TAG = Object.freeze(
  Object.fromEntries(Object.entries(FRAME_OPERATIONS).map(([name, tag]) => [tag, name])),
);
const SEGMENT_BY_TAG = Object.freeze(
  Object.fromEntries(Object.entries(FRAME_SEGMENTS).map(([name, tag]) => [tag, name])),
);

export class CanonicalFrameError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new CanonicalFrameError(code, message);
}

/**
 * The three shapes a frame cannot have, checked wherever a frame is built or read.
 *
 * Kept in one function rather than repeated at both ends: an encoder and a decoder that check
 * different things can produce a frame the other refuses, and the asymmetry only shows up
 * against another implementation.
 */
function validateFrame(operation, segment, bodyLength) {
  if (bodyLength > OWNER_LOCAL_MAX_FRAME_BODY_BYTES) {
    fail("FRAME_BODY_TOO_LARGE", `a frame body of ${bodyLength} bytes is past the profile's ${OWNER_LOCAL_MAX_FRAME_BODY_BYTES}`);
  }
  if (operation !== FRAME_OPERATIONS.data && segment !== FRAME_SEGMENTS.whole) {
    fail("FRAGMENTED_CONTROL", "only a data frame may be fragmented");
  }
  if (operation === FRAME_OPERATIONS.data && segment !== FRAME_SEGMENTS.whole && bodyLength === 0) {
    fail("EMPTY_FRAGMENT", "a fragment carries at least one byte");
  }
}

/** Encodes one frame. */
export function encodeCanonicalFrame({ channel, operation, segment, body }) {
  if (!Number.isSafeInteger(channel) || channel < 0 || channel > 0xffffffff) {
    fail("CHANNEL_WIDTH", "a channel identifier is a u32");
  }
  if (OPERATION_BY_TAG[operation] === undefined) fail("OPERATION_UNKNOWN", `operation ${operation} is unassigned`);
  if (SEGMENT_BY_TAG[segment] === undefined) fail("SEGMENT_UNKNOWN", `segment ${segment} is unassigned`);
  validateFrame(operation, segment, body.length);

  const frame = new Uint8Array(OWNER_LOCAL_FRAME_HEADER_BYTES + body.length);
  const view = new DataView(frame.buffer);
  frame[0] = OWNER_LOCAL_FRAMING_VERSION;
  view.setUint32(1, channel, false);
  frame[5] = operation;
  frame[6] = segment;
  view.setUint32(7, body.length, false);
  frame.set(body, OWNER_LOCAL_FRAME_HEADER_BYTES);
  return frame;
}

/**
 * Splits a message into the frames that carry it.
 *
 * The bound is judged on the **whole message including its envelope head**, before any frame is
 * built. Judging it after fragmenting would mean refusing at the last piece with earlier pieces
 * already sent, and taking them back is not a thing this layer can do.
 */
export function encodeMessageFrames(channel, message, options = {}) {
  const maximumMessageBytes = options.maximumMessageBytes ?? OWNER_LOCAL_MAX_MESSAGE_BYTES;
  if (!Number.isSafeInteger(maximumMessageBytes) || maximumMessageBytes <= 0
    || maximumMessageBytes > OWNER_LOCAL_MAX_MESSAGE_BYTES) {
    fail("MAXIMUM_MESSAGE_BYTES_INVALID", `maximumMessageBytes must be positive and at most ${OWNER_LOCAL_MAX_MESSAGE_BYTES}`);
  }
  if (message.length > maximumMessageBytes) {
    fail("MESSAGE_TOO_LARGE", `a message of ${message.length} bytes is past the ${maximumMessageBytes} admitted`);
  }

  if (message.length <= OWNER_LOCAL_MAX_FRAME_BODY_BYTES) {
    return [encodeCanonicalFrame({
      channel,
      operation: FRAME_OPERATIONS.data,
      segment: FRAME_SEGMENTS.whole,
      body: message,
    })];
  }

  const count = Math.ceil(message.length / OWNER_LOCAL_MAX_FRAME_BODY_BYTES);
  const frames = [];
  for (let index = 0; index < count; index += 1) {
    const start = index * OWNER_LOCAL_MAX_FRAME_BODY_BYTES;
    const segment = index === 0
      ? FRAME_SEGMENTS.first
      : (index + 1 === count ? FRAME_SEGMENTS.last : FRAME_SEGMENTS.middle);
    frames.push(encodeCanonicalFrame({
      channel,
      operation: FRAME_OPERATIONS.data,
      segment,
      body: message.subarray(start, start + OWNER_LOCAL_MAX_FRAME_BODY_BYTES),
    }));
  }
  return frames;
}

/**
 * Reads a byte stream into complete messages.
 *
 * Two jobs, in order: frames out of bytes, then a message out of frames. They are separate
 * because a chunk boundary is not a frame boundary and a frame boundary is not a message
 * boundary — collapsing either pair makes a partial read look like a malformed one.
 *
 * A refused sequence ends the reassembly rather than skipping the offending frame. After a
 * fault the reader no longer knows which frames belong to which message, so continuing would be
 * guessing at a boundary rather than reading one.
 */
export class OwnerLocalFrameReader {
  #buffered = [];
  #length = 0;
  #partial = null;
  #maximumMessageBytes;

  constructor(options = {}) {
    const maximumMessageBytes = options.maximumMessageBytes ?? OWNER_LOCAL_MAX_MESSAGE_BYTES;
    if (!Number.isSafeInteger(maximumMessageBytes) || maximumMessageBytes <= 0
      || maximumMessageBytes > OWNER_LOCAL_MAX_MESSAGE_BYTES) {
      fail("MAXIMUM_MESSAGE_BYTES_INVALID", `maximumMessageBytes must be positive and at most ${OWNER_LOCAL_MAX_MESSAGE_BYTES}`);
    }
    this.#maximumMessageBytes = maximumMessageBytes;
  }

  /** Adds bytes and returns every message they completed. */
  push(chunk) {
    this.#buffered.push(chunk);
    this.#length += chunk.length;

    const messages = [];
    for (;;) {
      if (this.#length < OWNER_LOCAL_FRAME_HEADER_BYTES) break;
      const joined = this.#join();
      if (joined[0] !== OWNER_LOCAL_FRAMING_VERSION) {
        this.#partial = null;
        fail("UNSUPPORTED_VERSION", `framing version ${joined[0]} is not this profile's`);
      }
      const view = new DataView(joined.buffer, joined.byteOffset, joined.byteLength);
      const operation = joined[5];
      const segment = joined[6];
      if (OPERATION_BY_TAG[operation] === undefined) {
        this.#partial = null;
        fail("OPERATION_UNKNOWN", `operation ${operation} is unassigned`);
      }
      if (SEGMENT_BY_TAG[segment] === undefined) {
        this.#partial = null;
        fail("SEGMENT_UNKNOWN", `segment ${segment} is unassigned`);
      }
      const bodyLength = view.getUint32(7, false);
      try {
        validateFrame(operation, segment, bodyLength);
      } catch (error) {
        this.#partial = null;
        throw error;
      }
      const total = OWNER_LOCAL_FRAME_HEADER_BYTES + bodyLength;
      if (joined.length < total) break;

      const body = joined.subarray(OWNER_LOCAL_FRAME_HEADER_BYTES, total);
      this.#buffered = [joined.subarray(total)];
      this.#length = this.#buffered[0].length;

      if (operation !== FRAME_OPERATIONS.data) continue;

      const message = this.#reassemble(segment, body);
      if (message !== null) messages.push(message);
    }
    return messages;
  }

  /** Bytes held that do not yet complete a frame. */
  get pending() {
    return this.#length;
  }

  /** Whether a message is half-arrived. */
  get collecting() {
    return this.#partial !== null;
  }

  #reassemble(segment, body) {
    const collecting = this.#partial !== null;
    if (!collecting && segment === FRAME_SEGMENTS.whole) {
      this.#checkMessageLength(body.length);
      return body;
    }
    if (!collecting && segment === FRAME_SEGMENTS.first) {
      this.#checkMessageLength(body.length);
      this.#partial = [body];
      return null;
    }
    if (collecting && (segment === FRAME_SEGMENTS.middle || segment === FRAME_SEGMENTS.last)) {
      const total = this.#partialLength() + body.length;
      this.#checkMessageLength(total);
      this.#partial.push(body);
      if (segment === FRAME_SEGMENTS.middle) return null;
      const message = new Uint8Array(total);
      let offset = 0;
      for (const part of this.#partial) {
        message.set(part, offset);
        offset += part.length;
      }
      this.#partial = null;
      return message;
    }
    this.#partial = null;
    fail(
      "UNEXPECTED_SEGMENT",
      `segment ${SEGMENT_BY_TAG[segment]} is not admitted while ${collecting ? "collecting" : "idle"}`,
    );
    return null;
  }

  #partialLength() {
    let total = 0;
    for (const part of this.#partial) total += part.length;
    return total;
  }

  #checkMessageLength(length) {
    if (length > this.#maximumMessageBytes) {
      this.#partial = null;
      fail("MESSAGE_TOO_LARGE", `a message of ${length} bytes is past the ${this.#maximumMessageBytes} admitted`);
    }
  }

  #join() {
    if (this.#buffered.length === 1) return this.#buffered[0];
    const joined = new Uint8Array(this.#length);
    let offset = 0;
    for (const part of this.#buffered) {
      joined.set(part, offset);
      offset += part.length;
    }
    this.#buffered = [joined];
    return joined;
  }
}
