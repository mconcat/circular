import type { CorrelationId } from "@circular/protocol";

/** The ceiling: correlations alive at once, not correlations ever issued. */
export declare const MAXIMUM_LIVE_CORRELATIONS: 4096;

/** A refusal or fault at the correlation pool. */
export declare class CorrelationPoolError extends Error {
  /** Names the exact refusal; `EXHAUSTED` is the published disposition of a full pool. */
  readonly code: string;
}

/** Bounds one session's live correlations. */
export interface CorrelationPoolOptions {
  /** Overrides the ceiling; the default is `MAXIMUM_LIVE_CORRELATIONS`. */
  readonly maximumLive?: number;
}

/** Maps one session's local exchange identities onto the wire's `u32` correlation slots. */
export declare class CorrelationPool {
  constructor(options?: CorrelationPoolOptions);
  /** How many slots are held right now. */
  readonly live: number;
  /** The ceiling this pool enforces. */
  readonly maximumLive: number;
  /** Takes a slot, or refuses with `EXHAUSTED` when every slot is held. */
  allocate(correlationId: CorrelationId | string): number;
  /** The slot held by one exchange, or `undefined`. */
  slotFor(correlationId: CorrelationId | string): number | undefined;
  /** The exchange holding one slot, or `undefined`. */
  correlationFor(slot: number): string | undefined;
  /** Returns a slot; refuses if it is not held. */
  release(slot: number): string;
  /** Returns the slot held by one exchange; refuses if it holds none. */
  releaseCorrelation(correlationId: CorrelationId | string): number;
  /** Returns every slot, as session teardown does. */
  releaseAll(): readonly number[];
}

/** How one verb relates to the correlation slot of its exchange. */
export type ExchangeRole = "opens" | "closes" | "continues" | "none";

/** The disposition of every declared verb. */
export declare const EXCHANGE_ROLE: Readonly<Record<string, ExchangeRole>>;

/** Whether an *arriving* envelope ends the exchange on its correlation. */
export declare function terminatesExchange(verb: string): boolean;

/** Whether sending this verb should take a slot. */
export declare function opensExchange(verb: string): boolean;
