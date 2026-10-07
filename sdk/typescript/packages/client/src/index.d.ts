/** @packageDocumentation Established-session client surface for Circular's stable RPC partitions. */

/** Published by circular_transport::OWNER_LOCAL_SOCKET_NAME. */
export declare const OWNER_LOCAL_SOCKET_NAME: 'daemon.sock';
export declare const OWNER_LOCAL_RESOURCE_CEILINGS: Readonly<{
  maximumBytes: 1048576; maximumDepth: 64;
  maximumContainerEntries: 65535; maximumStringBytes: 1048576;
}>;

import type {
  AbsoluteAddressDomain,
  Accepted,
  AuthoringCommitArguments,
  AuthoringCommitFrame,
  AuthoringEnvironment,
  AuthoringSnapshotAnchor,
  CommitEpochCommand,
  DeclarationAccepted,
  DeclarationContentCommand,
  Diagnostic,
  EpochContentRequest,
  EpochAddressDomain,
  ExportName,
  FeatureSet,
  Goodbye,
  InjectRequest,
  InjectionAccepted,
  LedgerTransitionAccepted,
  LosslessFrame,
  ConflatedFrame,
  CreditFrame,
  NonEmptyReadonlyArray,
  PageRequest,
  PositiveInteger,
  QueryDescriptor,
  QueryPage,
  QueryPaging,
  ReconstructiveDeclarationCommand,
  Rejected,
  RelativeAddressDomain,
  Result,
  RetentionComplete,
  SessionId,
  SessionRole,
  SessionToken,
  SetObservationControlRequest,
  ApprovalDecideRequest,
  SubscriptionDescriptor,
  SubscriptionEnded,
  SubscriptionOpened,
  TransportTrust,
  AcceptedCommitMetadata,
  StructureCursor,
  TargetName,
  CorrelationId,
  EpochId,
  BeginEpochCommand,
  ValidateEpochCommand,
  AbortEpochCommand,
  CurrentAuthoringRevision,
  EpochOpened,
  CandidateApplied,
  EpochValidated,
  EpochCommitted,
  EpochAborted,
  NonNegativeInteger,
} from "@circular/protocol";
import type { ActorHealthReasonCode, ActorHealthState } from "@circular/protocol/tables";
import type { ReplayPartition } from "./replay.js";
import type { OwnerLocalSession } from "./owner-local-session.js";

/** Session-establishment options for Circular's owner-local byte wire. */
export interface EstablishOptions {
  /** The value codec ceilings have no default; the caller must choose them. */
  readonly resourceCeilings: unknown;
  /** Optional protocol version, feature minor, and requested roles encoded as `Hello`. */
  readonly hello?: {
    readonly protocolVersion?: number;
    readonly minor?: number;
    readonly requestedRoles?: readonly unknown[];
    readonly resume?: Uint8Array;
  };
  /** Optional upper bound on simultaneously live request correlations. */
  readonly maxInFlight?: PositiveInteger;
  /** Optional request wait bound. Omission deliberately means no timeout. */
  readonly requestTimeoutMs?: number;
}

/** Visible finite default used when `EstablishOptions.maxInFlight` is omitted. */
export declare const DEFAULT_MAX_IN_FLIGHT: 256;

/** Establishes one session over the byte wire, including its Hello/HelloAck exchange. */
export declare function establish(
  transport: unknown,
  options: EstablishOptions,
): Promise<OwnerLocalSession>;

/** The established byte-wire session returned by `establish`. */
export type { OwnerLocalSession } from "./owner-local-session.js";

/** One immediately identifiable client request and its eventual structured protocol result. */
export interface CorrelatedRequest<AcceptedValue> {
  /** The session-local request correlation allocated before transport submission. */
  readonly correlation: CorrelationId;
  /** The eventual accepted-or-rejected result; normal rejection does not reject this promise. */
  readonly completion: Promise<Result<AcceptedValue>>;
}

/** A request accepted by the declaration partition. */
export type DeclarationRequest =
  | BeginEpochCommand<EpochAddressDomain>
  | EpochContentRequest
  | ValidateEpochCommand
  | CommitEpochCommand
  | AbortEpochCommand;

/** Low-level access to the existing declaration epoch RPCs. */
export interface DeclarationPartition {
  /** Opens an isolated candidate at an explicit revision and environment baseline. */
  begin(command: BeginEpochCommand<EpochAddressDomain>): CorrelatedRequest<EpochOpened>;
  /** Applies one existing declaration content command to an open candidate. */
  apply(request: EpochContentRequest): CorrelatedRequest<CandidateApplied>;
  /** Validates an open candidate without closing it. */
  validate(command: ValidateEpochCommand): CorrelatedRequest<EpochValidated>;
  /** Attempts one atomic declaration commit. */
  commit(command: CommitEpochCommand): CorrelatedRequest<EpochCommitted>;
  /** Explicitly discards an open candidate. */
  abort(command: AbortEpochCommand): CorrelatedRequest<EpochAborted>;
  /** Sends a same-partition batch and preserves one ordered result per request. */
  batch(requests: NonEmptyReadonlyArray<DeclarationRequest>): Promise<readonly Result<DeclarationAccepted>[]>;
}

/** A complete, anchored query outcome. */
export interface CompletePageResult<Item, Anchor> {
  readonly cut?: readonly unknown[];
  readonly folded_from?: readonly unknown[];
  /** The normal-completion discriminant. */
  readonly kind: "Complete";
  /** The immutable consistency anchor shared by every consumed page. */
  readonly anchor: Anchor;
  /** Every ordered item from the finite page stream. */
  readonly items: readonly Item[];
}

/** A diagnostic query outcome that preserves but does not promote partial items. */
export interface PartialPageResult<Item, Anchor> {
  readonly cut?: readonly unknown[];
  readonly folded_from?: readonly unknown[];
  /** The diagnostic-completion discriminant. */
  readonly kind: "Partial";
  /** The immutable consistency anchor shared by every accepted page. */
  readonly anchor: Anchor;
  /** Ordered items received before diagnostic termination. */
  readonly items: readonly Item[];
  /** The terminal diagnostic; callers must issue a fresh query to obtain completeness. */
  readonly diagnostic: Diagnostic;
}

/** The only two terminal outcomes of an already accepted page stream. */
export type PageOutcome<Item, Anchor> = CompletePageResult<Item, Anchor> | PartialPageResult<Item, Anchor>;

/** An accepted finite query stream that retains anchors and explicit terminal pages. */
export interface PageStream<Item, Anchor> extends AsyncIterable<QueryPage<Item, Anchor>> {
  /** The request correlation retained until a terminal page is observed. */
  readonly correlation: CorrelationId;
  /** The first page's anchor, shared by the entire stream. */
  readonly anchor: Anchor;
  /** Consumes through the explicit terminal page without converting partial output into complete output. */
  collect(): Promise<PageOutcome<Item, Anchor>>;
}

/** A typed query call; cursor-paged registrations require an explicit positive page bound. */
export type QueryCall<Args, Item, Anchor, Paging extends QueryPaging> = Paging extends "cursor"
  ? {
      /** The feature-owned typed registration descriptor. */
      readonly descriptor: QueryDescriptor<Args, Item, Anchor, Paging>;
      /** First-order registered arguments. */
      readonly args: Args;
      readonly since?: readonly unknown[];
      readonly upto?: readonly unknown[];
      /** Explicit page bound; no fixed client-side default is hidden here. */
      readonly limit: PositiveInteger;
    }
  : {
      /** The feature-owned typed registration descriptor. */
      readonly descriptor: QueryDescriptor<Args, Item, Anchor, Paging>;
      /** First-order registered arguments. */
      readonly args: Args;
      readonly since?: readonly unknown[];
      readonly upto?: readonly unknown[];
    };

/** Extracts the accepted stream type produced by a typed query call. */
export type QueryCallResult<C> = C extends QueryCall<infer _Args, infer Item, infer Anchor, infer _Paging>
  ? Result<PageStream<Item, Anchor>>
  : never;

/** Query-partition access that never hides anchors, page bounds, or terminal markers. */
export interface QueryPartition {
  /** Opens one finite query stream from a feature-owned descriptor. */
  open<Args, Item, Anchor, Paging extends QueryPaging>(
    call: QueryCall<Args, Item, Anchor, Paging>,
  ): Promise<Result<PageStream<Item, Anchor>>>;
  /** Opens a same-partition batch and preserves each call's distinct result type. */
  batch<const Calls extends readonly QueryCall<unknown, unknown, unknown, QueryPaging>[]>(
    calls: Calls,
  ): Promise<{ readonly [Index in keyof Calls]: QueryCallResult<Calls[Index]> }>;
}

/** Events emitted by an accepted lossless subscription. */
export type LosslessSubscriptionEvent<Payload, Anchor> =
  | LosslessFrame<Payload, Anchor>
  | RetentionComplete<Anchor>
  | SubscriptionEnded<Anchor>;

/** Events emitted by an accepted conflated subscription. */
export type ConflatedSubscriptionEvent<Payload, Anchor> =
  | ConflatedFrame<Payload, Anchor>
  | RetentionComplete<Anchor>
  | SubscriptionEnded<Anchor>;

/** Events emitted by an accepted credit subscription. */
export type CreditSubscriptionEvent<Payload, Anchor> =
  | CreditFrame<Payload, Anchor>
  | RetentionComplete<Anchor>
  | SubscriptionEnded<Anchor>;

/** Common lifetime and identity of one accepted subscription. */
export interface SubscriptionHandle<Anchor> {
  /** Correlation identity used by frames, unsubscribe, credit, and terminal values. */
  readonly correlation: CorrelationId;
  /** Registered flat target name. */
  readonly target: TargetName;
  /** Catch-up and live consistency anchor. */
  readonly anchor: Anchor;
  /** Fixed server-selected retained-frame depth. */
  readonly retentionDepth: NonNegativeInteger;
  /** Requests typed `ByClient` termination after already queued frames. */
  unsubscribe(): Promise<Result<{ readonly subscription: CorrelationId }>>;
}

/** An accepted lossless subscription; it intentionally exposes no credit operation. */
export interface LosslessSubscription<Payload, Anchor> extends SubscriptionHandle<Anchor>, AsyncIterable<LosslessSubscriptionEvent<Payload, Anchor>> {
  /** The server-selected delivery discipline. */
  readonly discipline: "lossless";
}

/** An accepted conflated subscription with visible folded-frame accounting. */
export interface ConflatedSubscription<Payload, Anchor> extends SubscriptionHandle<Anchor>, AsyncIterable<ConflatedSubscriptionEvent<Payload, Anchor>> {
  /** The server-selected delivery discipline. */
  readonly discipline: "conflated";
}

/** An accepted credit subscription whose consumer explicitly grants processed capacity. */
export interface CreditSubscription<Payload, Anchor> extends SubscriptionHandle<Anchor>, AsyncIterable<CreditSubscriptionEvent<Payload, Anchor>> {
  /** The server-selected delivery discipline. */
  readonly discipline: "credit";
  /** Adds a positive frame count without enabling hidden automatic replenishment. */
  grant(amount: PositiveInteger): Promise<Result<{ readonly pending_after: import("@circular/protocol/actor-query").CircularUInt }>>;
}

/** Selects the only handle subtype valid for a descriptor's registered discipline. */
export type SubscriptionFor<Payload, Anchor, Discipline> =
  Discipline extends "lossless" ? LosslessSubscription<Payload, Anchor>
    : Discipline extends "conflated" ? ConflatedSubscription<Payload, Anchor>
      : Discipline extends "credit" ? CreditSubscription<Payload, Anchor>
        : never;

/** A typed subscription-opening call with no caller-selected discipline or retention depth. */
export interface SubscriptionCall<Args, Payload, Anchor, Discipline extends "lossless" | "conflated" | "credit"> {
  /** The feature-owned typed target descriptor. */
  readonly descriptor: SubscriptionDescriptor<Args, Payload, Anchor, Discipline>;
  /** First-order registered target arguments. */
  readonly args: Args;
}

/** Extracts the accepted handle type produced by a typed subscription call. */
export type SubscriptionCallResult<C> = C extends SubscriptionCall<
  infer _Args,
  infer Payload,
  infer Anchor,
  infer Discipline
>
  ? Result<SubscriptionFor<Payload, Anchor, Discipline>>
  : never;

/** Subscription-partition access preserving the registration-selected delivery discipline. */
export interface SubscriptionPartition {
  /** Opens one subscription without accepting a discipline or retention override. */
  open<Args, Payload, Anchor, Discipline extends "lossless" | "conflated" | "credit">(
    call: SubscriptionCall<Args, Payload, Anchor, Discipline>,
  ): Promise<Result<SubscriptionFor<Payload, Anchor, Discipline>>>;
  /** Opens a same-partition batch and preserves each call's distinct handle result. */
  batch<
    const Calls extends readonly SubscriptionCall<
      unknown,
      unknown,
      unknown,
      "lossless" | "conflated" | "credit"
    >[],
  >(calls: Calls): Promise<{ readonly [Index in keyof Calls]: SubscriptionCallResult<Calls[Index]> }>;
}

/** Client operations over the event-injection and durable-ledger partitions. */
export interface InteractionPartitions {
  /** Injects one event through a declared export request role. */
  inject(request: InjectRequest): Promise<Result<InjectionAccepted>>;
  /** Sends only event-injection envelopes and preserves one result per request. */
  batchInjections(requests: NonEmptyReadonlyArray<InjectRequest>): Promise<readonly Result<InjectionAccepted>[]>;
  /** Commits one terminal decision for a pending durable approval item. */
  decideApproval(request: ApprovalDecideRequest): Promise<Result<LedgerTransitionAccepted>>;
  /** Commits one registered observation-control transition. */
  setObservationControl(request: SetObservationControlRequest): Promise<Result<LedgerTransitionAccepted>>;
  /** Sends only ledger-transition envelopes and preserves one result per request. */
  batchLedgerTransitions(
    requests: NonEmptyReadonlyArray<ApprovalDecideRequest | SetObservationControlRequest>,
  ): Promise<readonly Result<LedgerTransitionAccepted>[]>;
}

/** The accepted authoring-snapshot query arguments. */
export interface AuthoringSnapshotArguments {
  /** User canvas or authored descendant scope to reconstruct. */
  readonly scope: import("@circular/protocol").ScopeId;
}

/** The protocol-owned typed descriptor for compacted current-state reconstruction. */
export declare const authoringSnapshot: QueryDescriptor<
  AuthoringSnapshotArguments,
  ReconstructiveDeclarationCommand<RelativeAddressDomain>,
  AuthoringSnapshotAnchor,
  "cursor"
>;

/** The protocol-owned credit-controlled target that continues accepted commits after a snapshot cursor. */
export declare const authoringCommits: SubscriptionDescriptor<
  AuthoringCommitArguments,
  AuthoringCommitFrame,
  StructureCursor,
  "credit"
>;

/** An established session whose properties are the server-confirmed values, never the requested proposal. */
export interface Session {
  /** Engine-issued identity used only where records or payloads need to name this session. */
  readonly id: SessionId;
  /** Roles actually established by the server. */
  readonly roles: ReadonlySet<SessionRole>;
  /** Trust actually established from the transport channel. */
  readonly trust: TransportTrust;
  /** Per-partition feature meet fixed for this session's lifetime. */
  readonly features: FeatureSet;
  readonly token: SessionToken;
  /** Existing declaration epoch messages. */
  readonly declarations: DeclarationPartition;
  /** Registered finite state and record queries. */
  readonly queries: QueryPartition;
  /** Registered live target subscriptions. */
  readonly subscriptions: SubscriptionPartition;
  /** Export injection and durable ledger transitions. */
  readonly interactions: InteractionPartitions;
  /** Settled replay-control operations and descriptors. */
  readonly replay: ReplayPartition;
  /** Gracefully closes this session and resolves every live request and subscription. */
  close(command?: Goodbye): Promise<void>;
}

/** Re-exports the normal protocol success value for client-only consumers. */
export type { Accepted };

/** Re-exports the normal structured rejection value for client-only consumers. */
export type { Rejected };

export declare const reasons: Readonly<{ RevisionConflict: 26 }>;

/** Re-exports the normal result channel for client-only consumers. */
export type { Result };

/** Re-exports authoring commit metadata used by optimistic commit-cursor absorption. */
export type { AcceptedCommitMetadata };

/** Re-exports current authoring revision values used as mandatory mutation baselines. */
export type { CurrentAuthoringRevision };

/** Re-exports the pinned environment used as the second mandatory mutation baseline. */
export type { AuthoringEnvironment };

/** Re-exports the accepted absolute command type used by commit feeds. */
export type AcceptedDeclarationCommand = DeclarationContentCommand<AbsoluteAddressDomain>;

export { adaptAuthoringSession, actorCatalog, actorCreateAdmission, actorCreateInputs, authoringActorPorts } from './authoring-adapter.js';

export declare const actorEvents: typeof import('../../protocol/src/internal/record-values.js').recordRegistrations['actor.events']['query'];
export declare const actorEventsSubscription: typeof import('../../protocol/src/internal/record-values.js').recordRegistrations['actor.events']['subscription'];
export declare const records: typeof import('../../protocol/src/internal/record-values.js').recordRegistrations['records']['query'];
export declare const recordsSubscription: typeof import('../../protocol/src/internal/record-values.js').recordRegistrations['records']['subscription'];

/**
 * Reads retained and live records until the first System ActivationOutcome,
 * RevisionAdoptionOutcome or RecoveryOutcome whose revision is at or after `revision`, the
 * accepted commit's metadata.cursor (not its authoringAfter digest). `adopted` means that
 * outcome's revision stands; `failed` carries its code and is not retried. Both return the
 * outcome's own revision. No polling or runtime deadline. Session failure or subscription
 * termination returns `closed` with its existing code and reason and the requested revision;
 * every exit releases the subscription.
 */
export declare function waitForAdoption(session: Pick<OwnerLocalSession, 'hold'>, revision: bigint): Promise<
  | { readonly status: 'adopted'; readonly revision: bigint; readonly code: null }
  | { readonly status: 'failed'; readonly revision: bigint; readonly code: 22 | 24 | 30 }
  | { readonly status: 'closed'; readonly revision: bigint; readonly code: string | number; readonly reason: string }
>;

export type ObservationUInt = import('@circular/protocol/actor-query').CircularUInt;
export type ObservationScope = readonly unknown[];
export type ObservationActor = { readonly scope: ObservationScope; readonly local: string };
export type ObservationCut = readonly { readonly actor: unknown; readonly index: bigint }[];
export interface ObservationPage<Item, Anchor = unknown> {
  readonly anchor: Anchor;
  readonly items: readonly Item[];
  readonly terminal: 2n | readonly [1n, unknown] | readonly [3n, bigint];
  readonly cut?: ObservationCut;
  readonly folded_from?: ObservationCut;
  readonly reached?: { readonly revision_epoch: ObservationUInt; readonly cut: ObservationCut };
}
export interface DaemonHealthAnchor {
  readonly version: ObservationUInt;
  readonly lifecycle: 'running' | 'stopped' | 'activation_failed' | 'revision_adoption_failed'
    | 'recovery_failed' | null;
  readonly storage: ObservationUInt | null;
  readonly dead_letter: import('../../protocol/src/internal/record-values.js').RecordStamp | null;
  /**
   * The latest recorded stream start or restart's wall clock (`wall_ms`, Unix ms) and the
   * run-clock coordinate of that record (`at_ms`, the axis `observed_at_ms`·`at_ms`·`since_ms`
   * use). A run-clock time `t >= at_ms` is at wall time `wall_ms + (t - at_ms)`; an earlier time
   * belongs to an earlier lifetime this pair cannot place. The lifetime that opened the stream
   * has its start record's pair; `null` before the stream exists.
   */
  readonly wall_clock: { readonly at_ms: ObservationUInt; readonly wall_ms: ObservationUInt } | null;
  readonly config_defaults: readonly { readonly key: string; readonly value: ObservationUInt }[];
  readonly journal: {
    readonly ceiling: { readonly code: string; readonly record: ObservationUInt; readonly since_ms: ObservationUInt } | null;
    readonly usage: Readonly<Record<
      'arrivals_max_bytes' | 'arrivals_max_records' | 'bytes' | 'file_bytes' | 'records' | 'total_max_bytes',
      ObservationUInt>> | null;
  };
}
export type DaemonHealthItem = {
  readonly actor: ObservationActor;
  readonly reason: (typeof ActorHealthReasonCode)[number] | null;
  /** One-based arrival-journal record ordinal, not a commit or a time coordinate. */
  readonly record: ObservationUInt;
  readonly since_ms: ObservationUInt;
  readonly detail: { readonly code: string; readonly slot: string | null } | null;
} & (
  | { readonly state: (typeof ActorHealthState)[number]; readonly recorded?: undefined }
  | { readonly state: 'not_standing'; readonly recorded: (typeof ActorHealthState)[number] }
);
export type DaemonHealthPage = ObservationPage<DaemonHealthItem, DaemonHealthAnchor>;
/** Reads a `daemon.health` answer: its anchor and rows as recorded, each row read against the anchor's
 * `lifecycle` word (see `DaemonHealthItem`). This is the one place that reading is made; the canvas reads
 * health through it. */
export declare function daemonHealthPageFromValue(value: unknown): DaemonHealthPage;
export declare const daemonHealth: QueryDescriptor<null, DaemonHealthItem, DaemonHealthAnchor, 'none'>;

export interface TimelineItem {
  readonly at_ms: bigint;
  readonly stream: bigint;
  readonly cut: ObservationCut;
  readonly revision_epoch: ObservationUInt;
}
export interface TimelineAnchor {
  readonly epoch_plans: readonly [1n, {
    readonly latest_revision_epoch: ObservationUInt; readonly source_checkpoints: ObservationUInt;
  }] | readonly [2n, { readonly reason: string }];
}
export type TimelinePage = ObservationPage<TimelineItem, TimelineAnchor>;
export declare function timelinePageFromValue(value: unknown): TimelinePage;
export declare const timeline: QueryDescriptor<null, TimelineItem, TimelineAnchor, 'cursor'>;

export type RecordsCursor = import('../../protocol/src/internal/record-values.js').RecordsCursor;
export interface RecordsItem {
  readonly cursor: RecordsCursor;
  /** Recorded observation bucket in milliseconds, present only on observation records.
   * UInt values beyond Number.MAX_SAFE_INTEGER are rejected rather than rounded. */
  readonly observationBucket?: number;
  readonly reached?: ObservationPage<never>['reached'];
  /** ProductRecordCodec bytes, including Pipeline System observations 41–45. */
  readonly fact: Uint8Array;
  /** Kernel-decoded System observation, also present on records subscription items. */
  readonly system?: import('../../protocol/src/internal/record-values.js').RecordsItem['system'];
}
export type RecordsPage = ObservationPage<RecordsItem>;
/** Inherits the recorded `at` and `causal_parents` stamps with their wire spelling, and on effect
 * outcome rows the recorded `approval` decision and `ticket` use. A ticket use shows
 * only after the target's result is recorded. */
export interface ActorEventsItem extends WireActorEventsItem {
  readonly actor: ObservationActor;
  readonly index: bigint;
  readonly observed_at_ms: bigint;
  readonly body: unknown;
  readonly edge?: unknown;
}
import type { ActorEventsItem as WireActorEventsItem, RecordStamp } from '../../protocol/src/internal/record-values.js';
export type ActorEventsPage = ObservationPage<ActorEventsItem>;
export declare function recordsPageFromValue(value: unknown): RecordsPage;
export declare function actorEventsPageFromValue(value: unknown): ActorEventsPage;

export type ActorEventParent =
  /** The one given row that carried the event this stamp names. */
  | { readonly stamp: RecordStamp; readonly status: 'found'; readonly row: ActorEventsItem }
  /** No given row carried it. The answer is over the rows given; this function reads nothing more.
   * A `_lifecycle` row's parent is the record that minted its cell, which is no actor.events row. */
  | { readonly stamp: RecordStamp; readonly status: 'absent'; readonly code: 'not_in_rows' }
  /** Recorded fields do not pick one row, so none is named; `candidates` are the given rows they allow.
   * `emission_arrived_twice`: the sending actor recorded two arrivals of the emission
   * the stamp names (one emission on two of its inlets or wires).
   * `no_sending_end`: the row did not arrive over a wire and the stamp names an
   * emission, so nothing recorded says which arrival of it is meant.
   * `stamp_shared`: a given row's `at` is the stamp and another given row also carries
   * it, or a given row carries a `_lifecycle` row's parent (a record, never a row), so two events
   * were issued the same stamp. `candidates` are every given row that carries it; the event the
   * stamp names may be none of them. */
  | {
      readonly stamp: RecordStamp; readonly status: 'undecidable';
      readonly code: 'emission_arrived_twice' | 'no_sending_end' | 'stamp_shared';
      readonly candidates: readonly ActorEventsItem[];
    };
export declare function actorEventParents(row: ActorEventsItem, rows: readonly ActorEventsItem[]):
  | { readonly status: 'recorded'; readonly parents: readonly ActorEventParent[] }
  | { readonly status: 'unknown' };

/** Four-component effect key owned by approval_payload.rs, retained without reinterpretation. */
export type ApprovalEffectKey = readonly [unknown, unknown, unknown, unknown];
export interface RuntimeApprovalItem {
  readonly item: ApprovalEffectKey;
  readonly emitter: 3n | 4n | readonly [1n, ObservationScope, string] | readonly [2n, ObservationScope, Uint8Array];
  readonly target_effect: ApprovalEffectKey;
  readonly state: 1n | readonly [2n, ApprovalEffectKey, ApprovalEffectKey];
  readonly summary: readonly [2n, 1n];
  /** The recorded arrival that called this request: the emitter's own `actor.events` row
   * `{actor, index}` (also a `since` cut component for that actor). Its body is the call.
   * `null` when the queue records no arrivals. */
  readonly cause: { readonly actor: ObservationActor; readonly index: bigint } | null;
}
export interface RuntimeApprovalsAnchor {
  readonly producer: 1n | readonly [2n, 1n];
  readonly persistence: readonly [3n] | readonly [2n, 1n];
}
export type RuntimeApprovalsPage = ObservationPage<RuntimeApprovalItem, RuntimeApprovalsAnchor>;
export declare function runtimeApprovalsPageFromValue(value: unknown): RuntimeApprovalsPage;
export declare const runtimeApprovals: QueryDescriptor<null, RuntimeApprovalItem, RuntimeApprovalsAnchor, 'none'>;
export interface QueryCatalogAnchor {
  readonly preprocess: readonly {
    readonly kind: import('@circular/protocol').PreprocessStep['kind'];
    readonly slots: readonly import('@circular/protocol').CircularValue[];
  }[];
  readonly queries: readonly string[];
}
/** Items are the per-registration descriptors; the canon does not spell their fields. */
export type QueryCatalogPage = ObservationPage<unknown, QueryCatalogAnchor>;
export declare function queryCatalogPageFromValue(value: unknown): QueryCatalogPage;
export declare const deadLetters: QueryDescriptor<null, import('@circular/protocol').DeadLetterItem, null, 'cursor'>;
export type DeadLettersPage = ObservationPage<import('@circular/protocol').DeadLetterItem, null>;
export declare function deadLettersPageFromValue(value: unknown): DeadLettersPage;
export declare const instanceTransitions: QueryDescriptor<null, unknown, null, 'cursor'>;
export type InstanceTransitionsPage = ObservationPage<unknown, null>;
export declare function instanceTransitionsPageFromValue(value: unknown): InstanceTransitionsPage;
export declare const queryCatalog: QueryDescriptor<null, unknown, QueryCatalogAnchor, 'none'>;
/** `agent.harness-candidates` row: one declared harness adapter and where the daemon found its
 * program. `found` is the first of that adapter's declared install candidates a bind would accept
 * when the query was answered (an absolute path), or `null` when none is. */
export interface AgentHarnessCandidateItem {
  readonly name: string;
  readonly found: string | null;
}
/** `agent.harness-candidates` — Null args, Null anchor, one complete page; one row per declared
 * harness adapter in the daemon's declaration order. The daemon measures its own home and
 * filesystem at the query and does not read PATH. The answer binds nothing: a binding is one
 * `SetAgentHarness` command (`setAgentHarness`), and `agent.harnesses` lists the bindings. */
export declare const agentHarnessCandidates: QueryDescriptor<null, AgentHarnessCandidateItem, null, 'none'>;
export type AgentHarnessCandidatesPage = ObservationPage<AgentHarnessCandidateItem, null>;
export declare function agentHarnessCandidatesPageFromValue(value: unknown): AgentHarnessCandidatesPage;
export interface AgentHarnessItem {
  readonly name: string;
  readonly program: string | null;
  readonly saved: string | null;
}
/** `agent.harnesses` — Null args, one complete page in name order; the anchor is the same rows.
 * The two columns are read separately at the query and are not merged. They differ when a
 * binding was saved before a pipeline stood, when the document was edited by hand while the
 * daemon runs (the daemon reads the document when a pipeline stands), or when the standing System
 * did not take a command's binding. */
export declare const agentHarnesses: QueryDescriptor<null, AgentHarnessItem, readonly AgentHarnessItem[], 'none'>;
export type AgentHarnessesPage = ObservationPage<AgentHarnessItem, readonly AgentHarnessItem[]>;
export declare function agentHarnessesPageFromValue(value: unknown): AgentHarnessesPage;
export type SetAgentHarnessResult =
  | readonly [1n, import('@circular/protocol').SetAgentHarnessAccepted]
  | readonly [2n, { readonly code: bigint; readonly message: string; readonly hint?: string; readonly at?: unknown }];
/** Writes one harness binding into the daemon's settings document, or erases it (`program: null`),
 * and hands it to the standing pipeline: an agent that declares the name takes the new executor
 * between turns. The binding is not a declaration and is not recorded in the journal. Requires the
 * Operator role on an owner-local session. */
export declare function setAgentHarness(
  session: Pick<OwnerLocalSession, 'exchange'>,
  request: import('@circular/protocol').SetAgentHarnessRequest,
): Promise<SetAgentHarnessResult>;
/** `edge.depths` row: one edge into a standing actor, as that receiving actor measured its own inlet
 * when the daemon asked. `edge` is the edge identity carrier `actor.events` rows use (sending actor and
 * outlet, receiving actor and inlet, ordinal). `depth` counts inputs received on the edge and not yet
 * arrivals — held at the inlet (a declared delay, a closed inlet: pause, a busy effect actor) or waiting
 * to be recorded; it is the same measure an actor-health transition's `mailbox_depths` carries.
 * `queued` counts arrivals of the edge recorded into the actor's mailbox and not yet consumed.
 * `capacity` is the wire's declared capacity, or `null` when the wire declares none. */
export interface EdgeDepthItem {
  readonly edge: unknown;
  readonly depth: ObservationUInt;
  readonly queued: ObservationUInt;
  readonly capacity: ObservationUInt | null;
}
/** `edge.depths` row for an asked actor that ended before it answered: its name and the daemon's
 * `EndedBeforeAnswering` code. The answer is not silently short of that actor. */
export interface EdgeDepthEndedItem {
  readonly actor: { readonly local: string; readonly scope: unknown };
  readonly code: ObservationUInt;
}
/** `edge.depths` — a credit subscription with Null args. Opening asks every standing actor
 * once; each frame carries one row as an actor answers, so a fast actor's rows arrive while a slow one
 * is still in its turn. The stream ends with reason `Complete` once every asked actor answered or
 * ended; an actor that never answers yields no row and the stream stays open until closed. The
 * answer is the value now and is not recorded: a past cut shows depths only where an actor-health
 * transition recorded them. Opens through `openSubscription`. */
export declare const edgeDepths: typeof import('../../protocol/src/internal/record-values.js').recordRegistrations['edge.depths']['subscription'];
/** Reads one `edge.depths` frame payload: an edge row or an ended-actor row. */
export declare function edgeDepthsItemFromValue(value: unknown): EdgeDepthItem | EdgeDepthEndedItem;
export interface ApprovalDecisionReceipt {
  readonly item: ApprovalEffectKey;
  readonly outcome: readonly [1n, ApprovalEffectKey, ApprovalEffectKey] | 2n;
}
/** Existing TransitionResult arms; a refusal stays a result, transport failures throw. */
export type ApprovalDecisionResult = readonly [1n, ApprovalDecisionReceipt] | readonly [2n, {
  readonly code: bigint; readonly message: string; readonly hint?: string; readonly at?: unknown;
}];
export declare function decideApproval(session: Pick<OwnerLocalSession, 'exchange'>, request: {
  readonly item: ApprovalEffectKey;
  readonly decision: import('@circular/protocol').ApprovalDecision;
}): Promise<ApprovalDecisionResult>;
