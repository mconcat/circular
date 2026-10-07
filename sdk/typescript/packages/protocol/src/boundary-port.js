/**
 * Canonical physical identities for child pipeline boundary ports.
 *
 * This is the authoring-side mirror of
 * `crates/protocol/src/boundary_port.rs::BoundaryPortId::derive`. The daemon still validates the
 * complete child interface and remains the final authority on collisions and stale generations;
 * this module only gives an author the exact spelling that validation expects.
 */

import { actorIdentityValue } from "./establishment.js";
import { sha256 } from "./internal/sha256.js";
import { encodeValueBeta } from "./value.js";
import { resourceCeilings } from "./internal/closed-tables.js";

/** Copied from Rust's `BOUNDARY_PORT_ID_VERSION`; changing either side breaks the shared vectors. */
export const BOUNDARY_PORT_ID_VERSION = 1;
/** Copied from Rust's `SYNTH_BOUNDARY_LOCAL_VERSION`; shared vectors bind both implementations. */
export const SYNTH_BOUNDARY_LOCAL_VERSION = 1;

/**
 * Tags and reserved prefixes copied from Rust's `BoundaryPortDirection::{tag,prefix}`.
 * The tag enters the digest; the prefix carries the direction and codec version visibly.
 */
const DIRECTIONS = Object.freeze({
  inlet: Object.freeze({ tag: 1n, prefix: "_bi1_", synthLocalPrefix: "_bni1_" }),
  outlet: Object.freeze({ tag: 2n, prefix: "_bo1_", synthLocalPrefix: "_bno1_" }),
});

/** The ceilings Rust derive passes to `encode`: `Ceilings::for_boundary(Boundary::Identity)`. */
const PROVISIONAL_CEILINGS = resourceCeilings("Identity");

const U64_MAX = (1n << 64n) - 1n;

function directionDomain(direction) {
  const domain = DIRECTIONS[direction];
  if (domain === undefined) {
    throw new TypeError(`boundary direction must be \"inlet\" or \"outlet\", got ${String(direction)}`);
  }
  return domain;
}

function digestHex26(identity) {
  return Array.from(sha256(encodeValueBeta(identity, PROVISIONAL_CEILINGS)), (byte) => byte.toString(16).padStart(2, "0"))
    .join("")
    .slice(0, 26);
}

function generationBytes(generation) {
  const exact = typeof generation === "bigint"
    ? generation
    : Number.isSafeInteger(generation)
      ? BigInt(generation)
      : null;
  if (exact === null || exact < 0n || exact > U64_MAX) {
    throw new RangeError("boundary generation must be an unsigned 64-bit integer");
  }
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, exact, false);
  return bytes;
}

/**
 * Derives the daemon's reserved 31-byte port spelling from one exact authored actor identity.
 *
 * The canonical Value is `{ direction: Int, generation: Bytes(u64 BE), actor: { local, scope },
 * version: Int(1) }`. Its SHA-256 digest contributes the first thirteen bytes as lowercase hex.
 */
export function deriveBoundaryPortId(direction, actorKey, generation) {
  const domain = directionDomain(direction);
  if (actorKey === null || typeof actorKey !== "object" || actorKey.local === "") {
    throw new TypeError("boundary actor identity must have a non-empty local name");
  }
  const identity = {
    direction: domain.tag,
    generation: generationBytes(generation),
    actor: actorIdentityValue(actorKey),
    version: BigInt(BOUNDARY_PORT_ID_VERSION),
  };
  const digest = digestHex26(identity);
  return `${domain.prefix}${digest}`;
}

/**
 * Derives the reserved 32-byte local for a synthesized boundary actor.
 *
 * This mirrors `SynthBoundaryLocal::derive`: the canonical Value is
 * `{ direction, inner: { local, port }, version }`. Target scope, move membership/order/time,
 * outer endpoint, type, and arity are deliberately outside the identity.
 */
export function deriveSynthBoundaryLocal(direction, innerLocal, innerPort) {
  const domain = directionDomain(direction);
  if (typeof innerLocal !== "string" || typeof innerPort !== "string") {
    throw new TypeError("synthesized boundary identity requires string local and port spellings");
  }
  const identity = {
    direction: domain.tag,
    inner: { local: innerLocal, port: innerPort },
    version: BigInt(SYNTH_BOUNDARY_LOCAL_VERSION),
  };
  return `${domain.synthLocalPrefix}${digestHex26(identity)}`;
}
