/**
 * The canonical `Value` codec.
 *
 * Eight kinds: `Null`, `Bool`, `Int`, `Float`, `String`, `Bytes`, `Array`,
 * `Object`. The semantics follow general CBOR (RFC 8949); the format is this project's own.
 *
 * `Int` is `i64` and `Float` is binary64 held as bits rather than as a JavaScript number,
 * because `Float` equality is bitwise: `-0.0` stays in the value space, every NaN
 * encodes as the canonical quiet NaN, and any other NaN bit pattern is a decode rejection.
 * Object keys are ordered by their **original UTF-8 bytes**, which is not the
 * encoded-form order RFC 8949 section 4.2.1 uses.
 *
 * Every ceiling is the caller's: `maximumBytes`, `maximumContainerEntries`, and
 * `maximumStringBytes` have no fixed value, and only depth 64 is settled. Nothing here
 * supplies a default, so a caller cannot inherit a limit it did not choose.
 *
 * The byte layout is not adopted: no owner record has approved a codec. It is implemented here
 * here because a client needs a codec to run; test vectors are evidence *about* this
 * implementation rather than a second copy of it.
 */

const FAIL = Object.freeze({
  RESOURCE_CEILINGS_REQUIRED: "RESOURCE_CEILINGS_REQUIRED",
  RESOURCE_CEILING_INVALID: "RESOURCE_CEILING_INVALID",
});

/**
 * Tag assignment. Zero is reserved so a zero-filled buffer rejects rather than decoding as a
 * live kind — the same asymmetry the head tags have.
 *
 * Renumbered from the retired seven-kind profile rather than patched around its hole. Nothing
 * has been published, so there is no compatibility to preserve, and a contiguous run ordered
 * scalars-then-containers is easier to reason about than one carrying a retired number.
 */
export const VALUE_TAGS = Object.freeze({
  null: 1,
  bool: 2,
  int: 3,
  float: 4,
  string: 5,
  bytes: 6,
  array: 7,
  object: 8,
  uint: 9,
});

const TAG_NAMES = Object.freeze(
  Object.fromEntries(Object.entries(VALUE_TAGS).map(([name, tag]) => [tag, name])),
);

/** The single admitted NaN. Any other NaN bit pattern is rejected on decode. */
const CANONICAL_NAN_BITS = 0x7ff8_0000_0000_0000n;

const INT_MIN = -(2n ** 63n);
const INT_MAX = 2n ** 63n - 1n;
const UINT_MAX = 2n ** 64n - 1n;

/**
 * An unsigned 64-bit value.
 *
 * A wrapper rather than a bare `BigInt`, because a bare one already means `Int` and the two kinds
 * are distinct on the wire. Without it, decoding a `UInt` would hand back something that
 * re-encodes as an `Int` — a kind silently lost on a round trip, which is the failure the value
 * model was split into nine kinds to avoid.
 */
export class CircularUInt {
  constructor(value) {
    if (typeof value !== "bigint") throw new TypeError("a UInt holds a BigInt");
    if (value < 0n || value > UINT_MAX) {
      throw new RangeError(`${value} does not fit the unsigned 64-bit integer kind`);
    }
    this.value = value;
    Object.freeze(this);
  }
}

/** Constructs the unsigned kind. `uint(5n)` and `5n` are different values. */
export function uint(value) {
  return new CircularUInt(typeof value === "bigint" ? value : BigInt(value));
}
const MAXIMUM_U32 = 0xffff_ffff;

const textEncoder = new TextEncoder();
const strictTextDecoder = new TextDecoder("utf-8", { fatal: true });

export class ValueBetaError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new ValueBetaError(code, message);
}

/**
 * Every entry point requires all four ceilings from its caller. The production values are not
 * fixed, so this codec has no defaults to fall back on and refuses rather than inventing one.
 */
export function validateResourceCeilings(resourceCeilings) {
  if (resourceCeilings === null || typeof resourceCeilings !== "object" || Array.isArray(resourceCeilings)) {
    fail(FAIL.RESOURCE_CEILINGS_REQUIRED, "all four resource ceilings are required");
  }
  const required = ["maximumBytes", "maximumDepth", "maximumContainerEntries", "maximumStringBytes"];
  for (const name of required) {
    if (!Object.prototype.hasOwnProperty.call(resourceCeilings, name)) {
      fail(FAIL.RESOURCE_CEILINGS_REQUIRED, `missing resource ceiling ${name}`);
    }
    const value = resourceCeilings[name];
    if (!Number.isSafeInteger(value) || value < 1) {
      fail(FAIL.RESOURCE_CEILING_INVALID, `${name} must be a positive safe integer`);
    }
  }
  for (const name of ["maximumBytes", "maximumContainerEntries", "maximumStringBytes"]) {
    if (resourceCeilings[name] > MAXIMUM_U32) {
      fail(FAIL.RESOURCE_CEILING_INVALID, `${name} exceeds the u32 length space`);
    }
  }
  return Object.freeze({
    maximumBytes: resourceCeilings.maximumBytes,
    maximumDepth: resourceCeilings.maximumDepth,
    maximumContainerEntries: resourceCeilings.maximumContainerEntries,
    maximumStringBytes: resourceCeilings.maximumStringBytes,
  });
}

function assertDepth(depth, ceilings) {
  if (depth > ceilings.maximumDepth) fail("DEPTH_EXCEEDED", `depth ${depth} exceeds the ceiling`);
}

function assertEntries(count, ceilings) {
  if (count > ceilings.maximumContainerEntries) {
    fail("CONTAINER_ENTRIES_EXCEEDED", `container of ${count} exceeds the ceiling`);
  }
}

function assertStringBytes(byteLength, ceilings, label) {
  if (byteLength > ceilings.maximumStringBytes) {
    fail("STRING_BYTES_EXCEEDED", `${label} of ${byteLength} bytes exceeds the ceiling`);
  }
}

function assertTotalBytes(byteLength, ceilings) {
  if (byteLength > ceilings.maximumBytes) {
    fail("VALUE_BYTES_EXCEEDED", `encoded value of ${byteLength} bytes exceeds the ceiling`);
  }
}

/** Ascending by the original UTF-8 bytes, so length never precedes content. */
function compareUtf8(left, right) {
  const length = Math.min(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    if (left[index] !== right[index]) return left[index] < right[index] ? -1 : 1;
  }
  if (left.length === right.length) return 0;
  return left.length < right.length ? -1 : 1;
}

function strictUtf8Bytes(value, label) {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) {
        fail("STRING_UNPAIRED_SURROGATE", `${label} contains an unpaired surrogate`);
      }
      index += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      fail("STRING_UNPAIRED_SURROGATE", `${label} contains an unpaired surrogate`);
    }
  }
  return textEncoder.encode(value);
}

class Writer {
  constructor() {
    this.chunks = [];
    this.byteLength = 0;
  }

  push(bytes) {
    this.chunks.push(bytes);
    this.byteLength += bytes.length;
  }

  u8(value) {
    this.push(Uint8Array.of(value));
  }

  u32(value) {
    if (!Number.isSafeInteger(value) || value < 0 || value > MAXIMUM_U32) {
      fail("LENGTH_OUT_OF_RANGE", `length ${value} does not fit the u32 length space`);
    }
    const bytes = new Uint8Array(4);
    new DataView(bytes.buffer).setUint32(0, value, false);
    this.push(bytes);
  }

  i64(value) {
    const bytes = new Uint8Array(8);
    new DataView(bytes.buffer).setBigInt64(0, value, false);
    this.push(bytes);
  }

  u64(value) {
    const bytes = new Uint8Array(8);
    new DataView(bytes.buffer).setBigUint64(0, value, false);
    this.push(bytes);
  }

  f64Bits(bits) {
    const bytes = new Uint8Array(8);
    new DataView(bytes.buffer).setBigUint64(0, bits, false);
    this.push(bytes);
  }

  finish() {
    const out = new Uint8Array(this.byteLength);
    let offset = 0;
    for (const chunk of this.chunks) {
      out.set(chunk, offset);
      offset += chunk.length;
    }
    return out;
  }
}

function floatBits(value) {
  const view = new DataView(new ArrayBuffer(8));
  view.setFloat64(0, value, false);
  const bits = view.getBigUint64(0, false);
  return Number.isNaN(value) ? CANONICAL_NAN_BITS : bits;
}

function writeOneValue(writer, value, ceilings, depth) {
  assertDepth(depth, ceilings);

  if (value === null) {
    writer.u8(VALUE_TAGS.null);
    return null;
  }
  if (value === undefined) {
    fail("UNDEFINED_NOT_A_VALUE", "Undefined is not a value kind in profile beta");
  }
  if (typeof value === "boolean") {
    writer.u8(VALUE_TAGS.bool);
    writer.u8(value ? 1 : 0);
    return null;
  }
  if (value instanceof CircularUInt) {
    writer.u8(VALUE_TAGS.uint);
    writer.u64(value.value);
    return null;
  }
  if (typeof value === "bigint") {
    if (value < INT_MIN || value > INT_MAX) {
      fail("INT_OUT_OF_RANGE", `${value} does not fit the signed 64-bit integer kind`);
    }
    writer.u8(VALUE_TAGS.int);
    writer.i64(value);
    return null;
  }
  if (typeof value === "number") {
    writer.u8(VALUE_TAGS.float);
    writer.f64Bits(floatBits(value));
    return null;
  }
  if (typeof value === "string") {
    const bytes = strictUtf8Bytes(value, "String");
    assertStringBytes(bytes.length, ceilings, "String");
    writer.u8(VALUE_TAGS.string);
    writer.u32(bytes.length);
    writer.push(bytes);
    return null;
  }
  if (value instanceof Uint8Array) {
    assertStringBytes(value.length, ceilings, "Bytes");
    writer.u8(VALUE_TAGS.bytes);
    writer.u32(value.length);
    writer.push(value);
    return null;
  }
  if (Array.isArray(value)) {
    assertEntries(value.length, ceilings);
    writer.u8(VALUE_TAGS.array);
    writer.u32(value.length);
    return { array: value, entries: null, index: 0, length: value.length, depth };
  }
  if (typeof value === "object") {
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== null && prototype !== Object.prototype) {
      fail("OBJECT_FOREIGN_PROTOTYPE", "only plain or null-prototype objects encode as Object");
    }
    const entries = [];
    for (const key of Object.keys(value)) {
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      if (descriptor.get !== undefined || descriptor.set !== undefined) {
        fail("OBJECT_ACCESSOR", `key ${key} is an accessor`);
      }
      if (descriptor.value === undefined) {
        fail("OBJECT_UNDEFINED_VALUE", `key ${key} is present with no value; omit the key instead`);
      }
      entries.push([strictUtf8Bytes(key, "Object key"), descriptor.value]);
    }
    if (Object.getOwnPropertySymbols(value).length > 0) {
      fail("OBJECT_SYMBOL_KEY", "symbol keys have no Value image");
    }
    assertEntries(entries.length, ceilings);
    entries.sort((left, right) => compareUtf8(left[0], right[0]));
    for (let index = 1; index < entries.length; index += 1) {
      if (compareUtf8(entries[index - 1][0], entries[index][0]) === 0) {
        fail("OBJECT_DUPLICATE_KEY", "duplicate object key");
      }
    }
    writer.u8(VALUE_TAGS.object);
    writer.u32(entries.length);
    return { array: null, entries, index: 0, length: entries.length, depth };
  }
  fail("UNSUPPORTED_CARRIER", `${typeof value} has no Value image`);
}

/**
 * Encodes one value.
 *
 * The traversal keeps its own stack instead of recursing. How deep a value may nest is the
 * caller's `maximumDepth`; a recursive encoder answers that question with the JavaScript call
 * stack as well, so the same value encodes or throws `RangeError` depending on how many frames
 * the caller already holds. One input has one answer, and only `maximumDepth` decides how deep
 * is too deep.
 */
function writeValue(writer, rootValue, ceilings, rootDepth) {
  const stack = [];
  let pending = { value: rootValue, depth: rootDepth };

  for (;;) {
    if (pending !== null) {
      const { value, depth } = pending;
      pending = null;
      const frame = writeOneValue(writer, value, ceilings, depth);
      if (frame !== null) stack.push(frame);
    }

    for (;;) {
      if (stack.length === 0) return;
      const frame = stack[stack.length - 1];
      if (frame.index >= frame.length) {
        stack.pop();
        continue;
      }
      const index = frame.index;
      frame.index += 1;
      if (frame.entries === null) {
        if (!Object.prototype.hasOwnProperty.call(frame.array, index)) {
          fail("ARRAY_HOLE", `array index ${index} is a hole`);
        }
        pending = { value: frame.array[index], depth: frame.depth + 1 };
      } else {
        const [keyBytes, entryValue] = frame.entries[index];
        assertStringBytes(keyBytes.length, ceilings, "Object key");
        writer.u32(keyBytes.length);
        writer.push(keyBytes);
        pending = { value: entryValue, depth: frame.depth + 1 };
      }
      break;
    }
  }
}

export function encodeValueBeta(value, resourceCeilings) {
  const ceilings = validateResourceCeilings(resourceCeilings);
  const writer = new Writer();
  writeValue(writer, value, ceilings, 1);
  const bytes = writer.finish();
  assertTotalBytes(bytes.length, ceilings);
  return bytes;
}

class Reader {
  constructor(bytes) {
    this.bytes = bytes;
    this.offset = 0;
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  remaining() {
    return this.bytes.length - this.offset;
  }

  take(count, label) {
    if (count > this.remaining()) fail("TRUNCATED", `${label} is truncated`);
    const slice = this.bytes.subarray(this.offset, this.offset + count);
    this.offset += count;
    return slice;
  }

  u8(label) {
    if (this.remaining() < 1) fail("TRUNCATED", `${label} is truncated`);
    return this.bytes[this.offset++];
  }

  u32(label) {
    if (this.remaining() < 4) fail("TRUNCATED", `${label} is truncated`);
    const value = this.view.getUint32(this.offset, false);
    this.offset += 4;
    return value;
  }

  i64(label) {
    if (this.remaining() < 8) fail("TRUNCATED", `${label} is truncated`);
    const value = this.view.getBigInt64(this.offset, false);
    this.offset += 8;
    return value;
  }

  u64(label) {
    if (this.remaining() < 8) fail("TRUNCATED", `${label} is truncated`);
    const value = this.view.getBigUint64(this.offset, false);
    this.offset += 8;
    return value;
  }
}

/**
 * Reads one object key and checks it against the key before it.
 *
 * Kept beside the frame it fills because a key is read one step ahead of its value: the frame
 * holds the key it is waiting to assign, and the ordering rule needs the previous key's
 * original bytes rather than its decoded form.
 */
function readObjectKey(reader, ceilings, frame) {
  const keyLength = reader.u32("Object key length");
  assertStringBytes(keyLength, ceilings, "Object key");
  const keyBytes = reader.take(keyLength, "Object key body");
  let key;
  try {
    key = strictTextDecoder.decode(keyBytes);
  } catch {
    return fail("OBJECT_KEY_INVALID_UTF8", "Object key is not strict UTF-8");
  }
  if (frame.previousKey !== null) {
    const order = compareUtf8(frame.previousKey, keyBytes);
    if (order === 0) fail("OBJECT_DUPLICATE_KEY", "duplicate object key");
    if (order > 0) fail("OBJECT_KEY_ORDER", "object keys must ascend by original UTF-8 bytes");
  }
  frame.previousKey = keyBytes.slice();
  frame.key = key;
}

/**
 * Reads one value head.
 *
 * Returns the finished value for a scalar or an empty container, and a frame for a container
 * with entries still to read. It never reads a nested value itself — that is `readValue`'s
 * stack, not this function's.
 */
function readOneValue(reader, ceilings, depth) {
  assertDepth(depth, ceilings);
  const tag = reader.u8("value tag");
  if (tag === 0) fail("TAG_RESERVED", "tag 0 is reserved");
  const name = TAG_NAMES[tag];
  if (name === undefined) fail("TAG_UNASSIGNED", `tag ${tag} is unassigned`);

  switch (name) {
    case "null":
      return { value: null, frame: null };
    case "bool": {
      const body = reader.u8("Bool body");
      if (body !== 0 && body !== 1) fail("BOOL_BODY_INVALID", `Bool body ${body} is not 0 or 1`);
      return { value: body === 1, frame: null };
    }
    case "int":
      return { value: reader.i64("Int body"), frame: null };
    case "uint":
      return { value: new CircularUInt(reader.u64("UInt body")), frame: null };
    case "float": {
      const bits = reader.u64("Float body");
      const isNaNBits = (bits & 0x7ff0_0000_0000_0000n) === 0x7ff0_0000_0000_0000n
        && (bits & 0x000f_ffff_ffff_ffffn) !== 0n;
      if (isNaNBits && bits !== CANONICAL_NAN_BITS) {
        fail("FLOAT_NONCANONICAL_NAN", "only the canonical quiet NaN is admitted");
      }
      const view = new DataView(new ArrayBuffer(8));
      view.setBigUint64(0, bits, false);
      return { value: view.getFloat64(0, false), frame: null };
    }
    case "string": {
      const length = reader.u32("String length");
      assertStringBytes(length, ceilings, "String");
      const bytes = reader.take(length, "String body");
      try {
        return { value: strictTextDecoder.decode(bytes), frame: null };
      } catch {
        return fail("STRING_INVALID_UTF8", "String body is not strict UTF-8");
      }
    }
    case "bytes": {
      const length = reader.u32("Bytes length");
      assertStringBytes(length, ceilings, "Bytes");
      return { value: reader.take(length, "Bytes body").slice(), frame: null };
    }
    case "array": {
      const count = reader.u32("Array count");
      assertEntries(count, ceilings);
      if (count === 0) return { value: [], frame: null };
      return { value: undefined, frame: { out: [], keyed: false, remaining: count, depth } };
    }
    case "object": {
      const count = reader.u32("Object pair count");
      assertEntries(count, ceilings);
      if (count === 0) return { value: Object.create(null), frame: null };
      const frame = {
        out: Object.create(null),
        keyed: true,
        remaining: count,
        depth,
        previousKey: null,
        key: null,
      };
      readObjectKey(reader, ceilings, frame);
      return { value: undefined, frame };
    }
    default:
      return fail("TAG_UNASSIGNED", `tag ${tag} is unassigned`);
  }
}

/**
 * Decodes one value.
 *
 * The nesting is held in a stack this function owns rather than in the JavaScript call stack —
 * see `writeValue` for why. A decoder is the side that matters most: the bytes arrive from
 * somewhere else, so a recursive reader would let the sender's nesting, not the caller's
 * `maximumDepth`, decide whether the process survives.
 */
function readValue(reader, ceilings, rootDepth) {
  const stack = [];
  let depth = rootDepth;

  for (;;) {
    const read = readOneValue(reader, ceilings, depth);
    if (read.frame !== null) {
      stack.push(read.frame);
      depth = read.frame.depth + 1;
      continue;
    }

    let result = read.value;
    for (;;) {
      if (stack.length === 0) return result;
      const frame = stack[stack.length - 1];
      if (frame.keyed) frame.out[frame.key] = result;
      else frame.out.push(result);
      frame.remaining -= 1;
      if (frame.remaining === 0) {
        result = frame.out;
        stack.pop();
        continue;
      }
      if (frame.keyed) readObjectKey(reader, ceilings, frame);
      depth = frame.depth + 1;
      break;
    }
  }
}

export function decodeValueBeta(input, resourceCeilings) {
  const ceilings = validateResourceCeilings(resourceCeilings);
  if (!(input instanceof Uint8Array)) fail("INPUT_NOT_BYTES", "input must be a Uint8Array");
  assertTotalBytes(input.length, ceilings);
  const reader = new Reader(input);
  const value = readValue(reader, ceilings, 1);
  if (reader.remaining() !== 0) fail("TRAILING_BYTES", "input carries bytes past one root value");
  return value;
}
