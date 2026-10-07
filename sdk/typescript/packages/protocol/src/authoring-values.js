/** Shared values for hand-authored declaration commands. */

import { DEFAULT_EDGE_ATTRS } from "@circular/protocol/declaration";

export const address = (value) => Object.freeze({ arm: "absolute", value });

/** A scope identity is a segment sequence; the hierarchy lives in the segments, not in a string. */
export const scope = (...names) => address(names.map((name) => ({ name })));

export const FLAGS = Object.freeze({ bypass: false, mute: false, pause: false });

/** One authored edge capacity — the product default's, so graphs cannot disagree with it. */
export const EDGE_CAPACITY = DEFAULT_EDGE_ATTRS.policy.capacity;

/**
 * The edge attributes a declared edge carries.
 *
 * `Lossless` is a choice rather than a default: `Delivery` has no unspecified arm, `BestEffort`
 * would drop under load, and `Durable` would bind throughput to commits. The delay is the
 * irreducible `0/1`. `capacity` is authored because an absent capacity means the destination's
 * *declared* capacity, and a destination that declares none is an activation failure that must
 * not be covered by a default.
 */
export const EDGE_ATTRS = DEFAULT_EDGE_ATTRS;

export const PLACEHOLDER_ENVIRONMENT = Object.freeze({
  declarationSchema: Uint8Array.from([0x01]),
  specSet: Uint8Array.from([0x03]),
});
