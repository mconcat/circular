/** @packageDocumentation Existing Circular RPC values and transport-neutral protocol ports. */

import type * as ClosedTables from "./tables.generated.js";

declare const circularNominal: unique symbol;
declare const queryArguments: unique symbol;
declare const queryItems: unique symbol;
declare const queryAnchors: unique symbol;
declare const subscriptionArguments: unique symbol;
declare const subscriptionFrames: unique symbol;
declare const subscriptionAnchors: unique symbol;

type Nominal<Base, Name extends string> = Base & {
  readonly [circularNominal]: Name;
};

/** The two logical domains used to derive a child pipeline's projected boundary port. */
export type BoundaryPortDirection = "inlet" | "outlet";

/** The JavaScript carrier for one scope segment accepted by boundary-port derivation. */
export type BoundaryPortScopeSegment =
  | { readonly name: string }
  | { readonly of: string; readonly key: string | bigint | boolean };

/** The exact authored actor identity used as boundary-port derivation input. */
export interface BoundaryPortActorKey {
  readonly local: string;
  readonly scope: readonly BoundaryPortScopeSegment[];
}

/** Physical boundary-id codec version mirrored from the Rust protocol crate. */
export declare const BOUNDARY_PORT_ID_VERSION: 1;

/** Synthesized boundary-actor local codec version mirrored from the Rust protocol crate. */
export declare const SYNTH_BOUNDARY_LOCAL_VERSION: 1;

/** Derives the reserved physical port spelling expected by daemon boundary admission. */
export declare function deriveBoundaryPortId(
  direction: BoundaryPortDirection,
  actorKey: BoundaryPortActorKey,
  generation: bigint | number,
): string;

/** Derives the reserved local spelling for a synthesized boundary actor. */
export declare function deriveSynthBoundaryLocal(
  direction: BoundaryPortDirection,
  innerLocal: string,
  innerPort: PortId,
): string;

/** A non-empty readonly sequence. */
export type NonEmptyReadonlyArray<T> = readonly [T, ...T[]];

/** A finite unordered string-keyed object in Circular's first-order value domain. */
export interface CircularRecord {
  readonly [key: string]: CircularValue;
}

/** The closed first-order value domain carried by declarations, events, queries, and observations. */
export type CircularValue =
  | undefined
  | null
  | boolean
  | number
  | string
  | readonly CircularValue[]
  | CircularRecord;

/** Canonical codec bytes as lowercase hexadecimal for Map keys. Throws for unencodable values. */
export declare function valueKey(value: unknown): string;

/** Canonical byte equality. Absence (undefined) equals only absence; other unencodable values equal nothing. */
export declare function sameValue(a: unknown, b: unknown): boolean;

/** An opaque content digest; its algorithm, width, and external encoding are not fixed here. */
export type Digest = Nominal<string, "Digest">;

/** The content revision of a canonical executable program bundle. */
export type ProgramContentRevision = Nominal<string, "ProgramContentRevision">;

/** A stable authored actor identity, distinct from a runtime incarnation. */
export type ActorId = Nominal<string, "ActorId">;

/** A stable declared edge identity derived from both endpoints and its ordinal. */
export type EdgeId = Nominal<string, "EdgeId">;

/** A stable authored scope identity. */
export type ScopeId = Nominal<string, "ScopeId">;

/** An actor-local port identity supplied by an actor specification. */
export type PortId = Nominal<string, "PortId">;

/** A stable export-surface name in its project context. */
export type ExportName = Nominal<string, "ExportName">;

/** A stable annotation identity in its authored scope. */
export type AnnotationId = Nominal<string, "AnnotationId">;

/** A canonical top-level source binding used as visible authored actor identity. */
export type CanonicalBindingName = Nominal<string, "CanonicalBindingName">;

/** A client-chosen declaration commit identity used for durable deduplication. */
export type CommitId = Nominal<string, "CommitId">;

/** A server-issued identity for an open declaration epoch. */
export type EpochId = Nominal<string, "EpochId">;

/** A request identity unique only within the live-request set of one session. */
export type CorrelationId = Nominal<string, "CorrelationId">;

/** A digest of authored state and its authoring environment. */
export type AuthoringRevision = Nominal<string, "AuthoringRevision">;

/** A digest of executable topology and its revision-scoped authored-handle dictionary. */
export type TopologyRevision = Nominal<string, "TopologyRevision">;

/** A project-global monotonic occurrence cursor for accepted authoring commits. */
export type StructureCursor = Nominal<string, "StructureCursor">;

/** The pinned declaration schema, spec set, and reification contract tuple. */
export type AuthoringEnvironment = Nominal<string, "AuthoringEnvironment">;

/** A stable project identity. */
export type ProjectId = Nominal<Uint8Array, "ProjectId">;

/** A stable protocol-session identity used by records, not by envelope routing. */
export type SessionId = Nominal<string, "SessionId">;

/** Opaque 256-bit (32-byte) resumption token. No authority or textual projection. */
export type SessionToken = Nominal<Uint8Array, "SessionToken">;

/** A runtime actor incarnation identity, never interchangeable with an authored actor identity. */
export type Incarnation = Nominal<string, "Incarnation">;

/** A runtime generation component used by incarnation-scoped facts. */
export type Generation = Nominal<number, "Generation">;

/** A stable identity for one externally mediated effect. */
export type EffectId = Nominal<string, "EffectId">;

/** An opaque, optional hosted-actor lease token. */
export type LeaseToken = Nominal<string, "LeaseToken">;

/** A logical event stamp in one execution domain. */
export type Stamp = Nominal<string, "Stamp">;

/** A positive integer whose concrete upper bound is owned by the consuming contract. */
export type PositiveInteger = Nominal<number, "PositiveInteger">;

/** A non-negative integer whose concrete upper bound is owned by the consuming contract. */
export type NonNegativeInteger = Nominal<number, "NonNegativeInteger">;

/** A positive dimensionless replay-speed ratio. */
export type PositiveRatio = Nominal<number, "PositiveRatio">;

/** A closed current-revision value that distinguishes absence from a live digest. */
export type CurrentRevision<R> =
  | { readonly kind: "Absent" }
  | { readonly kind: "At"; readonly revision: R };

/** The current or expected authoring revision at an epoch boundary. */
export type CurrentAuthoringRevision = CurrentRevision<AuthoringRevision>;

/** The current topology revision, including the absence created by scope retirement. */
export type CurrentTopologyRevision = CurrentRevision<TopologyRevision>;

/** A complete consistency cut for a current authored-state reconstruction query. */
export interface AuthoringSnapshotAnchor {
  /** The authoring scope materialized by the query. */
  readonly scope: ScopeId;
  /** The authored-state revision used by mutation CAS and canonical reification. */
  readonly authoringRevision: CurrentAuthoringRevision;
  /** The executable topology revision at the same consistency cut. */
  readonly topologyRevision: CurrentTopologyRevision;
  /** The subscription handoff cursor at the same consistency cut. */
  readonly cursor: StructureCursor;
  /** The exact environment required to interpret every returned command. */
  readonly environment: AuthoringEnvironment;
}

/** A machine-readable diagnostic code allocated by one declared emission site. */
export type DiagnosticCode = Nominal<number, "DiagnosticCode">;

/** The closed reason taxonomy for normal protocol rejection. */
export type RejectionReason =
  | "Malformed"
  | "Unnegotiated"
  | "RoleInsufficient"
  | "TrustInsufficient"
  | "Unresolved"
  | "Invalid"
  | "Stale"
  | "Conflict"
  | "Exhausted"
  | "Unavailable";

/** A source span reported by the authoring host without exposing an engine stack. */
export interface SourceSpan {
  /** The canonical bundle source identifier. */
  readonly source: string;
  /** The inclusive one-based start line. */
  readonly startLine: number;
  /** The inclusive one-based start column. */
  readonly startColumn: number;
  /** The exclusive one-based end line. */
  readonly endLine: number;
  /** The exclusive one-based end column. */
  readonly endColumn: number;
}

/** One typed path component inside a decoded envelope. */
export type EnvelopePathSegment = string | number;

/** A non-empty structural path inside an envelope payload. */
export interface EnvelopePath {
  /** The path components from envelope root to the rejected field. */
  readonly segments: NonEmptyReadonlyArray<EnvelopePathSegment>;
}

/** A plan target that a diagnostic may safely reveal. */
export type PlanTarget =
  | { readonly kind: "Actor"; readonly actor: ActorId }
  | { readonly kind: "Edge"; readonly edge: EdgeId }
  | { readonly kind: "Export"; readonly exportName: ExportName }
  | { readonly kind: "Annotation"; readonly annotation: AnnotationId };

/** The closed diagnostic-location union; it excludes internal indexes and stack frames. */
export type DiagnosticSite =
  | { readonly kind: "SourceSpan"; readonly span: SourceSpan }
  | { readonly kind: "EnvelopePath"; readonly path: EnvelopePath }
  | { readonly kind: "PlanTarget"; readonly target: PlanTarget };

/** One deterministic, machine-addressable protocol diagnostic. */
export interface Diagnostic {
  /** The stable diagnostic code. */
  readonly code: DiagnosticCode;
  /** Human-readable explanatory text that is not used for machine decisions. */
  readonly message: string;
  /** Optional human guidance that is not used for machine decisions. */
  readonly hint: string | null;
  /** Optional public source, envelope, or plan location. */
  readonly at: DiagnosticSite | null;
}

/** A successfully accepted response value. */
export interface Accepted<T> {
  /** The success discriminant. */
  readonly status: "accepted";
  /** The partition-specific accepted value. */
  readonly value: T;
}

/** A normal rejected response value; it is never delivered through an exception-only channel. */
export interface Rejected {
  /** The rejection discriminant. */
  readonly status: "rejected";
  /** The closed reason that determines the caller's next action. */
  readonly reason: RejectionReason;
  /** Every diagnostic emitted at the first failing validation stage. */
  readonly diagnostics: NonEmptyReadonlyArray<Diagnostic>;
}

/** The normal return channel for a protocol operation. */
export type Result<T, R extends Rejected = Rejected> = Accepted<T> | R;

/** The nine protocol partitions. */
export type Partition =
  | "SessionMechanics"
  | "Declaration"
  | "Query"
  | "Subscription"
  | "EventInjection"
  | "LedgerTransition"
  | "ReplayControl"
  | "Experimental"
  | "Lifecycle";

/** The complete stable verb union, excluding dynamically negotiated experimental verbs. */
export type StableVerb =
  | "Hello"
  | "HelloAck"
  | "Goodbye"
  | "BeginEpoch"
  | "ValidateEpoch"
  | "CommitEpoch"
  | "AbortEpoch"
  | "UpsertActor"
  | "RetireActor"
  | "UpsertEdge"
  | "RetireEdge"
  | "UpsertScope"
  | "RetireScope"
  | "MoveToScope"
  | "UpsertExportMount"
  | "RetireExportMount"
  | "UpsertAnnotation"
  | "RetireAnnotation"
  | "SetPresentation"
  | "SetFlags"
  | "UpsertTemplate"
  | "RetireTemplate"
  | "CommandResult"
  | "Query"
  | "QueryResult"
  | "QueryClose"
  | "Subscribe"
  | "SubscribeAck"
  | "Credit"
  | "Unsubscribe"
  | "Frame"
  | "SubscriptionEnded"
  | "Inject"
  | "InjectAck"
  | "ApprovalDecide"
  | "SetObservationControl"
  | "TransitionResult"
  | "SetAgentHarness"
  | "ReplayStart"
  | "ReplayRewind"
  | "ReplayEnd"
  | "ReplayResult"
  | "Resume"
  | "Pause"
  | "LifecycleResult";

/** The closed verb subset owned by each stable partition. */
export type PartitionVerb<P extends Partition> =
  P extends "SessionMechanics" ? "Hello" | "HelloAck" | "Goodbye"
    : P extends "Declaration" ?
        | "BeginEpoch" | "ValidateEpoch" | "CommitEpoch" | "AbortEpoch"
        | "UpsertActor" | "RetireActor" | "UpsertEdge" | "RetireEdge"
        | "UpsertScope" | "RetireScope" | "MoveToScope" | "UpsertExportMount" | "RetireExportMount"
        | "UpsertAnnotation" | "RetireAnnotation" | "SetPresentation" | "SetFlags"
        | "UpsertTemplate" | "RetireTemplate" | "CommandResult"
      : P extends "Query" ? "Query" | "QueryResult" | "QueryClose"
        : P extends "Subscription" ?
            "Subscribe" | "SubscribeAck" | "Credit" | "Unsubscribe" | "Frame" | "SubscriptionEnded"
          : P extends "EventInjection" ? "Inject" | "InjectAck"
            : P extends "LedgerTransition" ? "ApprovalDecide" | "SetObservationControl" | "TransitionResult" | "SetAgentHarness"
              : P extends "ReplayControl" ? "ReplayStart" | "ReplayRewind" | "ReplayEnd" | "ReplayResult"
                : P extends "Lifecycle" ? "Resume" | "Pause" | "LifecycleResult"
                  : string;

/** A transport-independent envelope with no embedded session, time, or sender field. */
export interface Envelope<
  P extends Partition = Partition,
  V extends PartitionVerb<P> = PartitionVerb<P>,
  Payload = unknown,
> {
  /** The partition and verb pair that determines the payload schema. */
  readonly kind: { readonly partition: P; readonly verb: V };
  /** The originating request's session-local correlation identity. */
  readonly correlation: CorrelationId;
  /** The verb-specific first-order payload. */
  readonly payload: Payload;
}

/** The trust level established from the transport channel at session creation. */
export type TransportTrust = "Remote" | "LocalUser" | "LocalOwner";

/** A protocol major version whose exact numeric representation is a wire-format decision. */
export type ProtocolVersion = Nominal<number, "ProtocolVersion">;

/** A negotiated partition minor version. */
export type ProtocolMinor = Nominal<number, "ProtocolMinor">;

/** The per-partition feature proposal or negotiated meet. */
export type FeatureSet = ReadonlyMap<Partition, ProtocolMinor>;

/** The closed set of roles that may be requested and then independently established. */
export type SessionRole =
  | { readonly kind: "Reader" }
  | { readonly kind: "Writer"; readonly scope: ScopeId }
  | { readonly kind: "Operator" };

export interface Hello {
  /** The non-negotiated protocol major version. */
  readonly protocolVersion: ProtocolVersion;
  /** The caller's per-partition feature proposal. */
  readonly features: FeatureSet;
  /** Roles requested but not yet established. */
  readonly requestedRoles: ReadonlySet<SessionRole>;
}

/** The accepted half of a session-establishment response. */
export interface Established {
  /** The agreed protocol major version. */
  readonly protocolVersion: ProtocolVersion;
  /** The negotiated per-partition feature meet. */
  readonly features: FeatureSet;
  /** Roles actually established by the engine. */
  readonly roles: ReadonlySet<SessionRole>;
  /** Trust derived from the selected transport channel. */
  readonly trust: TransportTrust;
  /** A new authority-neutral session token. */
  readonly token: SessionToken;
}

/** An explicit graceful session-close payload with no hidden cancellation semantics. */
export interface Goodbye {
  /** The session-mechanics verb discriminant. */
  readonly kind: "Goodbye";
}

/** The canonical local source key of a child authored scope. */
export type ScopeName = Nominal<string, "ScopeName">;

export type InstanceScalarKey =
  | { readonly kind: "Text"; readonly value: string }
  | { readonly kind: "Int"; readonly value: bigint }
  | { readonly kind: "Bool"; readonly value: boolean };

/**
 * One segment of a scope identity — a closed sum, not a name.
 *
 * The wire carries it as `[1, name]` or `[2, of, key]`. This was a plain nominal
 * string, which could not express the instance arm at all; the wire codec on this side already
 * carried both arms, so the declaration surface was the last place that disagreed.
 */
export type ScopeSegment =
  | { readonly name: ScopeName }
  | { readonly of: ScopeName; readonly key: InstanceScalarKey | readonly InstanceScalarKey[] };

/**
 * Whether a declared scope stands up when it is declared or waits to be instantiated.
 *
 * `Template` is a prototype: declaring it stands nothing up, and a `ScopeSegment` instance arm
 * naming it is what does.
 */
export type ScopeRole = "Concrete" | "Template";

/** One boundary binding — an outer port name and the inner actor port it reaches. */
export interface ScopeBinding<D extends DeclarationAddressDomain> {
  /** The actor and port inside the scope. */
  readonly inner: PortEndpoint<D>;
  /** The name the container side calls this binding. */
  readonly outer: PortId;
}

/**
 * The scope's boundary ports — the only path between inside and outside.
 *
 * Each direction is keyed by the **outer** port name, so canonical order is that name's raw UTF-8
 * byte order and a repeated outer name is unrepresentable rather than diagnosable. It is not the
 * encoded order of the whole binding record: a length prefix compares first, which would put
 * `tick` before `event` on the wire and `event` before `tick` after any receiver rebuilt the map
 * — a round trip that does not close.
 */
export interface ScopeBoundary<D extends DeclarationAddressDomain> {
  /** Arrivals from the container, by outer name. */
  readonly inlets: readonly ScopeBinding<D>[];
  /** Emissions to the container, by outer name. */
  readonly outlets: readonly ScopeBinding<D>[];
}

/**
 * What `UpsertScope` carries.
 *
 * The scope's contents are not here. Actors, edges and child scopes are ordinary declarations
 * whose scope address happens to sit under this one.
 */
export interface ScopeDeclaration<D extends DeclarationAddressDomain> {
  /** How the contents become real. */
  readonly role: ScopeRole;
  /** Where inside meets outside. */
  readonly boundary: ScopeBoundary<D>;
}

/** A daemon catalog actor type name. */
export type ActorTypeId = Nominal<string, "ActorTypeId">;

/** The three complete runtime switches stored on each authored actor. */
export interface ActorFlags {
  /** Whether actor computation is bypassed. */
  readonly bypass: boolean;
  /** Whether router acceptance of actor emissions is muted. */
  readonly mute: boolean;
  /** Whether the actor stops consuming inputs. */
  readonly pause: boolean;
}

/** The full normalized semantic declaration carried by an upserted actor. */
export interface ActorDeclaration {
  /** The daemon catalog actor type name. */
  readonly actorType: ActorTypeId;
  /** The normalized first-order configuration value. */
  readonly config: CircularValue;
  /** The complete resolved live flag set. */
  readonly flags: ActorFlags;
}

/** The two shedding choices available to best-effort edge delivery. */
export type ShedPolicy = "DropNewest" | "DropOldest";

/** The closed edge-delivery policy union. */
export type DeliveryPolicy =
  | { readonly mode: "BestEffort"; readonly onFull: ShedPolicy }
  | "Lossless"
  | "Durable";

/** Full delivery and capacity policy for a declared edge. */
export interface WirePolicy {
  /** The delivery behavior used when accepted work reaches capacity. */
  readonly delivery: DeliveryPolicy;
  /** Optional positive outstanding-delivery capacity. */
  readonly capacity?: bigint;
}

/** Declared delay in rational seconds; reduction is checked by daemon admission. */
export interface DeclaredDelay {
  readonly num: bigint;
  readonly den: bigint;
}

/** The full mutable attributes of a declared edge, excluding identity. */
export interface PreprocessStep {
  /** The closed table `PreprocessKind` from `@circular/protocol/tables`. */
  readonly kind: (typeof ClosedTables.PreprocessKind)[number];
  readonly config: CircularValue;
}

export interface EdgeAttributes {
  readonly preprocess?: readonly PreprocessStep[];
  /** Declared physical delay in rational seconds, including zero. */
  readonly delay: DeclaredDelay;
  /** Delivery and capacity behavior copied at router acceptance. */
  readonly policy: WirePolicy;
}

/** The six address carriers varied by declaration context. */
export interface DeclarationAddressDomain {
  /** The actor-address carrier. */
  readonly actor: unknown;
  /** The edge-address carrier. */
  readonly edge: unknown;
  /** The scope-address carrier. */
  readonly scope: unknown;
  /** The export-mount-address carrier. */
  readonly exportMount: unknown;
  /** The annotation-address carrier. */
  readonly annotation: unknown;
  /** The presentation-owner-address carrier. */
  readonly presentationOwner: unknown;
}

/** An epoch-local forward reference to an actor that may not yet have been declared. */
export type EpochActorReference = Nominal<string, "EpochActorReference">;

/** An epoch-local forward reference to a declared edge. */
export type EpochEdgeReference = Nominal<string, "EpochEdgeReference">;

/** An epoch-local forward reference to an authored scope. */
export type EpochScopeReference = Nominal<string, "EpochScopeReference">;

/** An epoch-local forward reference to an export mount. */
export type EpochExportReference = Nominal<string, "EpochExportReference">;

/** An epoch-local forward reference to an annotation. */
export type EpochAnnotationReference = Nominal<string, "EpochAnnotationReference">;

/** An absolute export-mount identity that includes its authored scope. */
export type AbsoluteExportAddress = Nominal<string, "AbsoluteExportAddress">;

/** An absolute annotation identity that includes its authored scope. */
export type AbsoluteAnnotationAddress = Nominal<string, "AbsoluteAnnotationAddress">;

/** A scope-relative actor address rooted at the snapshot's synthetic root. */
export type RelativeActorAddress = Nominal<string, "RelativeActorAddress">;

/** A scope-relative edge address rooted at the snapshot's synthetic root. */
export type RelativeEdgeAddress = Nominal<string, "RelativeEdgeAddress">;

/** A scope-relative scope address rooted at the snapshot's synthetic root. */
export type RelativeScopeAddress = Nominal<string, "RelativeScopeAddress">;

/** A scope-relative export-mount address rooted at the snapshot's synthetic root. */
export type RelativeExportAddress = Nominal<string, "RelativeExportAddress">;

/** A scope-relative annotation address rooted at the snapshot's synthetic root. */
export type RelativeAnnotationAddress = Nominal<string, "RelativeAnnotationAddress">;

export type PresentationOwner<A, N> = { readonly actor: A; readonly annotation?: never } | { readonly annotation: N; readonly actor?: never };

/** Live mutation addresses, permitting stable identities and epoch-local forward references. */
export interface EpochAddressDomain extends DeclarationAddressDomain {
  readonly actor: ActorId | EpochActorReference;
  readonly edge: EdgeId | EpochEdgeReference;
  readonly scope: ScopeId | EpochScopeReference;
  readonly exportMount: AbsoluteExportAddress | EpochExportReference;
  readonly annotation: AbsoluteAnnotationAddress | EpochAnnotationReference;
  readonly presentationOwner: PresentationOwner<ActorId | EpochActorReference, AbsoluteAnnotationAddress | EpochAnnotationReference>;
}

/** Accepted-history addresses with only stable absolute identities. */
export interface AbsoluteAddressDomain extends DeclarationAddressDomain {
  readonly actor: ActorId;
  readonly edge: EdgeId;
  readonly scope: ScopeId;
  readonly exportMount: AbsoluteExportAddress;
  readonly annotation: AbsoluteAnnotationAddress;
  readonly presentationOwner: PresentationOwner<ActorId, AbsoluteAnnotationAddress>;
}

/** Snapshot addresses relative to one queried authoring scope. */
export interface RelativeAddressDomain extends DeclarationAddressDomain {
  readonly actor: RelativeActorAddress;
  readonly edge: RelativeEdgeAddress;
  readonly scope: RelativeScopeAddress;
  readonly exportMount: RelativeExportAddress;
  readonly annotation: RelativeAnnotationAddress;
  readonly presentationOwner: PresentationOwner<RelativeActorAddress, RelativeAnnotationAddress>;
}

/** A typed actor port endpoint in one declaration address domain. */
export interface PortEndpoint<D extends DeclarationAddressDomain> {
  /** The domain-bound actor address. */
  readonly actor: D["actor"];
  /** The specification-owned port identity. */
  readonly port: PortId;
}

/** A full declared edge value whose key is derived from both endpoints and ordinal. */
export interface EdgeDeclaration<D extends DeclarationAddressDomain> {
  /** The source endpoint. */
  readonly from: PortEndpoint<D>;
  /** The target endpoint. */
  readonly to: PortEndpoint<D>;
  /** The explicit identity component for parallel edges. */
  readonly ordinal: NonNegativeInteger;
  /** The full normalized edge attributes, under the wire key `attrs`. */
  readonly attrs: EdgeAttributes;
}

/** A write-capable child input boundary reference used only by an export request role. */
export interface BoundaryPortReference<D extends DeclarationAddressDomain> {
  /** The child boundary actor. */
  readonly actor: D["actor"];
  /** The exact child input's internal outlet. */
  readonly port: PortId;
}

/** A read-only observed port reference used by non-request export roles. */
export interface ObservedPortReference<D extends DeclarationAddressDomain> {
  /** The observed authored actor. */
  readonly actor: D["actor"];
  /** The observed output port. */
  readonly port: PortId;
}

/** An opaque registered operation declaration attached to one export surface. */
export type OperationDeclaration = Nominal<string, "OperationDeclaration">;

/** The fixed partial role map of one export surface. */
export interface ExportRoleBindings<D extends DeclarationAddressDomain> {
  /** Optional writable child input boundary. */
  readonly request: BoundaryPortReference<D> | null;
  /** Optional read-only progress output. */
  readonly progress: ObservedPortReference<D> | null;
  /** Optional read-only result output. */
  readonly result: ObservedPortReference<D> | null;
  /** Optional read-only error output. */
  readonly error: ObservedPortReference<D> | null;
}

/** The full export value replaced by one export-mount upsert. */
export interface ExportMountDeclaration<D extends DeclarationAddressDomain> {
  /** The closed fixed-role bindings. */
  readonly roles: ExportRoleBindings<D>;
  /** Optional operations declared for the surface as a whole. */
  readonly operations: OperationDeclaration | null;
  readonly surface?: CircularValue;
}

export type LayoutCoordinate = Nominal<number, "LayoutCoordinate">;

/** An absolute authored point in a parent scope's coordinate system, in layout units. */
export interface Point {
  /** Horizontal coordinate, positive to the right. */
  readonly x: LayoutCoordinate;
  /** Vertical coordinate, positive downward. */
  readonly y: LayoutCoordinate;
}

/** A non-negative authored size in logical layout units. */
export interface Size {
  /** Width in logical layout units. */
  readonly width: LayoutCoordinate;
  /** Height in logical layout units. */
  readonly height: LayoutCoordinate;
}

/** An authored board-grid position and extent. */
export interface BoardPlacement {
  /** Zero-based board column. */
  readonly column: NonNegativeInteger;
  /** Zero-based board row. */
  readonly row: NonNegativeInteger;
  /** Positive column span. */
  readonly width: PositiveInteger;
  /** Positive row span. */
  readonly height: PositiveInteger;
}

/** A canonical presentation group name. */
export type GroupName = Nominal<string, "GroupName">;

/** A registered static view selection name. */
export type ViewKind = Nominal<string, "ViewKind">;

/** A relative horizontal or vertical placement relationship. */
export type RelativeRelation = "RightOf" | "LeftOf" | "Above" | "Below";

/** A same-axis alignment relationship. */
export type AlignmentAxis = "Top" | "Bottom" | "Left" | "Right";

/** The closed authored anchor union consumed by layout. */
export type PresentationAnchor<D extends DeclarationAddressDomain> =
  | { readonly kind: "Flow" }
  | { readonly kind: "Relative"; readonly target: D["actor"]; readonly relation: RelativeRelation }
  | { readonly kind: "Align"; readonly target: D["actor"]; readonly axis: AlignmentAxis };

/** A full normalized actor presentation value; null denotes explicit field absence. */
export interface Presentation<D extends DeclarationAddressDomain> {
  /** Optional user-facing label. */
  readonly label: string | null;
  /** Optional adjacency group. */
  readonly group: GroupName | null;
  /** Optional relative or flow anchor. */
  readonly anchor: PresentationAnchor<D> | null;
  /** Optional fixed authored point. */
  readonly fixed: Point | null;
  /** Optional explicit authored size. */
  readonly size: Size | null;
  /** Optional explicit board placement. */
  readonly board: BoardPlacement | null;
  /** Optional registered view selection. */
  readonly view: { readonly kind: string; readonly config: unknown } | null;
  /** Whether a container is authored as collapsed. */
  readonly collapsed: boolean;
}

/** The two annotation kinds outside executable graph semantics. */
export type AnnotationKind = "Note" | "Backdrop";

/** Authored annotation placement in logical layout units. */
export interface AnnotationPlacement {
  /** The annotation's top-left point. */
  readonly origin: Point;
  /** The annotation's explicit size. */
  readonly size: Size;
}

/** A full annotation value replaced by one annotation upsert. */
export interface AnnotationDeclaration<D extends DeclarationAddressDomain> {
  /** The closed annotation kind. */
  readonly kind: AnnotationKind;
  /** Authored actor references used by the annotation. */
  readonly references: ReadonlySet<D["actor"]>;
  /** The annotation's own placement intent. */
  readonly placement: AnnotationPlacement;
  /** Static annotation text. */
  readonly body: string;
}

/** Opens a declaration candidate against an explicit revision and environment. */
export interface BeginEpochCommand<D extends DeclarationAddressDomain = EpochAddressDomain> {
  /** The declaration tag. */
  readonly kind: "BeginEpoch";
  /** The target authored scope. */
  readonly scope: D["scope"];
  /** The client-chosen durable deduplication identity. */
  readonly commitId: CommitId;
  /** The mandatory authoring CAS baseline. */
  readonly expectedRevision: CurrentAuthoringRevision;
  /** The mandatory environment CAS baseline. */
  readonly expectedEnvironment: AuthoringEnvironment;
}

/** Requests validation without closing an open declaration epoch. */
export interface ValidateEpochCommand {
  /** The declaration tag. */
  readonly kind: "ValidateEpoch";
  /** The open epoch to validate. */
  readonly epoch: EpochId;
}

/** Requests atomic commit of an open declaration epoch. */
export interface CommitEpochCommand {
  /** The declaration tag. */
  readonly kind: "CommitEpoch";
  /** The open epoch to commit. */
  readonly epoch: EpochId;
}

/** Discards an open candidate without changing live authored state. */
export interface AbortEpochCommand {
  /** The declaration tag. */
  readonly kind: "AbortEpoch";
  /** The open epoch to discard. */
  readonly epoch: EpochId;
}

/** Replaces one actor key with a complete normalized declaration. */
export interface UpsertActorCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "UpsertActor";
  /** The domain-bound actor key. */
  readonly actor: D["actor"];
  /** The complete normalized actor value. */
  readonly declaration: ActorDeclaration;
}

/** Explicitly removes one authored actor. */
export interface RetireActorCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "RetireActor";
  /** The domain-bound actor key to retire. */
  readonly actor: D["actor"];
}

/** Replaces one declared edge key with a complete normalized value. */
export interface UpsertEdgeCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "UpsertEdge";
  /** The domain-bound edge key. */
  readonly edge: D["edge"];
  /** The complete normalized edge value. */
  readonly declaration: EdgeDeclaration<D>;
}

/** Explicitly disconnects and retires one declared edge. */
export interface RetireEdgeCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "RetireEdge";
  /** The domain-bound edge key to retire. */
  readonly edge: D["edge"];
}

/**
 * Replaces one scope with its complete declared value — the same grammar as `UpsertActor`.
 *
 * **Instantiation is this command.** An address ending in an instance segment is this command's
 * ordinary domain rather than something it refuses, which is what lets stage one of
 * self-modification add no mutation machinery of its own. The declaration such a command carries
 * is empty: the prototype owns the boundary, and repeating it per cell would put one fact in two
 * places.
 */
export interface UpsertScopeCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "UpsertScope";
  /** The scope's own address. */
  readonly scope: D["scope"];
  /** The complete normalized scope value. */
  readonly declaration: ScopeDeclaration<D>;
}

/**
 * Retires one scope and its subtree.
 *
 * It carries no declaration — retirement names its target, and the value that stood there
 * already says what was removed.
 */
export interface RetireScopeCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "RetireScope";
  /** The scope address to retire. */
  readonly scope: D["scope"];
}

/** Moves a sequence of existing authored actors into an existing scope without retiring them. */
export interface MoveToScopeCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "MoveToScope";
  /** The domain-bound actor addresses to move; an empty list is structurally well formed. */
  readonly actors: readonly D["actor"][];
  /** The existing destination scope. */
  readonly target: D["scope"];
}

/** Replaces one export mount with its full role, operation and surface declaration. */
export interface UpsertExportMountCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "UpsertExportMount";
  /** The domain-bound export mount key. */
  readonly mount: D["exportMount"];
  /** The complete export declaration. */
  readonly declaration: ExportMountDeclaration<D>;
}

/** Explicitly retires one export mount. */
export interface RetireExportMountCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "RetireExportMount";
  /** The domain-bound export mount key to retire. */
  readonly mount: D["exportMount"];
}

/** Replaces one annotation with a complete static annotation value. */
export interface UpsertAnnotationCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "UpsertAnnotation";
  /** The domain-bound annotation key. */
  readonly annotation: D["annotation"];
  /** The complete annotation value. */
  readonly declaration: AnnotationDeclaration<D>;
}

/** Explicitly retires one annotation. */
export interface RetireAnnotationCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "RetireAnnotation";
  /** The domain-bound annotation key to retire. */
  readonly annotation: D["annotation"];
}

/** Replaces an actor's complete normalized presentation value. */
export interface SetPresentationCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "SetPresentation";
  /** The domain-bound presentation owner. */
  readonly owner: D["presentationOwner"];
  /** The complete normalized presentation value. */
  readonly presentation: Presentation<D>;
}

/** Sets the complete closed live-flag value for one actor. */
export interface SetFlagsCommand<D extends DeclarationAddressDomain> {
  /** The declaration tag. */
  readonly kind: "SetFlags";
  /** The domain-bound actor key. */
  readonly actor: D["actor"];
  /** The complete closed flag set. */
  readonly flags: ActorFlags;
}

export interface UpsertTemplateCommand {
  readonly kind: "UpsertTemplate";
  readonly name: string;
  readonly commands: readonly (ReconstructiveDeclarationCommand<RelativeAddressDomain> | UpsertTemplateCommand)[];
}
export interface RetireTemplateCommand {
  readonly kind: "RetireTemplate";
  readonly name: string;
}

/** The shared declaration command union used by live mutation, accepted history, and reconstruction. */
export type DeclarationCommand<D extends DeclarationAddressDomain> =
  | BeginEpochCommand<D>
  | ValidateEpochCommand
  | CommitEpochCommand
  | AbortEpochCommand
  | UpsertActorCommand<D>
  | RetireActorCommand<D>
  | UpsertEdgeCommand<D>
  | RetireEdgeCommand<D>
  | UpsertScopeCommand<D>
  | RetireScopeCommand<D>
  | MoveToScopeCommand<D>
  | UpsertExportMountCommand<D>
  | RetireExportMountCommand<D>
  | UpsertAnnotationCommand<D>
  | RetireAnnotationCommand<D>
  | SetPresentationCommand<D>
  | SetFlagsCommand<D>
  | UpsertTemplateCommand
  | RetireTemplateCommand;

/** The content-command subset admitted between epoch brackets. */
export type DeclarationContentCommand<D extends DeclarationAddressDomain> = Exclude<
  DeclarationCommand<D>,
  BeginEpochCommand<D> | ValidateEpochCommand | CommitEpochCommand | AbortEpochCommand
>;

/** The exact reconstructive subset emitted by an authored-state snapshot. */
export type ReconstructiveDeclarationCommand<D extends DeclarationAddressDomain> = Extract<
  DeclarationCommand<D>,
  | UpsertScopeCommand<D>
  | UpsertActorCommand<D>
  | UpsertEdgeCommand<D>
  | UpsertExportMountCommand<D>
  | UpsertAnnotationCommand<D>
  | SetPresentationCommand<D>
  | UpsertTemplateCommand
>;

/** A compacted current-state reconstruction made only from existing declaration commands. */
export type CompactedDeclarationCommandList = readonly ReconstructiveDeclarationCommand<RelativeAddressDomain>[];

/** One live content request associated with an already open epoch. */
export interface EpochContentRequest {
  /** The open epoch receiving the content command. */
  readonly epoch: EpochId;
  /** The existing declaration content command. */
  readonly command: DeclarationContentCommand<EpochAddressDomain>;
}

/** A typed grouping of accepted RPC brackets and their ordered command list. */
export interface RpcEpoch<C> {
  /** The accepted opening bracket. */
  readonly begin: BeginEpochCommand<AbsoluteAddressDomain>;
  /** Ordered accepted semantic content. */
  readonly content: readonly C[];
  /** The accepted terminal commit bracket. */
  readonly terminal: CommitEpochCommand;
}

/** A before/after revision pair for one affected authored scope. */
export interface RevisionTransition {
  /** Authoring revision before the accepted commit. */
  readonly authoringBefore: CurrentAuthoringRevision;
  /** Authoring revision after the accepted commit. */
  readonly authoringAfter: CurrentAuthoringRevision;
  /** Topology revision before the accepted commit. */
  readonly topologyBefore: CurrentTopologyRevision;
  /** Topology revision after the accepted commit. */
  readonly topologyAfter: CurrentTopologyRevision;
}

/** Durable metadata atomically published with one accepted authoring commit. */
export interface AcceptedCommitMetadata {
  /** The project-global occurrence cursor assigned to the commit. */
  readonly cursor: StructureCursor;
  /** The scope originally targeted by the epoch. */
  readonly targetScope: ScopeId;
  /** Complete transitions for every affected scope and authored ancestor. */
  readonly revisions: ReadonlyMap<ScopeId, RevisionTransition>;
  /** Authoring environment before the commit. */
  readonly beforeEnvironment: AuthoringEnvironment;
  /** Authoring environment after the commit. */
  readonly afterEnvironment: AuthoringEnvironment;
}

/** The accepted result of opening a declaration epoch. */
export interface EpochOpened {
  /** The new open epoch identity. */
  readonly epoch: EpochId;
}

/** The accepted result of applying one content command to an isolated candidate. */
export interface CandidateApplied {
  /** The open epoch whose candidate changed. */
  readonly epoch: EpochId;
}

/** The accepted result of non-terminal epoch validation. */
export interface EpochValidated {
  /** The still-open epoch that was validated. */
  readonly epoch: EpochId;
  /** A conservative upper bound on actors that commit may restart. */
  readonly restartUpperBound: ReadonlySet<ActorId>;
}

/** Runtime effects observed after an atomic accepted authoring commit. */
export interface CommitRuntimeEffects {
  /** Actors actually restarted by the committed diff. */
  readonly restartedActors: ReadonlySet<ActorId>;
  /** Authored scopes activated by the committed diff. */
  readonly activatedScopes: ReadonlySet<ScopeId>;
  /** Authored scopes terminated by the committed diff. */
  readonly terminatedScopes: ReadonlySet<ScopeId>;
}

/** The durable accepted result of one declaration commit. */
export interface EpochCommitted {
  /** Metadata published in the same atomic durability boundary. */
  readonly metadata: AcceptedCommitMetadata;
  /** Runtime effects calculated from the accepted semantic diff. */
  readonly runtimeEffects: CommitRuntimeEffects;
}

/** The accepted result of explicitly aborting an open declaration epoch. */
export interface EpochAborted {
  /** The terminally discarded epoch. */
  readonly epoch: EpochId;
}

/** The accepted response values of declaration-partition requests. */
export type DeclarationAccepted = EpochOpened | CandidateApplied | EpochValidated | EpochCommitted | EpochAborted;

/** A flat registered query name; the client package does not own its enumeration. */
export type QueryName = Nominal<string, "QueryName">;

/** Whether a registered query is one-shot or cursor-paged. */
export type QueryPaging = "none" | "cursor";

/** The consistency-anchor family registered for one query. */
export type AnchorKind = "Topology" | "AuthoringSnapshot" | "Records";

/** A feature-owned typed descriptor for one registered query. */
export interface QueryDescriptor<Args, Item, Anchor, Paging extends QueryPaging = "cursor"> {
  /** The registered flat wire name. */
  readonly name: QueryName;
  /** The registered paging policy. */
  readonly paging: Paging;
  /** The registered consistency-anchor family. */
  readonly anchorKind: AnchorKind;
  /** Compile-time argument witness, absent from wire values. */
  readonly [queryArguments]?: Args;
  /** Compile-time item witness, absent from wire values. */
  readonly [queryItems]?: Item;
  /** Compile-time anchor witness, absent from wire values. */
  readonly [queryAnchors]?: Anchor;
}

/** An opaque registration-specific cursor position. */
export type CursorPosition = Nominal<Uint8Array, "CursorPosition">;

/** An opaque registration and schema-specific cursor domain. */
export type CursorDomain = Nominal<string, "CursorDomain">;

/** A cursor that carries both its immutable anchor and its opaque position. */
export interface QueryCursor<Anchor> {
  /** The consistency anchor this cursor cannot cross. */
  readonly anchor: Anchor;
  /** The registration-specific cursor domain. */
  readonly domain: CursorDomain;
  /** Opaque position bytes represented by the negotiated codec. */
  readonly position: CursorPosition;
}

/** A positive page bound and optional continuation cursor. */
export interface PageRequest<Anchor> {
  /** Maximum item count requested for this page. */
  readonly limit: PositiveInteger;
  /** A cursor returned by the immediately preceding page, or null for the first page. */
  readonly cursor: QueryCursor<Anchor> | null;
}

/** A page that requires another request at the included cursor. */
export interface MoreQueryPage<Item, Anchor> {
  readonly cut?: readonly unknown[];
  /** Empty Cut when an unknown or excessive since required a full fold. */
  readonly folded_from?: readonly unknown[];
  readonly reached?: {
    readonly revision_epoch: import("@circular/protocol/actor-query").CircularUInt;
    readonly cut: readonly unknown[];
  };
  /** The page's immutable consistency anchor. */
  readonly anchor: Anchor;
  /** Ordered items in this finite page. */
  readonly items: readonly Item[];
  /** The explicit non-terminal marker. */
  readonly terminal: "More";
  /** The only cursor valid for the next page request. */
  readonly next: QueryCursor<Anchor>;
}

/** The final successful page in a finite query response. */
export interface CompleteQueryPage<Item, Anchor> {
  readonly cut?: readonly unknown[];
  /** Empty Cut when an unknown or excessive since required a full fold. */
  readonly folded_from?: readonly unknown[];
  readonly reached?: {
    readonly revision_epoch: import("@circular/protocol/actor-query").CircularUInt;
    readonly cut: readonly unknown[];
  };
  /** The page's immutable consistency anchor. */
  readonly anchor: Anchor;
  /** Ordered items in this final page. */
  readonly items: readonly Item[];
  /** The explicit normal terminal marker. */
  readonly terminal: "Complete";
}

/** A diagnostic terminal page that preserves preceding partial items. */
export interface DiagnosticQueryPage<Item, Anchor> {
  readonly cut?: readonly unknown[];
  /** Empty Cut when an unknown or excessive since required a full fold. */
  readonly folded_from?: readonly unknown[];
  readonly reached?: {
    readonly revision_epoch: import("@circular/protocol/actor-query").CircularUInt;
    readonly cut: readonly unknown[];
  };
  /** The invalidated or failed consistency anchor. */
  readonly anchor: Anchor;
  /** Ordered items accepted before diagnostic termination. */
  readonly items: readonly Item[];
  /** The explicit diagnostic terminal marker. */
  readonly terminal: "Diagnostic";
  /** The diagnostic that ended this finite stream. */
  readonly diagnostic: Diagnostic;
}

/** The closed page-response union. */
export type QueryPage<Item, Anchor> =
  | MoreQueryPage<Item, Anchor>
  | CompleteQueryPage<Item, Anchor>
  | DiagnosticQueryPage<Item, Anchor>;

/** A wire query request whose descriptor owner determines argument and item codecs. */
export interface QueryRequest<Args, Anchor> {
  readonly since?: readonly unknown[];
  /**
   * The correlation key of the replay lens this read goes through (a lens handle's
   * `correlation`). Absent, the read is live. Not carried together with an `upto`.
   */
  readonly lens?: bigint;
  /** The registered flat query name. */
  readonly name: QueryName;
  /** Registered first-order query arguments. */
  readonly args: Args;
  /** Paging state, or null for a non-paged registration. */
  readonly page: PageRequest<Anchor> | null;
}

/** A flat registered subscription target name. */
export type TargetName = Nominal<string, "TargetName">;

/** The three server-selected subscription delivery disciplines. */
export type DeliveryDiscipline = "lossless" | "conflated" | "credit";

/** A feature-owned typed descriptor for one registered subscription target. */
export interface SubscriptionDescriptor<
  Args,
  Frame,
  Anchor,
  Discipline extends DeliveryDiscipline,
> {
  /** The registered flat target name. */
  readonly name: TargetName;
  /** The server-owned delivery discipline. */
  readonly discipline: Discipline;
  /** Compile-time argument witness, absent from wire values. */
  readonly [subscriptionArguments]?: Args;
  /** Compile-time frame witness, absent from wire values. */
  readonly [subscriptionFrames]?: Frame;
  /** Compile-time anchor witness, absent from wire values. */
  readonly [subscriptionAnchors]?: Anchor;
}

/** A registered subscription target and its first-order arguments. */
export interface SubscriptionTarget<Args> {
  /** The registered flat target name. */
  readonly name: TargetName;
  /** Arguments whose shape is owned by the registration. */
  readonly args: Args;
}

/** A successful subscription opening, including server-owned policy values. */
export interface SubscriptionOpened<Anchor, Discipline extends DeliveryDiscipline> {
  /** The correlation identity that owns this subscription until termination. */
  readonly correlation: CorrelationId;
  /** The delivery discipline selected by target registration. */
  readonly discipline: Discipline;
  /** The fixed retention depth selected by target registration. */
  readonly retentionDepth: NonNegativeInteger;
  /** The catch-up/live consistency anchor. */
  readonly anchor: Anchor;
}

/** A lossless subscription data frame. */
export interface LosslessFrame<Payload, Anchor> {
  /** The data-frame discriminant. */
  readonly kind: "Data";
  /** The registration-owned payload. */
  readonly payload: Payload;
  /** The frame's consistency anchor. */
  readonly anchor: Anchor;
}

/** An opaque conflation slot key selected by target registration. */
export type SlotKey = Nominal<string, "SlotKey">;

/** A conflated subscription data frame with explicit loss accounting. */
export interface ConflatedFrame<Payload, Anchor> {
  /** The data-frame discriminant. */
  readonly kind: "Data";
  /** The registration-owned payload. */
  readonly payload: Payload;
  /** The frame's consistency anchor. */
  readonly anchor: Anchor;
  /** The registration-selected slot, or null when the target has one implicit slot. */
  readonly slot: SlotKey | null;
  /** The number of prior pending values overwritten in this slot. */
  readonly folded: NonNegativeInteger;
}

/** A credit-disciplined data frame with the remaining unpublished item count. */
export interface CreditFrame<Payload, Anchor> {
  /** The data-frame discriminant. */
  readonly kind: "Data";
  /** The registration-owned payload. */
  readonly payload: Payload;
  /** The frame's consistency anchor. */
  readonly anchor: Anchor;
  /** Items not yet published after this frame in the same world snapshot (u64). */
  readonly pending_after: import("@circular/protocol/actor-query").CircularUInt;
}

/** The unique retained-to-live transition marker in every subscription stream. */
export interface RetentionComplete<Anchor> {
  /** The transition-frame discriminant. */
  readonly kind: "RetentionComplete";
  /** The cut shared by retained and subsequent live frames. */
  readonly anchor: Anchor;
  /** Number of retained frames delivered before this marker. */
  readonly delivered: import("@circular/protocol/actor-query").CircularUInt;
}

export type ResetFloorOrCursor = Nominal<string, "ResetFloorOrCursor">;

/** The closed subscription-termination reason union. */
export type SubscriptionEndReason =
  | { readonly kind: "ByClient" }
  | { readonly kind: "ConsumerBehind" }
  | { readonly kind: "TargetGone" }
  | { readonly kind: "Withdrawn" }
  | { readonly kind: "SessionClosed" }
  | { readonly kind: "ResetRequired"; readonly floorOrCursor: ResetFloorOrCursor }
  | { readonly kind: "ScopeGone"; readonly cursor: StructureCursor }
  | { readonly kind: "IncompatibleClient"; readonly requiredEnvironment: AuthoringEnvironment }
  /** A target with a finite answer delivered every frame it will produce. */
  | { readonly kind: "Complete" };

/** The terminal value of a subscription stream. */
export interface SubscriptionEnded<Anchor> {
  /** The terminal-frame discriminant. */
  readonly kind: "Ended";
  /** The closed reason that determines the caller's recovery action. */
  readonly reason: SubscriptionEndReason;
  /** The registered diagnostic code for this termination site. */
  readonly diagnostic: Diagnostic;
  /** The last valid stream anchor. */
  readonly anchor: Anchor;
}

/** A positive frame-credit grant for one live credit subscription. */
export interface CreditRequest {
  /** The subscription correlation identity. */
  readonly subscription: CorrelationId;
  /** Positive number of additional frames accepted by the consumer. */
  readonly amount: PositiveInteger;
}

/** A request to terminate one live subscription by its correlation identity. */
export interface UnsubscribeRequest {
  /** The subscription correlation identity. */
  readonly subscription: CorrelationId;
}

/** A frame of the lossless protocol-owned authoring commit target. */
export interface AuthoringCommitFrame {
  /** The accepted absolute semantic command epoch. */
  readonly epoch: RpcEpoch<DeclarationContentCommand<AbsoluteAddressDomain>>;
  /** Metadata atomically committed with the epoch. */
  readonly metadata: AcceptedCommitMetadata;
  /**
   * The reconstructive snapshot rows this epoch changed, derived effects of the daemon fold
   * included (MoveToScope boundary synthesis and rewiring, retire cascades), with absolute
   * addresses. Retire rows come first; a retire row removes one key, and `RetireActor` also
   * removes that actor's presentation. Folding a snapshot's rows with the frames after its anchor
   * cursor yields the later snapshot's rows.
   */
  readonly delta: readonly (
    | ReconstructiveDeclarationCommand<AbsoluteAddressDomain>
    | RetireTemplateCommand
    | RetireScopeCommand<AbsoluteAddressDomain>
    | RetireActorCommand<AbsoluteAddressDomain>
    | RetireEdgeCommand<AbsoluteAddressDomain>
    | RetireExportMountCommand<AbsoluteAddressDomain>
    | RetireAnnotationCommand<AbsoluteAddressDomain>
  )[];
}

/** Arguments for the protocol-owned authoring commit subscription target. */
export interface AuthoringCommitArguments {
  /** Scope whose revision map membership filters accepted commits. */
  readonly scope: ScopeId;
  /** Last snapshot or commit cursor already incorporated by the caller. */
  readonly after: StructureCursor;
}

/**
 * A client-chosen, non-empty opaque byte key for export injection retries.
 *
 * A **byte string**. It was declared here as a tuple of numbers, and a client that believed the
 * declaration built one — which encodes as an `Array` of Floats and is refused by
 * `decode_inject` as `WrongCarrier { key: "idempotency" }`. The published vectors carry
 * `Bytes(1 bytes: [01])`.
 */
export type InjectionKey = Nominal<Uint8Array, "InjectionKey">;

/** The exact export-surface event injection payload. */
export interface InjectRequest {
  /**
   * The declared export mount, never an arbitrary actor or port: its authored scope and its name
   * in that scope — the key its `UpsertExportMount` declared it under. A root-scope mount is
   * `scope: []`. A bare name is not an address (mount names are unique per scope).
   */
  readonly mount: { readonly scope: readonly ScopeSegment[]; readonly local: ExportName };
  /** The first-order event payload. */
  readonly payload: CircularValue;
  /** The required client-chosen retry identity. */
  readonly idempotency: InjectionKey;
}

/**
 * The accepted arm of `InjectAck`: Rust `CommandResult::Accepted(Accepted::Nothing)`, wire tag `1`
 * alone (`crates/protocol/src/declaration_payload/result.rs`). The acceptance carries no fact —
 * the arrival's stamp belongs to the receiving boundary's own record, not to this answer.
 */
export type InjectionAccepted = undefined;

/** The two terminal decisions available for a pending approval item. */
export type ApprovalDecision = "Approve" | "Deny";

/**
 * A durable approval decision request.
 *
 * The pending effect belongs to this pipeline. The decision checks whether
 * that item is still pending, independently of other items in the queue.
 */
export interface ApprovalDecideRequest {
  /** The pending effect whose approval this decides. */
  readonly item: EffectId;
  /** The requested terminal decision. */
  readonly decision: ApprovalDecision;
}

/** A registered observation-control name. */
export type ObservationControlName = Nominal<string, "ObservationControlName">;

/** A durable registered observation-control transition. */
export interface SetObservationControlRequest {
  /** The registered control name. */
  readonly name: ObservationControlName;
  /** The registration-owned first-order value. */
  readonly value: CircularValue;
}

export interface SetAgentHarnessRequest {
  /** A harness name the daemon has an adapter for. */
  readonly name: string;
  /** The executable to bind, or `null` to erase the binding. */
  readonly program: string | null;
}

/** The accepted value of `SetAgentHarness`. No journal record is written, so `at` is `null`. */
export interface SetAgentHarnessAccepted {
  readonly at: null;
}

/** Evidence that a ledger transition was durably committed before its response. */
export interface LedgerTransitionAccepted {
  /** The stable key of the resulting fact. */
  readonly fact: string;
  /** The durable ledger revision containing the transition. */
  readonly revision: Digest;
}

/** Start exactly the authored cut the caller observed. */
export interface ResumeRequest {
  readonly expectedAuthoringRevision: AuthoringRevision;
}

export type PauseMode = "Pause" | "ForcePause";

/** Manual interrupt of the standing pipeline; omission means Pause. */
export interface PauseRequest {
  readonly mode?: PauseMode;
}

/** Accepted lifecycle transition of the standing pipeline. */
export type LifecycleAccepted =
  | { readonly kind: "Resumed" }
  | { readonly kind: "Paused" };

export declare function lifecycleResultFromValue(value: unknown): Result<LifecycleAccepted>;

/** A scope identity exactly as the daemon publishes it: `[1, name]` / `[2, of, key]` segments. */
export type ObservedScope = readonly (readonly [1n, string] | readonly [2n, string, InstanceTransitionKey])[];
/** A plan actor key exactly as the daemon publishes it: a scope and a local name. */
export interface ObservedActor {
  readonly scope: ObservedScope;
  readonly local: string;
}
/** An endpoint of a recorded edge or an unwired outlet. */
export interface DeadLetterEndpoint {
  readonly actor: ObservedActor;
  readonly port: string;
}
export type DeadLetterTarget =
  | readonly [1n, DeadLetterEndpoint, DeadLetterEndpoint, bigint]
  | readonly [2n, ObservedActor]
  | DeadLetterEndpoint;
/** The closed table `DeadLetterReasonKind` from `@circular/protocol/tables`. */
export type DeadLetterReasonCode = (typeof ClosedTables.DeadLetterReasonKind)[number];
/** `DeadLetterReason` carrier `{code, detail}`: `actor_declared` carries its declared name,
 * `processing` its cause, and every other code is a unit arm carrying `null`. */
export type DeadLetterReason =
  | { readonly code: Exclude<DeadLetterReasonCode, "actor_declared" | "processing">; readonly detail: null }
  | { readonly code: "actor_declared"; readonly detail: string }
  | { readonly code: "processing"; readonly detail: unknown };
/** One decoded `dead.letters` item; its subject stays the exact recorded value. */
export interface DeadLetterItem {
  /** Stamp of the dropped emission/delivery, distinct from the recorder's observation time. */
  readonly dropped: import('./internal/record-values.js').RecordStamp;
  /** The authored actor and origin port; a `null` port is a published fact, not an unread field. */
  readonly origin: { readonly actor: ObservedActor; readonly port: string | null };
  readonly reason: DeadLetterReason;
  /** The scope of the actor that recorded the dead letter. */
  readonly scope: ObservedScope;
  /** The failed product and its port shape (exact wire value). */
  readonly subject: { readonly shape: unknown; readonly value: unknown };
  /** Failure location; absence is unknown and is not inferred from the origin. */
  readonly target?: DeadLetterTarget;
  /** Recorded observation bucket in milliseconds; absent when the source carries none. */
  readonly observationBucket?: number;
}
/** `DeadLetterRow`: one item of a complete stream answer, with its position in that answer. */
export interface DeadLetterRow {
  readonly dropped: import('./internal/record-values.js').RecordStamp;
  readonly ordinal: bigint;
  readonly actor: ObservedActor;
  readonly port: string | null;
  readonly target: DeadLetterTarget | null;
  readonly reason: DeadLetterReason;
  readonly scope: ObservedScope;
  readonly subject_shape: unknown;
  readonly subject: unknown;
  /** Recorded observation bucket in milliseconds; absent when the source carries none. */
  readonly observationBucket?: number;
}
/** `DeadLetterProjection`: the rows cut from this state's stream. */
export interface DeadLetterProjection {
  readonly rows: readonly DeadLetterRow[];
}
export interface PreprocessFailurePoint {
  readonly edge: readonly [1n, DeadLetterEndpoint, DeadLetterEndpoint, bigint] | readonly [2n, ObservedActor];
  readonly index: bigint;
  readonly kind: string;
  readonly code: string | null;
}
export declare const DEAD_LETTER_REASONS: readonly DeadLetterReasonCode[];
export declare function deadLetterReasonFromValue(value: unknown): DeadLetterReason;
export declare function deadLetterTargetFromValue(value: unknown): DeadLetterTarget;
export declare function deadLetterItemFromValue(value: unknown): DeadLetterItem;
/** `preprocess_failure_point`: `null` when the cause carries no location. */
export declare function preprocessFailurePoint(detail: unknown): PreprocessFailurePoint | null;
/** `decode_dead_letters`: a complete answer's wire or decoded page items; any bad row refuses the whole answer. */
export declare function decodeDeadLetters(anchor: unknown, items: readonly unknown[]): DeadLetterProjection;

/** An instance key: a scalar or an ordered scalar tuple (tuple boundaries are identity). */
export type InstanceTransitionKey = string | bigint | boolean | readonly (string | bigint | boolean)[];
/** `InstanceTransition`: one instance-set mint or retirement of `container` (the instance-set scope). */
export interface InstanceTransition {
  readonly kind: "Minted" | "Retired";
  readonly container: ObservedScope;
  readonly key: InstanceTransitionKey;
}
/** `LifecyclePhase`, the closed incarnation transition reasons. */
export type IncarnationPhase =
  | "Draining" | "AdmissionClosed" | "Terminated" | "Prepared" | "Activated"
  | "Restarted" | "ConfigRestarted" | "Abandoned" | "ResumeDenied" | "ConfigApplied";
export interface IncarnationTransition {
  readonly phase: IncarnationPhase;
  readonly actor: ObservedActor;
  readonly generation: bigint;
  readonly declaration_revision: bigint;
  readonly config_revision: bigint;
  readonly at: bigint;
  readonly retired: bigint;
  readonly prepared: bigint;
  readonly remaining: bigint;
  readonly next_recovery: bigint;
}
/** `ActiveInstanceFactTable`: the live instance set of one exact container, in the order the keys became live. */
export interface ActiveInstanceFactTable {
  readonly container: ObservedScope;
  readonly rows: readonly { readonly key: InstanceTransitionKey }[];
}
export declare const INCARNATION_PHASES: readonly IncarnationPhase[];
export declare function instanceTransitionFromValue(value: unknown): InstanceTransition | IncarnationTransition;
/** `decode_instance_transitions`: every row of the answer in recorded order. */
export declare function decodeInstanceTransitions(items: readonly unknown[]): readonly (InstanceTransition | IncarnationTransition)[];
/** `project_active_instance_facts`: fold a complete answer into one container's live set. */
export declare function projectActiveInstanceFacts(
  anchor: unknown, items: readonly unknown[], container: ObservedScope,
): ActiveInstanceFactTable;

/** `ActorCreateInputPath`: `[1, key]` and `[2, index]` segments, never empty. */
export type ActorCreateInputPath = readonly (readonly [1n, string] | readonly [2n, bigint])[];
/** `ConfigInputConstraint` in its published arms. */
export type ConfigInputConstraint =
  | readonly [1n, "milliseconds" | "nonzero_milliseconds"]
  | readonly [2n, 0n | 1n, bigint | null]
  | readonly [3n] | readonly [4n]
  | readonly [5n, readonly string[]]
  | readonly [6n, "label" | "name" | "tool_name" | "model_provider_name" | "model_name"]
  | readonly [7n] | readonly [8n]
  /** Canonical Stream(Base), using PORT_BASE_SHAPES. */
  | readonly [9n]
  /** A list of intervals, each item in the domain of arm 1. */
  | readonly [10n, "milliseconds" | "nonzero_milliseconds"];
/**
 * `ConfigInputRequirement`: `[1]` Mandatory, `[2, default]` Optional, `[3, absent]` Omittable.
 * Omittable has no config default; `absent` is the port Flow carrier the slot's boundary takes
 * when the key is omitted (read it with `decodePortFlow`).
 */
export type ConfigInputRequirement = readonly [1n] | readonly [2n, unknown] | readonly [3n, unknown];
/** `ConfigInputSnippet`. */
export interface ConfigInputSnippet {
  readonly mode: "transform" | "predicate" | "number" | "reduce";
  readonly inlets: readonly string[];
}
/** `ActorCreateInputSlot`: one config slot. `shape` is the exact port-shape wire value. */
export interface ActorCreateInputSlot {
  /** Registration-owned slot text (ConfigSlotMetadata); null when undeclared. */
  readonly label: string | null;
  readonly description: string | null;
  readonly group: string | null;
  readonly path: ActorCreateInputPath;
  readonly shape: unknown;
  readonly constraint: ConfigInputConstraint | null;
  readonly requirement: ConfigInputRequirement;
  readonly snippet: ConfigInputSnippet | null;
  /** Declared capability policies and their mandatory inputs, when the slot declares any. */
  readonly policies?: { readonly [policy: string]: readonly ActorCreateInputSlot[] };
}
/** `ActorCreateInputRelation`: `[1, path]` UniqueObjectValues. */
export type ActorCreateInputRelation = readonly [1n, ActorCreateInputPath];
/** `ActorCreateInputState`: `[1]` NotRequired, `[2, {slots, relations}, draft, missing]` Authoring, `[3, reason]` Unavailable. */
export type ActorCreateInputState =
  | readonly [1n]
  | readonly [2n, { readonly slots: readonly ActorCreateInputSlot[]; readonly relations: readonly ActorCreateInputRelation[] },
      { readonly [key: string]: unknown }, readonly ActorCreateInputPath[]]
  | readonly [3n, string];
/** `ActorCreateInputCatalogEntry`. */
export interface ActorCreateInputCatalogEntry {
  readonly actor_type: string;
  readonly state: ActorCreateInputState;
}
export declare function actorCreateInputSlotFromValue(value: unknown): ActorCreateInputSlot;
export declare function actorCreateInputCatalogEntryFromValue(value: unknown): ActorCreateInputCatalogEntry;
/** `decode_actor_create_inputs`: the anchor names every entry, in order, once. */
export declare function decodeActorCreateInputs(anchor: unknown, items: readonly unknown[]): readonly ActorCreateInputCatalogEntry[];

/** One per-actor prefix length of a replay cut — the codec's component shape. */
export interface ReplayCutComponent {
  /** The actor whose arrival-log prefix this component measures. */
  readonly actor: unknown;
  /** The prefix length: how many of that actor's recorded arrivals the cut holds. */
  readonly index: bigint;
}

export interface ReplayTarget {
  readonly stream: bigint;
  readonly revision_epoch: bigint;
  readonly cut: readonly ReplayCutComponent[];
}

/** Whether a divergent replay's tail consumes live input or remains dry. */
export type ReplayTail = "Live" | "DryRun";

/** The four closed replay arrangements; the three with an origin carry one coordinate. */
export type ReplayArrangement =
  | { readonly kind: "Observational"; readonly from: ReplayTarget }
  | { readonly kind: "Counterfactual"; readonly from: ReplayTarget }
  | { readonly kind: "Divergent"; readonly at: ReplayTarget; readonly tail: ReplayTail }
  | "DryRun";

/** The four closed replay paces; Realtime is a reduced multiple of the recorded intervals. */
export type ReplayPace =
  | "Free"
  | "Paused"
  | { readonly kind: "Step"; readonly upto: ReplayTarget }
  | { readonly kind: "Realtime"; readonly num: bigint; readonly den: bigint };

/** `ReplayStart { arrangement, pace }` — two places and no third. */
export interface ReplayStartRequest {
  readonly arrangement: ReplayArrangement;
  readonly pace: ReplayPace;
}

export interface ReplayRewindRequest {
  readonly to?: ReplayTarget;
  readonly pace: ReplayPace;
}

/** `ReplayEnd` has no body; the envelope's correlation names the session. */
export type ReplayEndRequest = null;

/** A complete decoded-envelope result or a normal structured rejection. */
export type DecodeResult<E> = { readonly status: "complete"; readonly envelope: E } | Rejected;

/** A framing codec port; concrete Cap'n Proto bindings belong behind this boundary. */
export interface EnvelopeCodec<Frame> {
  /** Encodes one complete envelope using the negotiated canonical wire representation. */
  encode(envelope: Envelope): Frame;
  /** Decodes exactly one complete frame without exposing a partial-envelope state. */
  decode(frame: Frame): DecodeResult<Envelope>;
}

/** An ordered complete-frame transport with no transaction or application-state semantics. */
export interface Transport<Frame> {
  /** Sends one complete frame in session order. */
  send(frame: Frame): Promise<void>;
  /** Receives complete frames in sender order until the transport closes. */
  readonly incoming: AsyncIterable<Frame>;
  /** Closes transport resources; it does not synthesize protocol success. */
  close(): Promise<void>;
}

/** Stable protocol partitions in canonical declaration order. */
export declare const stablePartitions: readonly Partition[];

/** Stable verbs, excluding dynamically negotiated experimental verbs. */
export declare const stableVerbs: readonly StableVerb[];

/** Constructs the normal success channel. */
export declare function accepted<T>(value: T): Accepted<T>;

/** Constructs the normal structured rejection channel. */
export declare function rejected(
  reason: RejectionReason,
  diagnostics: NonEmptyReadonlyArray<Diagnostic>,
): Rejected;

/** Narrows an unknown value to the normal success channel. */
export declare function isAccepted<T = unknown>(value: unknown): value is Accepted<T>;

/** Narrows an unknown value to a structured protocol rejection. */
export declare function isRejected(value: unknown): value is Rejected;

/** Constructs one transport-independent envelope. */
export declare function envelope<
  P extends Partition,
  V extends PartitionVerb<P>,
  Payload,
>(partition: P, verb: V, correlation: CorrelationId, payload: Payload): Envelope<P, V, Payload>;

/** Defines a feature-owned query descriptor without materializing its type witnesses. */
export declare function defineQuery<Args, Item, Anchor, Paging extends QueryPaging>(
  descriptor: Pick<QueryDescriptor<Args, Item, Anchor, Paging>, "name" | "paging" | "anchorKind">,
): QueryDescriptor<Args, Item, Anchor, Paging>;

/** Defines a feature-owned subscription descriptor without materializing its type witnesses. */
export declare function defineSubscription<
  Args,
  Frame,
  Anchor,
  Discipline extends DeliveryDiscipline,
>(
  descriptor: Pick<SubscriptionDescriptor<Args, Frame, Anchor, Discipline>, "name" | "discipline">,
): SubscriptionDescriptor<Args, Frame, Anchor, Discipline>;

/** Zero-copy framing for in-process transports; remote transports provide a negotiated codec. */
export declare const identityEnvelopeCodec: EnvelopeCodec<Envelope>;

/** The four resource ceilings every value codec entry point requires from its caller. */
export interface ValueResourceCeilings {
  /** Total encoded bytes admitted for one value. */
  readonly maximumBytes: number;
  /** Nesting depth admitted for one value. */
  readonly maximumDepth: number;
  /** Entries admitted in one array or object. */
  readonly maximumContainerEntries: number;
  /** Bytes admitted for one string or byte string. */
  readonly maximumStringBytes: number;
}

/**
 * The negotiated codec for a byte transport: a twelve-byte head, a payload version tag, and one
 * canonical value.
 *
 * The ceilings have no default here on purpose — three of the four are open pending
 * measurement, so a codec that supplied its own would choose a limit the caller never picked.
 */
export declare function wireEnvelopeCodec(
  resourceCeilings: ValueResourceCeilings,
): EnvelopeCodec<Uint8Array>;

/** Rust `circular_core::BaseShape` in its wire spelling: the closed table `BaseShape` from `@circular/protocol/tables`. */
export type BaseShape = (typeof ClosedTables.BaseShape)[number];

/** The base spellings accepted by the port Flow/Shape carrier reader. */
export declare const PORT_BASE_SHAPES: readonly BaseShape[];

/** Rust `PortShape` (`crates/protocol/src/port_type.rs`), wire arms 1–5. */
export type PortShape =
  | { readonly kind: "Any" }
  | { readonly kind: "Base"; readonly base: BaseShape }
  | { readonly kind: "Array"; readonly item: PortShape }
  | { readonly kind: "Object"; readonly fields: readonly PortShapeField[]; readonly open: boolean }
  | { readonly kind: "Variable"; readonly name: string };

/** Rust `PortShapeField`. */
export interface PortShapeField {
  readonly name: string;
  readonly shape: PortShape;
}

/** Rust `PortRate`: a positive tick period or a rate variable. */
export type PortRate =
  | { readonly kind: "Period"; readonly ticks: bigint }
  | { readonly kind: "Variable"; readonly name: string };

/** Rust `PortFlow`. */
export type PortFlow =
  | { readonly kind: "Stream"; readonly item: PortShape }
  | { readonly kind: "Signal"; readonly item: PortShape; readonly rate: PortRate };

/**
 * Rust `PortFlowAvailability`: the `flow` member of one
 * `authoring.actor-ports` port row. `Unavailable` carries the producer's reason as a value.
 */
export type PortFlowAvailability =
  | { readonly kind: "Known"; readonly flow: PortFlow }
  | { readonly kind: "Unavailable"; readonly reason: string };

/** Rust `decode_port_shape`; refuses exactly what the Rust decoder refuses. Throws `TypeError`. */
export declare function decodePortShape(value: unknown): PortShape;

export { CircularUInt } from "./value.js";

/** Rust `decode_port_flow`; refuses exactly what the Rust decoder refuses. Throws `TypeError`. */
export declare function decodePortFlow(value: unknown): PortFlow;

/**
 * Rust `encode_port_flow`: a Flow as its wire carrier, the inverse of `decodePortFlow`. Refuses what
 * the decoder refuses. Throws `TypeError`.
 */
export declare function encodePortFlow(flow: PortFlow): CircularValue;

export type TimelineMarkKind = "edit" | "restart" | "pause" | "resume";

/** The closed four in declaration order. */
export declare const TIMELINE_MARK_KINDS: readonly TimelineMarkKind[];

/** The registered name of the bar summary query. */
export declare const TIMELINE_BINS_QUERY: "timeline.bins";

/** The registered name of the instant's replay-coordinate query. */
export declare const TIMELINE_AT_QUERY: "timeline.at";

/** The largest bin count one `timeline.bins` answer carries. */
export declare const TIMELINE_MAX_BINS: bigint;

/**
 * `timeline.bins` arguments. Instants are milliseconds on the recorded axis; `actor` narrows the
 * arrival and incident counts to one actor (marks are the stream's).
 */
export interface TimelineBinsArgs {
  readonly from_ms: bigint | import("./value.js").CircularUInt;
  readonly to_ms: bigint | import("./value.js").CircularUInt;
  readonly bins: bigint | import("./value.js").CircularUInt;
  readonly actor?: BoundaryPortActorKey;
}

/** `timeline.at` arguments. */
export interface TimelineAtArgs {
  readonly at_ms: bigint | import("./value.js").CircularUInt;
}

/**
 * One bin: its arrival count and its incident count (dead letters and failed health transitions).
 * Arrival counts include activation, edit and stop arrivals on `_lifecycle`.
 * `arrival.scan` omits those lifecycle arrivals.
 */
export interface TimelineBin {
  readonly count: bigint;
  readonly incidents: bigint;
}

/** One mark on the bar — at most one per kind per bin. */
export interface TimelineMark {
  readonly kind: TimelineMarkKind;
  readonly at_ms: bigint;
}

/** A `timeline.bins` answer. `to_ms` is the end the bins actually cover. */
export interface TimelineBins {
  readonly from_ms: bigint;
  readonly to_ms: bigint;
  readonly bin_ms: bigint;
  readonly bins: readonly TimelineBin[];
  readonly marks: readonly TimelineMark[];
  /** Arrivals whose observed instant preceded their column's running maximum. */
  readonly clock_regressions: bigint;
}

/** A `timeline.at` answer. */
export interface TimelineAt {
  readonly requested_ms: bigint;
  /** The observed instant of the last arrival inside the cut (running maximum); 0 when the cut is empty. */
  readonly resolved_ms: bigint;
  /**
   * The replay coordinate `{stream, revision_epoch, cut}` exactly as the wire carries it — the spelling
   * a `timeline` checkpoint has (without its instant). At runtime the replay lens entry
   * `replayTargetFromCheckpoint` (the client's `replay.start({from})` / the lens's `rewind({to})` / Step
   * pace) reads it unchanged. The client's replay types accept it as `TimelineAt["target"]` beside
   * `TimelineItem` (`@circular/client/replay` `from` / `to` / Step `upto`).
   */
  readonly target: {
    readonly stream: bigint;
    readonly revision_epoch: import("./value.js").CircularUInt;
    /** The wire cut: one `{actor, index}` per recorded actor column, `index` an Int prefix length. */
    readonly cut: readonly { readonly actor: unknown; readonly index: bigint }[];
  };
}

/** Rust `TimelineBinsArgs::coverage`: the bin width and the covered end. Throws `TypeError` outside the value space. */
export declare function timelineCoverage(args: Pick<TimelineBinsArgs, "from_ms" | "to_ms" | "bins">): {
  readonly bin_ms: bigint;
  readonly to_ms: bigint;
};

/** The `timeline.bins` argument value; refuses what the daemon refuses. */
export declare function timelineBinsArgsValue(args: TimelineBinsArgs): unknown;

/** The `timeline.at` argument value. */
export declare function timelineAtArgsValue(args: TimelineAtArgs): unknown;

/** Reads a `timeline.bins` answer (the page anchor). Throws `TypeError`. */
export declare function timelineBinsFromValue(value: unknown): TimelineBins;

/** Reads a `timeline.at` answer (the page anchor). Throws `TypeError`. */
export declare function timelineAtFromValue(value: unknown): TimelineAt;
