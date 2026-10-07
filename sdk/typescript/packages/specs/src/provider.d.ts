import type {
  ConstructorExportName,
  ModuleSpecifierOrigin,
  ActorTypeId,
  ActorTypeName,
} from "./model.js";

/** Maps a daemon catalog type name to its SDK constructor spelling. */
export interface ConstructorBinding<Name extends ActorTypeName = ActorTypeName> {
  /** Names the daemon-owned catalog type. */
  readonly actorTypeId: ActorTypeId & Name;
  /** Records the resolved module provenance. */
  readonly importSpecifier: ModuleSpecifierOrigin;
  /** Names the SDK module's constructor export. */
  readonly constructorExport: ConstructorExportName;
}

/** Reports one successfully resolved constructor provenance binding. */
export interface ResolvedConstructorBinding<Name extends ActorTypeName = ActorTypeName> {
  readonly status: "resolved";
  readonly binding: ConstructorBinding<Name>;
}

/** Reports a binding failure without guessing by constructor text. */
export interface RejectedConstructorBinding {
  readonly status: "rejected";
  readonly diagnostics: readonly ProviderBindingDiagnostic[];
}

/** Describes one constructor-binding diagnostic. */
export interface ProviderBindingDiagnostic {
  readonly code: string;
  readonly message: string;
}

export type ConstructorBindingResult<Name extends ActorTypeName = ActorTypeName> =
  | ResolvedConstructorBinding<Name>
  | RejectedConstructorBinding;

/** Resolves source imports and canonical SDK spellings for catalog type names. */
export interface ProviderBindingRegistry {
  /** Resolves a module/export pair without consulting config, labels, or local aliases. */
  resolve<Name extends ActorTypeName>(request: {
    readonly importSpecifier: ModuleSpecifierOrigin;
    readonly constructorExport: ConstructorExportName;
    readonly expectedActorType: Name;
  }): ConstructorBindingResult<Name>;
  /** Finds the canonical constructor binding used by deterministic reification. */
  canonical<Name extends ActorTypeName>(actorType: Name): ConstructorBindingResult<Name>;
}

/** Complete immutable input for one constructor-binding registry. */
export interface ProviderBindingRegistryInput {
  readonly bindings: readonly ConstructorBinding[];
}

/** Creates immutable catalog-type to SDK-constructor lookup. */
export declare function createProviderBindingRegistry(
  input: ProviderBindingRegistryInput,
): ProviderBindingRegistry;
