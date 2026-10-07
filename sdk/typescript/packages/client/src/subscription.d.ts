import type { CircularUInt } from "@circular/protocol/actor-query";
/** A refusal while opening, reading, crediting, or closing a subscription. */
export declare class SubscriptionError extends Error {
  constructor(code: string, message: string);
  readonly code: string;
}

/** Reads a wire frame; an absent slot is null, while a zero fold count stays zero. */
export declare function readFrame(body: unknown): Readonly<{
  arm: "Lossless";
  origin: "Retained" | "Live";
  payload: unknown;
  folded: null;
  slot: null;
} | {
  arm: "Credit";
  origin: "Retained" | "Live";
  payload: unknown;
  folded: null;
  slot: null;
  readonly pending_after: CircularUInt;
} | {
  arm: "Conflated";
  origin: "Retained" | "Live";
  payload: unknown;
  folded: bigint;
  slot: unknown;
} | {
  arm: "RetentionComplete";
  anchor: unknown;
  delivered: CircularUInt;
}>;

/** Reads a terminal frame, preserving its reason argument and anchor. */
export declare function readEnding(body: unknown): {
  readonly reason: "ByClient" | "ConsumerBehind" | "TargetGone" | "Withdrawn"
    | "SessionClosed" | "ResetRequired" | "ScopeGone" | "IncompatibleClient" | "Complete";
  readonly argument: unknown;
  readonly code: bigint;
  readonly anchor: unknown;
};

/** Opens one held correlation. Initial credit is required; no speed is selected implicitly. */
export declare function openSubscription(
  session: {
    hold(label: string): {
      readonly slot: number;
      send(partition: string, verb: string, payload: unknown): Promise<unknown>;
      next(waitMs?: number): Promise<{
        readonly kind: { readonly verb: string };
        readonly payload: unknown;
      } | null>;
      release(): void;
    };
  },
  options: {
    readonly target: string;
    readonly args: unknown;
    readonly initialCredit: bigint | number;
    /**
     * The replay lens this subscription reads through — the handle `session.replay.start`
     * returned. Absent, the subscription is live and no lens on the connection touches it.
     */
    readonly lens?: { readonly correlation: number };
    readonly openTimeoutMs?: number;
    readonly creditTimeoutMs?: number;
    readonly closeTimeoutMs?: number;
  },
): Promise<{
  readonly ack: unknown;
  readonly correlation: number;
  readonly balance: bigint;
  readonly ended: ReturnType<typeof readEnding> | null;
  credit(frames: bigint | number): Promise<unknown>;
  /** Infinity waits for a frame, subscription ending, or session closure without a timer. */
  receive(waitMs?: number): Promise<ReturnType<typeof readFrame> | null>;
  end(body: unknown): ReturnType<typeof readEnding>;
  /**
   * Resolves on the reply; the terminal frame arrives separately. `null` when the stream's ending
   * already reached this side: no `Unsubscribe` is sent to an ended stream.
   */
  close(): Promise<unknown>;
  /** Sends the one `Unsubscribe` this stream still needs and returns at once. */
  release(): void;
}>;
