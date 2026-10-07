/** Internal exact wire witnesses; public names remain the registered target strings. */
import type { AuthoringCommitArguments, AuthoringCommitFrame } from '../index.js';
import type { CircularUInt } from '../value.js';
import type { QueryDescriptor, SubscriptionDescriptor } from '../index.js';
type Scope = readonly unknown[];
type Producer = 1n | 2n | {readonly scope: Scope; readonly local: string};
export type RecordStamp = readonly [CircularUInt, CircularUInt, Producer, CircularUInt, CircularUInt];
/** The four-component effect key `approval_payload.rs` puts on the wire — the `runtime.approvals`
 * `item` carrier, kept without reinterpretation. */
type ApprovalEffectKey = readonly [unknown, unknown, unknown, unknown];
export interface ActorEventsItem {
  readonly [field: string]: unknown;
  readonly origin?: RecordStamp;
  /** This row's own recorded stamp. Absence remains unknown. */
  readonly at?: RecordStamp;
  /** Recorded causal parents, in recorded order. [] means no recorded parents;
   * absence remains unknown. Wire and SDK use the same spelling. */
  readonly causal_parents?: readonly RecordStamp[];
  /** On an effect settlement row, the occasion its recorded EffectId names — the
   * causing arrival's stamp and, when it came on one, `edge`, spelled as a row spells `origin`
   * and `edge`. For an edge delivery, a scheduled fire or an outcome return, the same column's
   * row with that `origin` (and `edge`, when both carry one) is the causing arrival; an injected
   * row's `origin` is its recorded causal parent, so no row matches an injection-caused occasion.
   * Absent when no arrival caused the effect (a poll). */
  readonly occasion?: { readonly origin: RecordStamp; readonly edge?: unknown };
  readonly port?: string;
  /** On the effect outcome row where the requesting actor recorded the decision on its
   * approval request. `item` is that request — the same value as its `runtime.approvals` row's
   * `item`. Only an approved decision's recorded body carries `target_effect`: a denied decision
   * names no target effect, because the recorded denial body carries none. */
  readonly approval?:
    | { readonly item: ApprovalEffectKey; readonly decision: 'approved'; readonly target_effect: ApprovalEffectKey }
    | { readonly item: ApprovalEffectKey; readonly decision: 'denied' };
  /** On the result row of an effect that used an approved ticket. `item` is the request
   * that issued the ticket. A decision and a consumption are separate facts in separate fields.
   * A consumption shows only after the target's result is recorded; while the target runs, the
   * request reads as approved only. */
  readonly ticket?: { readonly item: ApprovalEffectKey };
}
export interface Reached { readonly revision_epoch: CircularUInt; readonly cut: readonly unknown[] }
export interface RecordsCursor {
  readonly anchor: readonly [Uint8Array, Scope];
  readonly domain: 'records';
  /** Opaque resume position of the recorded fact. */
  readonly position: Uint8Array;
}
export interface RecordsArguments { readonly scope: Scope; readonly since?: RecordsCursor }
export interface RecordsItem {
  readonly cursor: RecordsCursor;
  readonly fact: Uint8Array;
  /** Recorded observation bucket in milliseconds, present only on observation records.
   * UInt values beyond Number.MAX_SAFE_INTEGER are rejected rather than rounded. */
  readonly observationBucket?: number;
  /** Decoded by the daemon's kernel codec; no SDK record-envelope reader. */
  readonly system?: { readonly producer: 'Pipeline'; readonly revision: CircularUInt } & (
    | { readonly kind: 'PauseAccepted'; readonly force: boolean }
    | { readonly kind: 'ResumeAccepted' }
    | { readonly kind: 'ActivationOutcome' | 'RevisionAdoptionOutcome' | 'RecoveryOutcome'; readonly code: 22 | 24 | 30 | null }
  );
  readonly reached?: Reached;
}
/** An `edge.depths` frame for one edge into an actor that answered, measured by that
 * receiving actor at its own inlet. */
export interface EdgeDepthsEdgeRow {
  readonly edge: unknown;
  readonly depth: CircularUInt;
  readonly queued: CircularUInt;
  readonly capacity: CircularUInt | null;
}
/** An `edge.depths` frame for an asked actor that ended before it answered —
 * `code` is the daemon's `EndedBeforeAnswering` number. */
export interface EdgeDepthsEndedRow {
  readonly actor: { readonly scope: Scope; readonly local: string };
  readonly code: CircularUInt;
}
export declare function edgeDepthsItemFromValue(value: unknown): EdgeDepthsEdgeRow | EdgeDepthsEndedRow;
export declare function recordStampFromValue(value: unknown): RecordStamp;
export declare function actorEventsItemFromValue(value: unknown): ActorEventsItem;
export declare function reachedFromValue(value: unknown): Reached;
export declare function recordsCursorFromValue(value: unknown): RecordsCursor;
export declare function recordsArgumentsValue(value: unknown): RecordsArguments;
export declare function recordsItemFromValue(value: unknown): RecordsItem;
export declare function recordQueryPageFromValue<T>(value: unknown, readItem: (value: unknown) => T): {
  readonly cut?: readonly unknown[]; readonly folded_from?: readonly unknown[];
  readonly anchor: unknown; readonly items: readonly T[];
  readonly terminal: bigint | readonly [bigint, unknown]; readonly reached?: Reached;
};
export declare const recordRegistrations: {
  readonly 'authoring-commits': {
    readonly subscription: SubscriptionDescriptor<AuthoringCommitArguments, AuthoringCommitFrame, bigint, 'credit'>;
    readonly item: (value: unknown) => AuthoringCommitFrame;
    readonly args: (value: unknown) => AuthoringCommitArguments;
  };
  readonly 'actor.events': {
    readonly query: QueryDescriptor<null, ActorEventsItem, unknown, 'cursor'>;
    readonly subscription: SubscriptionDescriptor<null, ActorEventsItem, unknown, 'credit'>;
    readonly item: typeof actorEventsItemFromValue;
    readonly args: (value: unknown) => null;
  };
  readonly 'edge.depths': {
    /** Null args, credit discipline; one row per frame as each asked actor answers,
     * ending `Complete` once every asked actor answered or ended. */
    readonly subscription: SubscriptionDescriptor<null, EdgeDepthsEdgeRow | EdgeDepthsEndedRow, unknown, 'credit'>;
    readonly item: typeof edgeDepthsItemFromValue;
    readonly args: (value: unknown) => null;
  };
  readonly records: {
    readonly query: QueryDescriptor<RecordsArguments, RecordsItem, RecordsCursor['anchor'], 'cursor'>;
    readonly subscription: SubscriptionDescriptor<RecordsArguments, RecordsItem, RecordsCursor['anchor'], 'credit'>;
    readonly item: typeof recordsItemFromValue;
    readonly subscriptionItem: typeof recordsItemFromValue;
    readonly args: typeof recordsArgumentsValue;
  };
};

export type { DeadLetterItem, DeadLetterTarget } from '../index.js';
export { deadLetterTargetFromValue, deadLetterItemFromValue } from '../index.js';
export declare const deadLetterRegistration: {
  readonly query: QueryDescriptor<null, import('../index.js').DeadLetterItem, bigint, 'cursor'>;
  readonly item: typeof import('../index.js').deadLetterItemFromValue;
  readonly args: (value: unknown) => null;
};
export declare const DAEMON_HEALTH_REASONS: readonly string[];
