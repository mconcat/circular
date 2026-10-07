import assert from "node:assert/strict";
import test from "node:test";

import {
  FRAME_OPERATIONS,
  FRAME_SEGMENTS,
  OWNER_LOCAL_FRAME_HEADER_BYTES,
  OWNER_LOCAL_FRAMING_VERSION,
  OWNER_LOCAL_MAX_FRAME_BODY_BYTES,
  OWNER_LOCAL_MAX_MESSAGE_BYTES,
  OwnerLocalFrameReader,
  encodeCanonicalFrame,
  encodeMessageFrames,
} from "@circular/client/canonical-frame";

function expectCode(fn, code) {
  try {
    fn();
  } catch (error) {
    assert.equal(error.code, code, `thrown: ${error.message}`);
    return;
  }
  assert.fail(`expected ${code} but nothing was thrown`);
}

const body = (length, fill = 0x5a) => new Uint8Array(length).fill(fill);

test("the header layout is the profile's, field by field", () => {
  const frame = encodeCanonicalFrame({
    channel: 0x01020304,
    operation: FRAME_OPERATIONS.data,
    segment: FRAME_SEGMENTS.whole,
    body: Uint8Array.from([0xaa, 0xbb]),
  });
  assert.deepEqual([...frame], [
    OWNER_LOCAL_FRAMING_VERSION,
    0x01, 0x02, 0x03, 0x04,
    FRAME_OPERATIONS.data,
    FRAME_SEGMENTS.whole,
    0x00, 0x00, 0x00, 0x02,
    0xaa, 0xbb,
  ]);
  assert.equal(OWNER_LOCAL_FRAME_HEADER_BYTES, 11);
});

test("a message that fits in one frame is whole, not a one-element sequence", () => {
  const frames = encodeMessageFrames(0, body(16));
  assert.equal(frames.length, 1);
  assert.equal(frames[0][6], FRAME_SEGMENTS.whole);
});

test("a message past one frame is first, middle, and last, in that order", () => {
  const message = body(OWNER_LOCAL_MAX_FRAME_BODY_BYTES * 2 + 7);
  const frames = encodeMessageFrames(0, message);
  assert.deepEqual(frames.map((frame) => frame[6]), [
    FRAME_SEGMENTS.first, FRAME_SEGMENTS.middle, FRAME_SEGMENTS.last,
  ]);
  assert.equal(frames[2].length, OWNER_LOCAL_FRAME_HEADER_BYTES + 7);

  const reader = new OwnerLocalFrameReader();
  const seen = frames.flatMap((frame) => reader.push(frame));
  assert.equal(seen.length, 1);
  assert.deepEqual([...seen[0]], [...message]);
});

test("the fragmentation boundary is exact at both sides of it", () => {
  const at = encodeMessageFrames(0, body(OWNER_LOCAL_MAX_FRAME_BODY_BYTES));
  assert.equal(at.length, 1, "a message of exactly one frame's body is whole");
  assert.equal(at[0].length, OWNER_LOCAL_FRAME_HEADER_BYTES + OWNER_LOCAL_MAX_FRAME_BODY_BYTES);

  const past = encodeMessageFrames(0, body(OWNER_LOCAL_MAX_FRAME_BODY_BYTES + 1));
  assert.equal(past.length, 2, "one byte past it is two frames");
  assert.deepEqual(past.map((frame) => frame[6]), [FRAME_SEGMENTS.first, FRAME_SEGMENTS.last]);
  assert.equal(past[1].length, OWNER_LOCAL_FRAME_HEADER_BYTES + 1, "the second frame carries the one byte");

  for (const frames of [at, past]) {
    const reader = new OwnerLocalFrameReader();
    const seen = frames.flatMap((frame) => reader.push(frame));
    assert.equal(seen.length, 1);
    assert.equal(seen[0].length, frames.reduce((total, frame) => total + frame.length - OWNER_LOCAL_FRAME_HEADER_BYTES, 0));
  }
});

test("a stream that ends between fragments is half a message, not an idle reader", () => {
  const frames = encodeMessageFrames(0, body(OWNER_LOCAL_MAX_FRAME_BODY_BYTES + 100));
  const reader = new OwnerLocalFrameReader();
  assert.deepEqual(reader.push(frames[0]), [], "the first fragment completes no message");
  assert.equal(reader.pending, 0, "no bytes are held: the frame was whole");
  assert.equal(reader.collecting, true, "but a message is half-arrived");
});

test("a chunk boundary is not a frame boundary and a frame boundary is not a message boundary", () => {
  const message = body(OWNER_LOCAL_MAX_FRAME_BODY_BYTES + 3, 0x11);
  const wire = Buffer.concat(encodeMessageFrames(7, message).map(Buffer.from));

  const reader = new OwnerLocalFrameReader();
  const collected = [];
  for (let offset = 0; offset < wire.length; offset += 997) {
    collected.push(...reader.push(Uint8Array.from(wire.subarray(offset, offset + 997))));
  }
  assert.equal(collected.length, 1, "one message, however the bytes were cut");
  assert.deepEqual([...collected[0]], [...message]);
  assert.equal(reader.pending, 0);
  assert.equal(reader.collecting, false);
});

test("a head is bounded before its body is waited for", () => {
  const oversized = new Uint8Array(OWNER_LOCAL_FRAME_HEADER_BYTES);
  oversized[0] = OWNER_LOCAL_FRAMING_VERSION;
  oversized[5] = FRAME_OPERATIONS.data;
  oversized[6] = FRAME_SEGMENTS.whole;
  new DataView(oversized.buffer).setUint32(7, OWNER_LOCAL_MAX_FRAME_BODY_BYTES + 1, false);
  expectCode(() => new OwnerLocalFrameReader().push(oversized), "FRAME_BODY_TOO_LARGE");
});

test("the two shapes a fragment cannot have", () => {
  expectCode(
    () => encodeCanonicalFrame({ channel: 0, operation: FRAME_OPERATIONS.open, segment: FRAME_SEGMENTS.first, body: body(1) }),
    "FRAGMENTED_CONTROL",
  );
  expectCode(
    () => encodeCanonicalFrame({ channel: 0, operation: FRAME_OPERATIONS.data, segment: FRAME_SEGMENTS.middle, body: body(0) }),
    "EMPTY_FRAGMENT",
  );
});

test("a segment out of sequence ends the reassembly rather than being skipped", () => {
  const reader = new OwnerLocalFrameReader();
  expectCode(
    () => reader.push(encodeCanonicalFrame({
      channel: 0, operation: FRAME_OPERATIONS.data, segment: FRAME_SEGMENTS.last, body: body(4),
    })),
    "UNEXPECTED_SEGMENT",
  );

  const second = new OwnerLocalFrameReader();
  second.push(encodeCanonicalFrame({
    channel: 0, operation: FRAME_OPERATIONS.data, segment: FRAME_SEGMENTS.first, body: body(4),
  }));
  assert.equal(second.collecting, true);
  expectCode(
    () => second.push(encodeCanonicalFrame({
      channel: 0, operation: FRAME_OPERATIONS.data, segment: FRAME_SEGMENTS.whole, body: body(4),
    })),
    "UNEXPECTED_SEGMENT",
  );
  assert.equal(second.collecting, false, "a refused sequence does not leave a half message behind");
});

test("unknown version, operation, and segment are each named separately", () => {
  const frame = encodeCanonicalFrame({
    channel: 0, operation: FRAME_OPERATIONS.data, segment: FRAME_SEGMENTS.whole, body: body(1),
  });
  const withByte = (index, value) => {
    const copy = Uint8Array.from(frame);
    copy[index] = value;
    return copy;
  };
  expectCode(() => new OwnerLocalFrameReader().push(withByte(0, 2)), "UNSUPPORTED_VERSION");
  expectCode(() => new OwnerLocalFrameReader().push(withByte(5, 9)), "OPERATION_UNKNOWN");
  expectCode(() => new OwnerLocalFrameReader().push(withByte(6, 9)), "SEGMENT_UNKNOWN");
});

test("the message bound is the profile's and a caller may lower it but not raise it", () => {
  assert.equal(OWNER_LOCAL_MAX_MESSAGE_BYTES, 16 * 1024 * 1024);
  expectCode(
    () => encodeMessageFrames(0, body(4), { maximumMessageBytes: OWNER_LOCAL_MAX_MESSAGE_BYTES + 1 }),
    "MAXIMUM_MESSAGE_BYTES_INVALID",
  );
  expectCode(() => encodeMessageFrames(0, body(64), { maximumMessageBytes: 32 }), "MESSAGE_TOO_LARGE");
  expectCode(() => new OwnerLocalFrameReader({ maximumMessageBytes: 0 }), "MAXIMUM_MESSAGE_BYTES_INVALID");
});

test("a control frame carries no message and is passed over rather than given a meaning", () => {
  const reader = new OwnerLocalFrameReader();
  const control = encodeCanonicalFrame({
    channel: 3, operation: FRAME_OPERATIONS.open, segment: FRAME_SEGMENTS.whole, body: body(0),
  });
  assert.deepEqual(reader.push(control), []);
  const data = encodeCanonicalFrame({
    channel: 3, operation: FRAME_OPERATIONS.data, segment: FRAME_SEGMENTS.whole, body: body(2),
  });
  assert.equal(reader.push(data).length, 1, "a control frame must not disturb the next message");
});
