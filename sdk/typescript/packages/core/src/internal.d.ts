import type {
  AuthoringRevision,
  CircularValue,
  DeclarationContentCommand,
  EdgeId,
  EdgeAttributes,
  EdgeDeclaration,
  EpochAddressDomain,
  EpochAnnotationReference,
  EpochEdgeReference,
  EpochExportReference,
  EpochActorReference,
  EpochScopeReference,
  ActorDeclaration,
  ActorId,
  PortEndpoint,
  PortId,
  Presentation as ProtocolPresentation,
} from "@circular/protocol";
import type { PublicActorSpelling } from "./config.js";
import type {
  CurrentActorHandle,
  ActorHandle,
} from "./handles.js";
import type { CurrentEdgeHandle } from "./wiring.js";

/** Symbolic reference families allocated inside one declaration epoch. */
export type ExecutionReferenceKind = "actor" | "edge" | "scope" | "export" | "annotation";

/** Maps allocator families to their existing protocol reference carriers. */
export interface ExecutionReferenceMap {
  readonly actor: EpochActorReference;
  readonly edge: EpochEdgeReference;
  readonly scope: EpochScopeReference;
  readonly export: EpochExportReference;
  readonly annotation: EpochAnnotationReference;
}

/** Port lookup returned by the pinned provider/spec resolver. */
export interface ResolvedActorPorts {
  readonly defaultInput?: PortId | null;
  readonly defaultOutput?: PortId | null;
  readonly inputs: readonly { readonly id: PortId; readonly primary: boolean; readonly flow?: CircularValue }[];
  readonly outputs: readonly { readonly id: PortId; readonly primary: boolean; readonly flow?: CircularValue }[];
  readonly input?: (name: string) => PortId | undefined;
  readonly output?: (name: string) => PortId | undefined;
}

/** Complete result of resolving one public constructor under the pinned environment. */
export interface ResolvedAuthoredActor {
  readonly declaration: ActorDeclaration;
  readonly ports: ResolvedActorPorts;
}

/** Snapshot-bound metadata needed to create a current actor handle without an RPC read. */
export interface CurrentActorDescriptor<Name extends PublicActorSpelling = PublicActorSpelling> {
  readonly actor: ActorId;
  readonly spelling: Name;
  readonly declaration: ActorDeclaration;
  readonly ports: ResolvedActorPorts;
  /** The export mounts that name this actor in any role; `remove()` retires them with it. */
  readonly mounts?: readonly EpochAddressDomain["exportMount"][];
  readonly revision: AuthoringRevision;
  readonly writable?: boolean;
}

/** Snapshot-bound metadata needed to create a current edge handle without an RPC read. */
export interface CurrentEdgeDescriptor {
  readonly edge: EdgeId;
  readonly declaration: EdgeDeclaration<EpochAddressDomain>;
}

/**
 * Host-only synchronous lowering context. It is not part of the `@circular/core`
 * public package entry point and must be installed by an execution adapter.
 */
export interface CircularExecutionContext {
  allocateReference<Kind extends ExecutionReferenceKind>(
    kind: Kind,
  ): ExecutionReferenceMap[Kind];
  emit(command: DeclarationContentCommand<EpochAddressDomain>): void;
  resolveActor(
    spelling: PublicActorSpelling,
    authoredConfig: unknown,
  ): ResolvedAuthoredActor;
  resolveEdge(
    from: PortEndpoint<EpochAddressDomain>,
    to: PortEndpoint<EpochAddressDomain>,
    options: unknown,
    existingOrdinal?: number,
  ): { readonly ordinal: number; readonly attrs: EdgeAttributes };
  /**
   * The presentation the owner holds in the fold as this program's verbs leave it, or null when it has
   * none. A SetPresentation lays the axes the program said over it.
   */
  presentationOf?(owner: EpochAddressDomain["presentationOwner"]): ProtocolPresentation<EpochAddressDomain> | null;
  /** Accumulates one role of a roles-only mount; the host emits complete Export values at program end. */
  declareExportMount?(binding: {
    readonly name: string;
    readonly role: "request" | "progress" | "result" | "error";
    readonly endpoint: PortEndpoint<EpochAddressDomain>;
  }): void;
  declareExportSurface?(binding: { readonly name: string; readonly surface: import("@circular/protocol").CircularValue }): void;
  /** Resolves shorthand only when a semantic prepass supplied stable mount identity. */
  claimFixedExportRole?(claim: {
    readonly role: "request" | "progress" | "result" | "error";
    readonly endpoint: PortEndpoint<EpochAddressDomain>;
    readonly actor: EpochAddressDomain["actor"];
  }):
    | DeclarationContentCommand<EpochAddressDomain>
    | readonly DeclarationContentCommand<EpochAddressDomain>[];
  resolveCurrentActor?(
    address: unknown,
    spelling?: PublicActorSpelling,
  ): CurrentActorDescriptor | undefined;
  resolveCurrentEdge?(address: unknown): CurrentEdgeDescriptor | undefined;
}

/** Runs a strictly synchronous authored callback inside one nested-safe execution context. */
export declare function runWithExecutionContext<Result>(
  context: CircularExecutionContext,
  callback: () => Result,
): Result;

/** Returns the active host-only lowering context. */
export declare function getExecutionContext(): CircularExecutionContext;

/** Synchronously forwards one existing declaration content command. */
export declare function emitDeclaration(
  command: DeclarationContentCommand<EpochAddressDomain>,
): void;

/** Allocates one epoch-local symbolic reference. */
export declare function allocateReference<Kind extends ExecutionReferenceKind>(
  kind: Kind,
): ExecutionReferenceMap[Kind];

/** Returns the exact protocol endpoint represented by a core handle. */
export declare function endpointOf(
  endpoint: unknown,
  direction?: "source" | "target" | "writable",
): PortEndpoint<EpochAddressDomain>;

/** Returns the stable or epoch-local actor address represented by a handle. */
export declare function actorOf(handle: ActorHandle): EpochAddressDomain["actor"];

/** Creates a snapshot-bound current actor handle. */
export declare function createCurrentActorHandle<Name extends PublicActorSpelling>(
  address: unknown | CurrentActorDescriptor<Name>,
  spelling?: Name,
): CurrentActorHandle<Name>;

/** Creates a snapshot-bound current edge handle. */
export declare function createCurrentEdgeHandle(
  address: unknown | CurrentEdgeDescriptor,
): CurrentEdgeHandle;

/** Core-owned constructor provenance, shared by the pinned prepass. */
export declare const COMBINATOR_NAMES: typeof import("@circular/protocol/tables").PreprocessKind;
export declare function constructorSpelling(value: unknown): string | undefined;
/** The SDK (camelCase) name a spelling is called by; the spelling itself stays the actor type key. */
export declare function publicName(spelling: string): string;

/** The fold row a keyed content verb writes and the field that names it; null for a verb that writes none. */
export declare function declarationRow(kind: string): { readonly table: string; readonly field: string } | null;

export declare function joinConfigIssue(config: unknown): string | null;
export declare function assembleConfigIssue(config: unknown): string | null;
