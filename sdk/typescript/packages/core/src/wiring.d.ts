import type { CircularValue, EdgeId, DeclaredDelay, WirePolicy } from "@circular/protocol";
import type { DownstreamActorMethods } from "./catalog.js";

declare const pendingEndpointBrand: unique symbol;
declare const sourceEndpointBrand: unique symbol;
declare const targetEndpointBrand: unique symbol;
declare const writableBoundaryBrand: unique symbol;
declare const newEdgeBrand: unique symbol;
declare const currentEdgeBrand: unique symbol;

export type { DeclaredDelay, DeliveryPolicy, ShedPolicy, WirePolicy } from "@circular/protocol";

/** Describes one authored edge's identity and delivery attributes. */
export interface EdgeOptions {
  /**
   * Selects the parallel-edge identity component.
   * Omission means ordinal 0 of this (source, target) pair, with or without `circular:current`:
   * an existing ordinal-0 edge is updated in place and the same line deployed again leaves one edge.
   * A parallel wire takes an explicit ordinal. Two lines of one program may not declare the same
   * (source, target, ordinal); that is refused with `CIRCULAR_EDGE_ORDINAL_CONFLICT`.
   */
  readonly ordinal?: number;
  /** Sets the physical delay in rational seconds. */
  readonly delay?: DeclaredDelay;
  /** Sets the complete delivery policy. */
  readonly policy?: WirePolicy;
}

/** Marks an endpoint belonging to an actor created in the current mutation epoch. */
export interface NewHandleMode {
  /** Discriminates an epoch-local new handle. */
  readonly kind: "new";
}

/** Marks an endpoint resolved from one anchored current-state snapshot. */
export interface CurrentHandleMode {
  /** Discriminates an anchored current-state handle. */
  readonly kind: "current";
}

/** Distinguishes new and anchored-current authored handles. */
export type HandleMode = NewHandleMode | CurrentHandleMode;

/** Port value parameters are structural hints, including bigint and Uint8Array.
 * They do not constrain or change the protocol CircularValue carrier. */
/** Represents a graph source endpoint and its upstream-first construction operations. */
export interface SourceEndpoint<
  Value = CircularValue,
  Mode extends HandleMode = HandleMode,
> extends DownstreamActorMethods<Value> {
  /** Prevents arbitrary host values from being used as graph sources. */
  readonly [sourceEndpointBrand]: { readonly value: Value; readonly mode: Mode };
  /** Binds a roles-only mount; writable boundary handles default to request, sources to result.
   * Requires the installed authoring host; otherwise throws CIRCULAR_EXPORT_MOUNT_HOST_MISSING.
   */
  mount<Receiver extends SourceEndpoint<Value, Mode>>(this: Receiver, name: string,
    role?: Receiver extends { readonly [writableBoundaryBrand]: true }
      ? "request" : "result" | "progress" | "error"): Receiver;
  into<TargetValue, TargetMode extends HandleMode>(
    target: TargetEndpoint<TargetValue, TargetMode>,
    options?: EdgeOptions,
  ): this;
}

/** A local value: upstream endpoint plus ordered preprocessing, with no observable port. */
export interface PendingEndpoint<Value = CircularValue> extends DownstreamActorMethods<Value> {
  readonly [pendingEndpointBrand]: Value;
  into<TargetValue, TargetMode extends HandleMode>(
    target: TargetEndpoint<TargetValue, TargetMode>, options?: EdgeOptions,
  ): this;
}

/** Represents a graph target endpoint selected from an actor's canonical input set. */
export interface TargetEndpoint<
  Value = CircularValue,
  Mode extends HandleMode = HandleMode,
> {
  /** Prevents arbitrary host values from being used as graph targets. */
  readonly [targetEndpointBrand]: { readonly value: Value; readonly mode: Mode };
}

/**
 * The read-role mount verb on a named inlet endpoint (`actor.in.<x>`): a read role takes an observed
 * port ref in either direction. Only inlet *entries* carry it —
 * an actor handle is both a source and a target endpoint, so the verb cannot live on `TargetEndpoint`.
 */
export interface InletObservation {
  /** Observes this inlet under a read role; `request` is not an inlet's role.
   * Requires the installed authoring host; otherwise throws CIRCULAR_EXPORT_MOUNT_HOST_MISSING.
   */
  mount(name: string, role?: "result" | "progress" | "error"): this;
}

/** Represents a read-only observed endpoint used by export and GUI declaration APIs. */
export type ObservedEndpoint<Value = CircularValue> = SourceEndpoint<
  Value,
  HandleMode
>;

/**
 * Represents a source-shaped child ingress boundary that external export injection may write.
 * Only child `projectInput` handles carry this marker in the v2 authored surface.
 */
export interface WritableBoundaryEndpoint<
  Value = CircularValue,
  Mode extends HandleMode = HandleMode,
> extends SourceEndpoint<Value, Mode> {
  /** Separates writable boundaries from ordinary observed source endpoints. */
  readonly [writableBoundaryBrand]: true;
}

/** Represents an edge created in the current declaration epoch. */
export interface NewEdgeHandle {
  /** Prevents confusion with current-state edge handles. */
  readonly [newEdgeBrand]: true;
  /** Replaces delay and policy for this new edge before the epoch is committed. The chained
   * preprocess steps are kept; a `preprocess` member is refused with
   * `CIRCULAR_EDGE_PREPROCESS_REPLACEMENT` — redeclare the edge to change its steps. */
  replaceOptions(options: EdgeOptions): this;
}

/** Represents one accepted edge resolved at an anchored current revision. */
export interface CurrentEdgeHandle {
  /** Holds the stable declared edge identity. */
  readonly edgeId: EdgeId;
  /** Prevents confusion with epoch-local edge handles. */
  readonly [currentEdgeBrand]: true;
  /** Emits the exact edge retirement operation in the current mutation epoch. */
  disconnect(): void;
  /** Replaces delay and policy while preserving the edge's source, target, ordinal and preprocess
   * steps. A `preprocess` member is refused with `CIRCULAR_EDGE_PREPROCESS_REPLACEMENT`; the steps
   * change by redeclaring the same (source, target, ordinal), e.g. `source.map(…).into(target)`. */
  replaceOptions(options: Omit<EdgeOptions, "ordinal">): void;
}
