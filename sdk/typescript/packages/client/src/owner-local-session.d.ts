/** A refusal in the session's exchange discipline.
 * ESTABLISHMENT_REJECTED preserves the daemon rejection object in inherited Error.cause. */
export declare class SessionError extends Error {
  /** Names the exact refusal. */
  readonly code: string;
}

/** The partition spellings a `Hello` names a minor for, in declaration order (`@circular/protocol/tables` `Partition`). */
export declare const PARTITION_SPELLINGS: readonly string[];

/** Visible finite default used when `maxInFlight` is omitted. */
export declare const DEFAULT_MAX_IN_FLIGHT: 256;

export declare function helloBody(options?: {
  readonly protocolVersion?: number;
  readonly minor?: number;
  readonly requestedRoles?: readonly unknown[];
}): unknown;

export interface Established {
  readonly features: unknown;
  readonly protocolVersion: bigint;
  readonly roles: unknown;
  readonly token: Uint8Array;
  readonly trust: bigint;
}

/** Reads an `Established` from a `HelloAck` body, refusing an incomplete one. */
export declare function readEstablished(body: unknown): Established;

/** An established session over a transport that moves whole envelopes. */
/** One held correlation slot: send on it, await the next envelope, release it. */
export interface OwnerLocalHeldStream {
  readonly releaseOn: string | null;
  send(partition: string, verb: string, payload: unknown): Promise<void>;
  /** Infinity waits for an envelope or session closure without a timer. */
  next(waitMs?: number): Promise<unknown>;
  /**
   * Hands the slot back. A paged query read short of `Complete` is closed with `QueryClose`
   * once its last request is answered; the slot stays taken until the close's terminal arrives.
   * A subscription the daemon still holds on the slot is ended with one `Unsubscribe`; the slot
   * stays taken until every request on it is answered. Returns at once.
   */
  release(): void;
}

export interface OwnerLocalSession {
  readonly established: Established;
  /** Envelopes that arrived on a slot nobody held. */
  readonly unmatched: readonly unknown[];
  /** How many exchanges are live. */
  readonly liveCorrelations: number;
  /** Sends one semantic declaration through the shared physical command codec. */
  declare(
    command: import("@circular/protocol").DeclarationCommand<import("@circular/protocol").EpochAddressDomain>,
  ): Promise<import("@circular/protocol").Result<unknown>>;
  /** Acquires all pages of one immutable compacted declaration snapshot. */
  authoringSnapshot(
    scope: import("@circular/protocol").ScopeId,
    pageLimit: number,
    /** Upper actor-local cut returned by an observation query; omitted for Live. */
    upto?: readonly unknown[],
  ): Promise<unknown>;
  /** Opens the credit-controlled accepted semantic epoch feed after a snapshot cursor. */
  authoringCommits(
    scope: import("@circular/protocol").ScopeId,
    after: import("@circular/protocol").StructureCursor,
  ): Promise<{
    readonly scope: import("@circular/protocol").ScopeId;
    readonly after: import("@circular/protocol").StructureCursor;
    grant(frames: bigint | number): Promise<void>;
    next(waitMs?: number): Promise<unknown>;
    unsubscribe(): Promise<void>;
    release(): void;
  }>;
  /** Sends any verb and returns the answering envelope. */
  /** Lifecycle accepts wire Values or SDK input: Pause {mode?: "Pause" | "ForcePause"};
   * Resume {expectedAuthoringRevision: Uint8Array} takes the snapshot At.revision digest (32 bytes).
   * The session lowers the mode and revision field before transport. */
  exchange(partition: "Query", verb: "Query", payload: {
    readonly name: string;
    readonly args: unknown;
    readonly since?: readonly unknown[];
    readonly upto?: readonly unknown[];
    readonly page?: { readonly limit: bigint; readonly cursor?: unknown };
  }): Promise<{
    readonly payload: readonly [1n, {
      readonly anchor: unknown;
      readonly items: readonly unknown[];
      readonly terminal: bigint | readonly [bigint, unknown];
      readonly cut?: readonly unknown[];
      readonly folded_from?: readonly unknown[];
    }] | readonly [2n, unknown];
  }>;
  exchange(partition: string, verb: string, payload: unknown): Promise<unknown>;
  /**
   * Holds one correlation slot for a stream of envelopes (queries with pages, subscriptions).
   * `releaseOn` names the verb whose arrival ends the hold; `release()` frees the slot.
   */
  hold(label: string, releaseOn?: string | null): OwnerLocalHeldStream;
  /**
   * The event-injection partition. `inject` sends one `Inject` to an export mount whose request
   * role is bound (never an actor or a port) and returns the `InjectAck` result as a value.
   */
  readonly interactions: Pick<import("./index.js").InteractionPartitions, "inject">;
  readonly replay: import("./replay.js").ReplayPartition;
  /** Says goodbye and closes. */
  goodbye(): Promise<void>;
  close(): Promise<void>;
}

/** Establishes a session over the byte wire: the establishment pair, then commands. */
export declare function establish(
  transport: unknown,
  options: {
    readonly resourceCeilings: unknown;
    readonly hello?: Parameters<typeof helloBody>[0];
    /** Optional upper bound on simultaneously live request correlations. */
    readonly maxInFlight?: number;
    readonly requestTimeoutMs?: number;
  },
): Promise<OwnerLocalSession>;
