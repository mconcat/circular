import type { CircularUInt } from "./value.js";
/**
 * `Subscribe { target, args, lens? }`. `lens` is the correlation key of the replay lens the
 * subscription reads through; absent, the subscription is live.
 */
export declare function subscribeValue(request: {
  readonly target: string;
  readonly args: unknown;
  readonly lens?: bigint | number;
}): unknown;
export declare function creditValue(request: { readonly frames: bigint | number }): unknown;
export declare const UNSUBSCRIBE_BODY: null;
export declare function subscriptionFrameFromValue(value: unknown): {
  readonly arm: "Lossless" | "Conflated";
  readonly origin: "Retained" | "Live";
  readonly payload: unknown;
  readonly folded?: bigint;
  readonly slot?: unknown;
} | {
  readonly arm: "Credit";
  readonly origin: "Retained" | "Live";
  readonly payload: unknown;
  /** Items not yet published after this frame in the same world snapshot; not credit balance. */
  readonly pending_after: CircularUInt;
} | {
  readonly arm: "RetentionComplete";
  readonly anchor: unknown;
  readonly delivered: CircularUInt;
};
export declare function subscriptionEndedFromValue(value: unknown): unknown;
