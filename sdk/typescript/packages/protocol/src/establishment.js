
import { encodeValueBeta } from "./value.js";

/** The transport trust grades. All three are argument-free, so `trust` is one integer. */
export const TRANSPORT_TRUST_TAGS = Object.freeze({
  LocalOwner: 1,
  LocalUser: 2,
  Remote: 3,
});

/**
 * The session roles and the arguments each arm carries.
 *
 * `null` marks an argument-free arm, whose value is the tag itself. The others publish their
 * argument order here because the table is what carries the identity — position is the table's
 * projection, not the identity itself.
 */
export const SESSION_ROLE_ARMS = Object.freeze({
  Reader: Object.freeze({ tag: 1, argument: null }),
  Writer: Object.freeze({ tag: 2, argument: "scope" }),
  Operator: Object.freeze({ tag: 4, argument: null }),
});

/** Scope segment arms. */
export const SCOPE_SEGMENT_ARMS = Object.freeze({ named: 1, instance: 2 });

function compareBytes(left, right) {
  const shared = Math.min(left.length, right.length);
  for (let index = 0; index < shared; index += 1) {
    if (left[index] !== right[index]) return left[index] < right[index] ? -1 : 1;
  }
  return left.length === right.length ? 0 : (left.length < right.length ? -1 : 1);
}

export class EstablishmentValueError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new EstablishmentValueError(code, message);
}

export function canonicalValueSequence(values, resourceCeilings) {
  if (!Array.isArray(values)) fail("SET_NOT_ARRAY", "a set is supplied as an array of values");
  const encoded = values.map((value) => ({ value, bytes: encodeValueBeta(value, resourceCeilings) }));
  encoded.sort((left, right) => compareBytes(left.bytes, right.bytes));
  for (let index = 1; index < encoded.length; index += 1) {
    if (compareBytes(encoded[index - 1].bytes, encoded[index].bytes) === 0) {
      fail("SET_DUPLICATE", "a set carries the same element twice");
    }
  }
  return encoded.map((entry) => entry.value);
}

/** The value of one session role: the tag alone, or a sequence whose head is the tag. */
export function sessionRoleValue(role, argument) {
  const arm = SESSION_ROLE_ARMS[role];
  if (arm === undefined) fail("ROLE_UNKNOWN", `${role} is not a declared session role`);
  if (arm.argument === null) {
    if (argument !== undefined) fail("ROLE_TAKES_NO_ARGUMENT", `${role} carries no argument`);
    return BigInt(arm.tag);
  }
  if (argument === undefined) fail("ROLE_ARGUMENT_MISSING", `${role} carries a ${arm.argument}`);
  return [BigInt(arm.tag), argument];
}

/** The value of one transport trust grade. All are argument-free. */
export function transportTrustValue(grade) {
  const tag = TRANSPORT_TRUST_TAGS[grade];
  if (tag === undefined) fail("TRUST_UNKNOWN", `${grade} is not a declared transport trust grade`);
  return BigInt(tag);
}

/**
 * A scope identity: an ordered sequence of segments.
 *
 * **Not sorted.** `ScopeId` is a sequence rather than a set and its order is the hierarchy
 * itself, so sorting would erase it. The set rule above applies where there is no order to
 * lose, and this is not such a place.
 */
export function scopeIdentityValue(segments) {
  if (!Array.isArray(segments)) fail("SCOPE_NOT_ARRAY", "a scope identity is a sequence of segments");
  return segments.map((segment) => scopeSegmentValue(segment));
}

/**
 * Reads a scope identity back into segments.
 *
 * The inverse of `scopeIdentityValue`, so a decoded address can be handed back in the shape it
 * was built from. Without it the codec would encode `{name}` objects and decode `[tag, name]`
 * sequences, and a "round trip" that returns a different shape is not one.
 */
export function scopeIdentityFromValue(value) {
  if (!Array.isArray(value)) fail("SCOPE_NOT_ARRAY", "a scope identity is a sequence of segments");
  return value.map((segment) => {
    if (!Array.isArray(segment) || segment.length < 2) {
      fail("SEGMENT_SHAPE", "a segment is a sequence headed by its arm tag");
    }
    const [tag, ...args] = segment;
    if (tag === BigInt(SCOPE_SEGMENT_ARMS.named)) {
      if (typeof args[0] !== "string") fail("SEGMENT_NAME_NOT_TEXT", "a segment name is text");
      return { name: args[0] };
    }
    if (tag === BigInt(SCOPE_SEGMENT_ARMS.instance)) {
      if (typeof args[0] !== "string") fail("SEGMENT_SHAPE", "an instance segment names what it is of");
      return { of: args[0], key: args[1] };
    }
    fail("SEGMENT_ARM_UNKNOWN", `segment arm ${tag} is unassigned`);
    return undefined;
  });
}

export function actorIdentityValue(key) {
  if (key === null || typeof key !== "object" || Array.isArray(key) || key instanceof Uint8Array) {
    fail("ACTOR_KEY_SHAPE", "a plan actor key is an object of a scope and a local name");
  }
  if (typeof key.local !== "string") fail("ACTOR_LOCAL_NOT_TEXT", "a local actor name is text");
  for (const name of Object.keys(key)) {
    if (name !== "local" && name !== "scope") {
      fail("ACTOR_KEY_UNEXPECTED", `a plan actor key carries no \`${name}\``);
    }
  }
  return { local: key.local, scope: scopeIdentityValue(key.scope) };
}

/** Reads a plan actor key back into the shape it was built from. */
export function actorIdentityFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) {
    fail("ACTOR_KEY_SHAPE", "a plan actor key is an object of a scope and a local name");
  }
  if (typeof value.local !== "string") fail("ACTOR_LOCAL_NOT_TEXT", "a local actor name is text");
  for (const name of Object.keys(value)) {
    if (name !== "local" && name !== "scope") {
      fail("ACTOR_KEY_UNEXPECTED", `a plan actor key carries no \`${name}\``);
    }
  }
  return { local: value.local, scope: scopeIdentityFromValue(value.scope) };
}

export function scopeSegmentValue(segment) {
  if (segment === null || typeof segment !== "object") fail("SEGMENT_SHAPE", "a segment is an object");
  if ("name" in segment) {
    if (typeof segment.name !== "string") {
      fail("SEGMENT_NAME_NOT_TEXT", "a segment name is text: character grammar and NFC normalization define the kind, and carrying it as bytes puts both outside the contract");
    }
    return [BigInt(SCOPE_SEGMENT_ARMS.named), segment.name];
  }
  if (typeof segment.of === "string") {
    const { key } = segment;
    const scalar = typeof key;
    if (scalar !== "string" && scalar !== "bigint" && scalar !== "boolean") {
      fail(
        "SEGMENT_KEY_KIND",
        "an instance key is a String, an Int, or a Bool; identity excludes Float because NaN and signed zero have more than one normal form",
      );
    }
    return [BigInt(SCOPE_SEGMENT_ARMS.instance), segment.of, key];
  }
  fail("SEGMENT_ARM_UNKNOWN", "a segment is either named or an instance");
  return undefined;
}
