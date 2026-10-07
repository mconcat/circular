/** @packageDocumentation Settled replay-control and time-machine query declarations. */

import type {
  CircularValue,
  NonEmptyReadonlyArray,
  PositiveInteger,
  QueryDescriptor,
  ReplayArrangement,
  ReplayPace,
  ReplayTarget,
  Result,
  Stamp,
  TimelineAt,
} from "@circular/protocol";
import type { TimelineItem } from "./index.js";

declare const replayNominal: unique symbol;

/** A flat registered display-series name. */
export type DisplayName = string & { readonly [replayNominal]: "DisplayName" };

/** A recorded positive logical-tick bucket width. */
export type PositiveTickCount = number & { readonly [replayNominal]: "PositiveTickCount" };

/** An opaque cursor previously returned by the display-range registration. */
export type DisplayCursor = string & { readonly [replayNominal]: "DisplayCursor" };

/** A store generation used to invalidate cursor positions after schema replacement. */
export type StoreGeneration = string & { readonly [replayNominal]: "StoreGeneration" };

/** An upper record bound for one immutable display-range consistency cut. */
export type RecordBound = string & { readonly [replayNominal]: "RecordBound" };

/** The opaque three-part key of one recorded display lane and optional scale. */
export interface DisplayKey<Args = CircularValue> {
  /** The registered flat display name. */
  readonly name: DisplayName;
  /** Canonical registration-owned display arguments. */
  readonly args: Args;
  /** A recorded bucket width, or null for an unbucketed display lane. */
  readonly bucket: PositiveTickCount | null;
}

/** The closed start position for a record scan. */
export type ScanStart =
  | { readonly kind: "Beginning" }
  | { readonly kind: "After"; readonly cursor: DisplayCursor }
  | { readonly kind: "Tail"; readonly count: PositiveInteger };

/** A closed logical-stamp interval used by a time-machine display request. */
export interface DisplayInterval {
  /** Inclusive interval start. */
  readonly start: Stamp;
  /** Inclusive interval end. */
  readonly end: Stamp;
}

/** Exact arguments of the registered `display-range` query. */
export interface DisplayRequest<Args = CircularValue> {
  /** The display lane and recorded bucket selected by the caller. */
  readonly key: DisplayKey<Args>;
  /** Closed logical interval requested for display. */
  readonly interval: DisplayInterval;
  /** Horizontal pixels allocated to this display region. */
  readonly width: PositiveInteger;
  /** Explicit finite-scan start position. */
  readonly start: ScanStart;
}

/** The record-scan anchor used by a display-range response. */
export interface DisplayRangeAnchor {
  /** Immutable upper record bound. */
  readonly upto: RecordBound;
  /** Store generation in which returned cursors are interpretable. */
  readonly storeGeneration: StoreGeneration;
}

/** Why a requested display interval has no retained record representation. */
export type NoDisplayRecordReason = "ScaleNeverRecorded" | "RetainedAway";

/** An explicit unknown-display interval, distinct from an empty successful page. */
export interface NoDisplayRecord {
  /** The no-record discriminant. */
  readonly kind: "NoRecord";
  /** Interval for which no display fact can be claimed. */
  readonly interval: DisplayInterval;
  /** Whether the scale never existed or retention removed the interval. */
  readonly reason: NoDisplayRecordReason;
}

/** One display-range item, preserving the explicit no-record outcome. */
export type DisplayAnswer<Record> =
  | { readonly kind: "Record"; readonly record: Record }
  | NoDisplayRecord;

/** Diagnostic detail returned when the requested display scale was never recorded. */
export interface ScaleUnavailable {
  /** The requested unrecorded scale. */
  readonly requested: PositiveTickCount;
  /** Every available recorded scale in canonical order. */
  readonly available: NonEmptyReadonlyArray<PositiveTickCount>;
}

/** The protocol-owned typed descriptor for the registered display-range query. */
export declare const displayRange: QueryDescriptor<
  DisplayRequest,
  DisplayAnswer<CircularValue>,
  DisplayRangeAnchor,
  "cursor"
>;

/** Deterministically selects a recorded scale for a display interval and pixel width. */
export declare function chooseDisplayScale(
  interval: DisplayInterval,
  recordedScales: NonEmptyReadonlyArray<PositiveTickCount>,
  width: PositiveInteger,
): PositiveTickCount;

/**
 * A replay pace whose Step destination is a replay coordinate: a `timeline.at` answer's `target`,
 * or a `timeline` checkpoint (the same coordinate with its instant, which the lens drops).
 */
export type ReplayCheckpointPace =
  | "Free"
  | "Paused"
  | { readonly kind: "Step"; readonly upto: TimelineItem | TimelineAt["target"] }
  | { readonly kind: "Realtime"; readonly num: bigint; readonly den: bigint };

export interface ReplayLens {
  /** The lens's correlation key on its connection — what a read names to go through this lens. */
  readonly correlation: number;
  rewind(request: {
    readonly to?: TimelineItem | TimelineAt["target"];
    readonly pace: ReplayCheckpointPace;
  }): Promise<Result<unknown>>;
  /** Closes the lens; the subscriptions naming it end with `TargetGone`. Live reads are untouched. */
  end(): Promise<Result<unknown>>;
}

export interface ReplayPartition {
  /** Opens a lens at a coordinate with a pace, on a correlation key of its own. */
  start(request: {
    readonly from: TimelineItem | TimelineAt["target"];
    readonly pace: ReplayCheckpointPace;
  }): Promise<Result<ReplayLens>>;
}

/** Re-exports the closed replay arrangement union for replay-subpath consumers. */
export type { ReplayArrangement };

/** Re-exports the closed replay pace union used by start and rewind payloads. */
export type { ReplayPace };

/** Re-exports the replay coordinate `{stream, revision_epoch, cut}`. */
export type { ReplayTarget };
