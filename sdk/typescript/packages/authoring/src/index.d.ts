/** @packageDocumentation Client-side authoring and execution contracts. */

import type {
  NormalizedModulePath,
  SourceModuleBytes,
  SourceProgramBundle,
  AuthoringDiagnostic,
  SdkOperationResult,
} from "@circular/generator";

import type {
  AcceptedCommitMetadata,
  AnnotationId,
  AuthoringEnvironment,
  AuthoringSnapshotAnchor,
  CanonicalBindingName,
  CircularRecord,
  CircularValue,
  CommitId,
  CompactedDeclarationCommandList,
  CorrelationId,
  CurrentAuthoringRevision,
  DeclarationContentCommand,
  Diagnostic,
  Digest,
  EdgeAttributes,
  EdgeId,
  EpochAddressDomain,
  ExportName,
  ActorDeclaration,
  ActorId,
  NonEmptyReadonlyArray,
  PortEndpoint,
  QueryPage,
  ReconstructiveDeclarationCommand,
  Rejected,
  RelativeAddressDomain,
  ScopeId,
  SourceSpan,
} from "@circular/protocol";
import type { Session } from "@circular/client";
import type {
  CurrentEdgeHandle,
  CurrentActorHandle,
  EdgeOptions,
  HandleMode,
  PublicActorSpelling,
  SourceEndpoint,
  TargetEndpoint,
} from "@circular/core";
import type {
  ConstructorExportName,
  ModuleSpecifierOrigin,
} from "@circular/specs";

declare const authoringNominal: unique symbol;
declare const preparedProgramBrand: unique symbol;
declare const expressionTermBrand: unique symbol;
declare const completeAuthoringSnapshotBrand: unique symbol;

type Nominal<Base, Name extends string> = Base & {
  readonly [authoringNominal]: Name;
};

/** A pinned semantic-prepass and canonical-authoring profile identity. */
export type AuthoringProfileId = Nominal<string, "AuthoringProfileId">;

/** A version of the semantic-prepass transform and recognition rules. */
export type SemanticPrepassVersion = Nominal<string, "SemanticPrepassVersion">;

/** A canonical authoring virtual module recognized by the client host. */
export type CanonicalVirtualModuleSpecifier = "circular:current";

/** An opaque expression term captured from an admitted dataflow callback. */
export interface SemanticExpressionTerm {
  /** Prevents arbitrary values from masquerading as validated expression terms. */
  readonly [expressionTermBrand]: true;
}

/** A request to resolve one static dependency inside a source bundle. */
export interface BundleModuleRequest {
  /** The module containing the dependency reference. */
  readonly referrer: NormalizedModulePath;
  /** The literal manifest-relative module specifier recorded by prepass. */
  readonly specifier: string;
}

/** A dependency resolved without consulting a process working directory or server filesystem. */
export interface ResolvedBundleModule {
  /** The normalized module path selected inside the supplied bundle. */
  readonly path: NormalizedModulePath;
  /** The immutable module bytes selected at that path. */
  readonly bytes: SourceModuleBytes;
}

/** A bundle-only resolver used by the code execution host. */
export interface BundleModuleResolver {
  /** Resolves a static dependency or returns diagnostics without escaping the supplied bundle. */
  resolve(
    bundle: SourceProgramBundle,
    request: BundleModuleRequest,
  ): SdkOperationResult<ResolvedBundleModule, AuthoringDiagnostic>;
}

/** One generated-to-original source mapping segment produced by semantic prepass. */
export interface AuthoringSourceMapSegment {
  /** The range in the transformed module evaluated by a host. */
  readonly generated: SourceSpan;
  /** The corresponding range in the original submitted module. */
  readonly original: SourceSpan;
}

/** The immutable mapping from transformed authoring modules to original source ranges. */
export interface AuthoringSourceMap {
  /** All non-overlapping source-map segments in generated-range order. */
  readonly segments: readonly AuthoringSourceMapSegment[];
}

/** A pinned profile used to recognize and transform an authored TypeScript bundle. */
export interface SemanticPrepassProfile {
  /** The canonical authoring profile identity. */
  readonly id: AuthoringProfileId;
  /** The exact transform and semantic-recognition version. */
  readonly version: SemanticPrepassVersion;
  /** The environment whose specs and provider bindings interpret the program. */
  readonly environment: AuthoringEnvironment;
  /** The allowed canonical virtual modules for this profile. */
  readonly virtualModules: ReadonlySet<CanonicalVirtualModuleSpecifier>;
}

/** A static module dependency discovered by semantic prepass. */
export interface PreparedModuleDependency {
  /** The module containing the dependency. */
  readonly referrer: NormalizedModulePath;
  /** The original source range of the module specifier. */
  readonly origin: SourceSpan;
  /** The normalized bundle-local module selected by the resolver. */
  readonly resolved: NormalizedModulePath;
}

/** A prepassed executable bundle whose semantic metadata cannot be forged by callers. */
export interface PreparedProgram {
  /** The original submitted source bundle. */
  readonly source: SourceProgramBundle;
  /** The transformed module bundle evaluated by the code execution host. */
  readonly executable: SourceProgramBundle;
  /** The exact semantic-prepass profile applied to the source. */
  readonly profile: SemanticPrepassProfile;
  /** The transformed-to-original diagnostic source map. */
  readonly sourceMap: AuthoringSourceMap;
  /** All validated nested-scope and template dependencies. */
  readonly dependencies: readonly PreparedModuleDependency[];
  /** Prevents callers from constructing a prepared program without semantic prepass. */
  readonly [preparedProgramBrand]: true;
}

/** Options that constrain semantic prepass to a supplied bundle resolver. */
export interface SemanticPrepassOptions {
  /** The bundle-only resolver used for nested scope and template module references. */
  readonly moduleResolver?: BundleModuleResolver;
  /** Maximum child scope depth; defaults to 64. */
  readonly maximumDepth?: number;
}

/** Prepasses a whole module bundle without executing it or opening a mutation epoch. */
export declare function semanticPrepass(
  bundle: SourceProgramBundle,
  profile: SemanticPrepassProfile,
  options: SemanticPrepassOptions,
): SdkOperationResult<PreparedProgram, AuthoringDiagnostic>;

/** One reconstructive snapshot item using the protocol's existing declaration-command codec. */
export type AuthoringSnapshotItem = ReconstructiveDeclarationCommand<RelativeAddressDomain>;

/** One anchored protocol query page of existing reconstructive declaration commands. */
export type AuthoringSnapshotPage = QueryPage<AuthoringSnapshotItem, AuthoringSnapshotAnchor>;

/** A complete current authored-state reconstruction suitable for direct client folding. */
export interface CompleteAuthoringSnapshot {
  /** The exact revision, topology, cursor, environment, and queried scope consistency cut. */
  readonly anchor: AuthoringSnapshotAnchor;
  /** The compacted ordered reconstructive subset of the existing declaration-command union. */
  readonly commands: CompactedDeclarationCommandList;
  /** Prevents partial pages or unchecked command arrays from masquerading as a complete snapshot. */
  readonly [completeAuthoringSnapshotBrand]: true;
}

/** A current authored-scope handle whose removal lowers to explicit scope retirement. */
export interface CurrentScopeHandle {
  /** The stable current scope identity bound to one snapshot anchor. */
  readonly id: ScopeId;
  /** Enqueues explicit removal in the current mutation epoch. */
  remove(): void;
}

/** A current export-mount handle whose removal lowers to explicit mount retirement. */
export interface CurrentExportHandle {
  /** The stable export name bound to one snapshot anchor. */
  readonly name: ExportName;
  /** Enqueues explicit removal in the current mutation epoch. */
  remove(): void;
}

/** A current annotation handle whose removal lowers to explicit annotation retirement. */
export interface CurrentAnnotationHandle {
  /** The stable annotation identity bound to one snapshot anchor. */
  readonly id: AnnotationId;
  /** Enqueues explicit removal in the current mutation epoch. */
  remove(): void;
}

/**
 * A source-level exact edge key validated and nominalized by the current-module resolver.
 * The key is the JSON text of the edge's `EdgeId` value with object keys sorted, in
 * snapshot-relative form: `{"from":{"actor":{"local":"a","scope":[]},"port":"<outlet id>"},
 * "ordinal":0,"to":{"actor":{"local":"b","scope":[]},"port":"<inlet id>"}}`. A port is its
 * port id as the daemon answers it (`authoring.actor-ports`); an `input` actor's outlet and a
 * container boundary port carry a derived id (e.g. `_bi1_…`) that the author never spelled.
 * Read the exact key from the snapshot's `UpsertEdge.edge.value`, or look the edge up by its
 * endpoints instead: `current.edge(source, target, { ordinal })`.
 */
export type CurrentEdgeKey = string | EdgeId;

/** A source-level exact scope key validated and nominalized by the current-module resolver. */
export type CurrentScopeKey = string | ScopeId;

/** A source-level exact export key validated and nominalized by the current-module resolver. */
export type CurrentExportKey = string | ExportName;

/** A source-level exact annotation key validated and nominalized by the current-module resolver. */
export type CurrentAnnotationKey = string | AnnotationId;

/** Complete snapshot-bound lookups shared by the named exports and `current` namespace. */
export interface CurrentLookupNamespace {
  /** Resolves one exact canonical binding or snapshot-relative actor address. */
  actor(binding: string | CanonicalBindingName): CurrentActorHandle<PublicActorSpelling>;
  /** Resolves one exact current edge for explicit disconnection or a delay/policy change.
   * Preprocess steps change by redeclaring the edge, not through this handle. */
  edge(id: CurrentEdgeKey): CurrentEdgeHandle;
  /** Resolves one exact current edge by its endpoints: `source` is an
   * outlet (`handle.out.x`, or a handle with a primary outlet), `target` an inlet (`handle.in.x`, or a
   * handle with a primary inlet), and an omitted ordinal is 0 — the same identity an omitted ordinal
   * declares. Derived port ids (an `input` actor's outlet, a container boundary) come from the
   * handles, so the author never spells them. A missing edge is `authoring.current.lookup-missing`
   * with the searched key; an option other than `ordinal` is `authoring.current.edge-options-invalid`. */
  edge<SourceValue, SourceMode extends HandleMode, TargetValue, TargetMode extends HandleMode>(
    source: SourceEndpoint<SourceValue, SourceMode>,
    target: TargetEndpoint<TargetValue, TargetMode>,
    options?: { readonly ordinal?: number },
  ): CurrentEdgeHandle;
  /** Resolves one exact current authored scope for explicit removal. */
  scope(id: CurrentScopeKey): CurrentScopeHandle;
  /** Resolves one exact current export mount for explicit removal. */
  export(name: CurrentExportKey): CurrentExportHandle;
  /** Resolves one exact current annotation for explicit removal. */
  annotation(id: CurrentAnnotationKey): CurrentAnnotationHandle;
}

/** The host record behind one anchor-specific `circular:current` ESM module. */
export interface CurrentProjectModule {
  /** The exact complete snapshot anchor to which every exported handle is bound. */
  readonly anchor: AuthoringSnapshotAnchor;
  /** The stable lookup namespace accompanying the generated named actor exports. */
  readonly current: CurrentLookupNamespace;
}

/** A request to bind the current-state virtual module to one complete snapshot. */
export interface CurrentProjectResolutionRequest {
  /** The only canonical current-state virtual specifier. */
  readonly specifier: "circular:current";
  /** The complete snapshot whose exact revision and environment bind every handle. */
  readonly snapshot: CompleteAuthoringSnapshot;
}

/** Resolves the fixed current-state virtual module without consulting labels or runtime actors. */
export interface CurrentProjectResolver {
  /** Produces one anchor-bound lookup namespace from a complete immutable snapshot. */
  resolve(
    request: CurrentProjectResolutionRequest,
  ): SdkOperationResult<CurrentProjectModule, AuthoringDiagnostic>;
}

/** Ephemeral complete lookup index; it is never serialized, canonical, or sent over RPC. */
export interface CurrentSnapshotIndex {
  /** The exact snapshot anchor used to build every handle. */
  readonly anchor: AuthoringSnapshotAnchor;
  /** Exact canonical-binding or relative-address actor lookups. */
  readonly actors: ReadonlyMap<string, CurrentActorHandle<PublicActorSpelling>>;
  /** Exact edge lookups. */
  readonly edges: ReadonlyMap<string, CurrentEdgeHandle>;
  /** Exact scope lookups. */
  readonly scopes: ReadonlyMap<string, CurrentScopeHandle>;
  /** Exact export mount lookups. */
  readonly exports: ReadonlyMap<string, CurrentExportHandle>;
  /** Exact annotation lookups. */
  readonly annotations: ReadonlyMap<string, CurrentAnnotationHandle>;
}

/** Eagerly binds a complete snapshot through the host's relative-to-live identity binder. */
export interface CurrentSnapshotIndexer {
  /** Builds all lookup entries before user code starts; this method must not perform lazy field RPC. */
  index(snapshot: CompleteAuthoringSnapshot): SdkOperationResult<CurrentSnapshotIndex, AuthoringDiagnostic>;
}

/** Creates the safe complete-snapshot resolver used by `circular:current`. */
export declare function createCurrentProjectResolver(indexer: CurrentSnapshotIndexer): CurrentProjectResolver;

/** All host-supplied inputs required to execute one authored program as one implicit declaration epoch. */
export interface CodeExecutionOptions {
  /** The user-authored scope receiving relative declarations. */
  readonly targetScope: ScopeId;
  /** The mandatory authored-state CAS baseline. */
  readonly expectedRevision: CurrentAuthoringRevision;
  /** The mandatory authoring-environment CAS baseline. */
  readonly expectedEnvironment: AuthoringEnvironment;
  /** The client-chosen durable deduplication identity for this user action. */
  readonly commitId: CommitId;
  /**
   * The complete snapshot the epoch is fenced by, or null before the first commit. It binds
   * `circular:current` handles when the program imports them, and a presentation axis the program does
   * not say keeps its value in it. With null, an actor's presentation starts from the defaults.
   */
  readonly currentSnapshot: CompleteAuthoringSnapshot | null;
}

/** A durably committed program. Use waitForAdoption(session, commit.cursor) before injection. */
export interface CodeExecutionCommitted {
  /** The committed terminal discriminant. */
  readonly status: "committed";
  /** The server-accepted commit metadata and cursor. */
  readonly commit: AcceptedCommitMetadata;
  /** Existing declaration content commands emitted for optimistic inspection. */
  readonly commands: readonly DeclarationContentCommand<EpochAddressDomain>[];
  /** Stable ordered host and admission warnings. */
  readonly diagnostics: readonly AuthoringDiagnostic[];
}

/** A rejected or aborted execution that never exposes a partial committed state. */
export interface CodeExecutionRejected {
  /** The rejected terminal discriminant. */
  readonly status: "rejected";
  /** The structured protocol rejection when the failure reached an RPC boundary. */
  readonly protocol: Rejected | null;
  /** Every stable ordered host, lowering, admission, and terminal diagnostic. */
  readonly diagnostics: NonEmptyReadonlyArray<AuthoringDiagnostic>;
}

/**
 * CommitEpoch was sent and its answer never arrived, and the commit record did not show it.
 * The daemon may have committed the program — this is not a rejection. Read the authoring
 * snapshot (its revision moves when the commit lands) or look for `commitId` in the
 * authoring-commits feed before retrying.
 */
export interface CodeExecutionUnknown {
  /** The unknown-outcome discriminant. */
  readonly status: "unknown";
  /** The durable deduplication identity this execution committed under, if it did. */
  readonly commitId: CommitId;
  /** Existing declaration content commands that were submitted. */
  readonly commands: readonly DeclarationContentCommand<EpochAddressDomain>[];
  /** Why the outcome is unknown, and how to settle it. */
  readonly diagnostics: NonEmptyReadonlyArray<AuthoringDiagnostic>;
}

/** The normal terminal result of one code-execution epoch. */
export type CodeExecutionResult = CodeExecutionCommitted | CodeExecutionRejected | CodeExecutionUnknown;

/** Ports resolved for one constructor under the pinned authoring environment. */
export interface ResolvedActorPorts {
  /** Default input port, or null for a source-only actor. */
  readonly defaultInput: import("@circular/protocol").PortId | null;
  /** Default output port, or null for a sink-only actor. */
  readonly defaultOutput: import("@circular/protocol").PortId | null;
  /** Resolves one declared input name or throws a deterministic host diagnostic. */
  input(name: string): import("@circular/protocol").PortId;
  /** Resolves one declared output name or throws a deterministic host diagnostic. */
  output(name: string): import("@circular/protocol").PortId;
}

/** Complete normalized constructor resolution supplied by specs and catalog constructor bindings. */
export interface ResolvedActorConstructor {
  /** The exact existing protocol actor payload. */
  readonly declaration: ActorDeclaration;
  /** Direction-safe resolved port identities. */
  readonly ports: ResolvedActorPorts;
}

/** Resolves authored constructor spelling without exposing provider package effects to the engine. */
export interface AuthoringActorResolver {
  /** Resolves and normalizes one constructor invocation under the pinned environment. */
  resolve(spelling: PublicActorSpelling, config: unknown): ResolvedActorConstructor;
}

/**
 * Per-execution resolver for normalized edge attributes and the authored ordinal.
 * The ordinal is the authored one, or the handle's existing one, or 0 — never a value chosen
 * from the current graph. Redeclaring an existing identity updates it.
 */
export interface AuthoringEdgeResolver {
  /** Resolves one exact edge payload under the pinned environment. Throws
   * `CIRCULAR_EDGE_ORDINAL_CONFLICT` when another line of the same program already declared
   * this (from, to, ordinal). */
  resolve(
    from: PortEndpoint<EpochAddressDomain>,
    to: PortEndpoint<EpochAddressDomain>,
    options: EdgeOptions,
    existingOrdinal?: number,
  ): { readonly ordinal: number; readonly attrs: EdgeAttributes };
  /** Releases the key of an edge retired earlier in the same program, so a later line may
   * declare that (from, to, ordinal) again. The host calls it for each emitted `RetireEdge`. */
  release?(edge: EpochAddressDomain["edge"]): void;
}

/** Creates an isolated resolver so one program's reservations never leak across executions. */
export interface AuthoringEdgeResolverFactory {
  /** Binds edge defaults to one execution. */
  create(options: CodeExecutionOptions): AuthoringEdgeResolver;
}

/** Pinned authored-option interpreter that returns the full existing protocol attribute payload. */
export interface AuthoringEdgePolicyResolver {
  /** Resolves omitted and explicit options without relying on core hard-coded defaults. */
  resolve(options: EdgeOptions): EdgeAttributes;
}

/** Inputs for the per-execution edge resolver. */
export interface AuthoringEdgeResolverFactoryOptions {
  /** Environment-pinned default and option interpreter. */
  readonly policies: AuthoringEdgePolicyResolver;
}

/** Creates the per-execution edge resolver over a caller-supplied pinned policy interpreter.
 * An omitted ordinal is 0 of its (from, to) pair; the current graph never shifts it. */
export declare function createAuthoringEdgeResolverFactory(
  options: AuthoringEdgeResolverFactoryOptions,
): AuthoringEdgeResolverFactory;

/** Fixed virtual-module values supplied while one prepared program is evaluated. */
export interface AuthoringVirtualModules {
  /** Complete snapshot-backed lookup, or null when the program did not request current state. */
  readonly current: CurrentLookupNamespace | null;
}

/** Synchronous evaluator implemented by an outer CLI, MCP code tool, or embedded V8 adapter. */
export interface AuthoringProgramEvaluator {
  /** Evaluates one already prepared module; top-level await and post-return SDK calls are forbidden. */
  evaluate(program: PreparedProgram, modules: AuthoringVirtualModules): void;
}

/** Dependencies that give a code host its only live authored-state write path. */
export interface CodeExecutionHostDependencies {
  /** The established session whose declaration partition carries all mutations. */
  readonly session: Pick<Session, "declarations">;
  /** The resolver used only when a prepared program imports the current virtual module. */
  readonly currentProjectResolver: CurrentProjectResolver;
  /** Catalog constructor resolver used by the core package. */
  readonly actors: AuthoringActorResolver;
  /** Per-execution edge/default/ordinal resolver used by the core package. */
  readonly edges: AuthoringEdgeResolverFactory;
  /** Outer sandbox or loader that evaluates the submitted prepared program exactly once. */
  readonly evaluator: AuthoringProgramEvaluator;
}

/** Executes each submitted program through one host-owned implicit declaration epoch. */
export interface CodeExecutionHost {
  /** Begins, evaluates once, drains exact emitted commands, validates and commits; failures abort. */
  execute(
    program: PreparedProgram,
    options: CodeExecutionOptions,
  ): Promise<CodeExecutionResult>;
}

/** Creates the host adapter used outside authored code by a CLI, MCP code tool, or embedded runtime. */
export declare function createCodeExecutionHost(
  dependencies: CodeExecutionHostDependencies,
): CodeExecutionHost;

/** A logical content-addressed reference whose physical encoding remains contract-versioned. */
export interface ContentAddressedRef<Kind extends string> {
  /** The referenced artifact family. */
  readonly kind: Kind;
  /** The verified content digest of the referenced artifact bytes. */
  readonly digest: Digest;
}

/** The installed five-seam host. Omitted `policies` means the product default. */
export interface InstalledCodeExecutionHostDependencies {
  readonly session: import('@circular/client').OwnerLocalSession;
  readonly policies?: AuthoringEdgePolicyResolver;
  readonly profile: SemanticPrepassProfile;
  readonly pageLimit?: number;
}
/** Config pair; admissionPath distinguishes a measured admission from the container catalog fallback. */
export interface AdmissionConfigPair {
  readonly module: string;
  readonly binding: string;
  readonly actorType: string;
  readonly authored: unknown;
  readonly admitted: unknown;
  readonly equal: boolean;
  readonly admissionPath?: 'actor.create-admission' | 'catalog';
  readonly diagnostics?: readonly Diagnostic[];
}
/** Prepares, queries the daemon, then executes through the existing one-epoch host. */
export declare function createCodeExecutionHost(dependencies: InstalledCodeExecutionHostDependencies): {
  execute(program: SourceProgramBundle | PreparedProgram, options: CodeExecutionOptions): Promise<CodeExecutionResult & { readonly admissionPairs?: readonly AdmissionConfigPair[]; readonly declarations?: { readonly path: string; readonly text: string; readonly modules?: ReadonlyMap<string, string> } }>;
};
/** Generates anchor-specific named exports with structural PortFlow hints.
 * Stream/Signal rates are omitted; connection validity remains the daemon Shape layer.
 * Catalog/admission ports without flow retain CircularValue. */
export declare function generateCurrentModule(index: CurrentSnapshotIndex): CurrentProjectModule & {
  readonly text: string;
  readonly declarations: string;
  readonly exports: Readonly<Record<string, unknown>>;
};
