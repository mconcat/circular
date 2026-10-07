/** @packageDocumentation Compiler-free SDK program reconstruction. */

import type { CircularValue, DiagnosticCode, EnvelopePath, NonEmptyReadonlyArray, SourceSpan } from "@circular/protocol";
import type { ProviderBindingRegistry } from "@circular/specs";

declare const generatorNominal: unique symbol;
type Nominal<Base, Name extends string> = Base & { readonly [generatorNominal]: Name };

/** A normalized manifest-relative TypeScript module path. */
export type NormalizedModulePath = Nominal<string, "NormalizedModulePath">;

/** Immutable source bytes for one TypeScript module. */
export type SourceModuleBytes = Readonly<Uint8Array>;

/** A user or agent-authored TypeScript module bundle. */
export interface SourceProgramBundle {
  /** The manifest-relative entry module. */
  readonly entry: NormalizedModulePath;
  /** Every module byte sequence keyed by its normalized manifest-relative path. */
  readonly modules: ReadonlyMap<NormalizedModulePath, SourceModuleBytes>;
}

/** The five phases that may reject or warn during one mutation execution. */
export type AuthoringDiagnosticPhase = "Prepass" | "Host" | "Lowering" | "Admission" | "Commit";

/** Integrity phases used outside a writer mutation execution. */
export type IntegrityDiagnosticPhase = "Artifact";

/** Every phase represented by this package's structured diagnostics. */
export type SdkDiagnosticPhase = AuthoringDiagnosticPhase | IntegrityDiagnosticPhase;

/** The stable message-template key of one authoring diagnostic. */
export type DiagnosticMessageKey = Nominal<string, "DiagnosticMessageKey">;

/** A path to one field of one compacted declaration command. */
export interface CommandDiagnosticLocation {
  /** The location discriminant. */
  readonly kind: "Command";
  /** The zero-based position in the ordered compacted command list. */
  readonly commandIndex: number;
  /** The structural field path inside that existing command value. */
  readonly path: EnvelopePath | null;
}

/** A manifest or dependency location inside one module bundle. */
export interface BundleDiagnosticLocation {
  /** The location discriminant. */
  readonly kind: "Bundle";
  /** The normalized module associated with the diagnostic. */
  readonly module: NormalizedModulePath;
  /** The referenced module specifier when the diagnostic concerns a dependency. */
  readonly specifier: string | null;
}

/** A source, envelope, existing-command, or bundle location visible outside a host. */
export type AuthoringDiagnosticLocation =
  | { readonly kind: "Source"; readonly span: SourceSpan }
  | { readonly kind: "Envelope"; readonly path: EnvelopePath }
  | CommandDiagnosticLocation
  | BundleDiagnosticLocation;

/** One deterministic host diagnostic with a phase-appropriate public location. */
export interface SdkDiagnostic<Phase extends SdkDiagnosticPhase = SdkDiagnosticPhase> {
  /** The phase that owns the condition and its disposition. */
  readonly phase: Phase;
  /** Whether the condition blocks the whole operation or is an admitted warning. */
  readonly class: "Rejection" | "Warning";
  /** The registry-owned machine-readable diagnostic code. */
  readonly code: DiagnosticCode;
  /** The primary public source, envelope, command, or bundle location. */
  readonly primary: AuthoringDiagnosticLocation;
  /** Additional ordered locations needed to explain the same condition. */
  readonly related: readonly AuthoringDiagnosticLocation[];
  /** The stable message-template key, never a machine-decision input. */
  readonly message: DiagnosticMessageKey;
  /** First-order template arguments, never a substitute for the diagnostic code. */
  readonly args: readonly CircularValue[];
}

/** A diagnostic emitted while preparing, lowering, admitting, or committing a mutation program. */
export type AuthoringDiagnostic = SdkDiagnostic<AuthoringDiagnosticPhase>;

/** A successful local SDK operation with all admitted warnings preserved. */
export interface SdkOperationComplete<Value, DiagnosticValue extends SdkDiagnostic = SdkDiagnostic> {
  /** The successful completion discriminant. */
  readonly status: "complete";
  /** The operation's complete value. */
  readonly value: Value;
  /** Stable ordered warnings retained after successful completion. */
  readonly diagnostics: readonly DiagnosticValue[];
}

/** A rejected local SDK operation that never exposes a partial value. */
export interface SdkOperationRejected<DiagnosticValue extends SdkDiagnostic = SdkDiagnostic> {
  /** The rejected completion discriminant. */
  readonly status: "rejected";
  /** The non-empty stable ordered diagnostics that rejected the whole operation. */
  readonly diagnostics: NonEmptyReadonlyArray<DiagnosticValue>;
}

/** The normal complete-or-rejected result of a local SDK host operation. */
export type SdkOperationResult<Value, DiagnosticValue extends SdkDiagnostic = SdkDiagnostic> =
  | SdkOperationComplete<Value, DiagnosticValue>
  | SdkOperationRejected<DiagnosticValue>;

/** Pins constructor provenance to an SDK release and the daemon's spec_set. */
export interface StructureGeneratorOptions {
  readonly bindings: ProviderBindingRegistry;
  readonly sdkVersion: string;
  readonly specSet: Uint8Array;
}

/** Ordinary generated source, not a semantic-prepass PreparedProgram. */
export interface GeneratedStructure {
  readonly program: SourceProgramBundle;
  readonly pin: { readonly sdkVersion: string; readonly specSet: Uint8Array };
  /** Original admission envelope, if supplied; epoch IDs are rebound by the execution host. */
  readonly begin: import("@circular/protocol").BeginEpochCommand | null;
  /**
   * Every printed statement of `program`, in module order and then line order. Keys use the admission
   * key shape: the scope path, then the local name, joined by `/`.
   */
  readonly statements: readonly {
    /** The module that holds the statement. */
    readonly module: NormalizedModulePath;
    /** The one-based line of the statement in that module. */
    readonly line: number;
    /**
     * The declaration the statement is about. An actor owns its declaration, presentation, flags and
     * mount lines, and every wire that flows into it. A Note owns its declaration and presentation.
     * An export mount owns its surface. A template body's statements belong to the template.
     */
    readonly owner:
      | { readonly actor: string }
      | { readonly annotation: string }
      | { readonly mount: string }
      | { readonly template: string };
    /** Keys of the other actors the statement names: a wire's source, an anchor target, a Note's refs. */
    readonly refs: readonly string[];
  }[];
}

/** Product §7 generator. Scoped admission keys use scope/name; root keys use the binding name. */
export interface ProgramGeneratorOptions extends StructureGeneratorOptions {
  readonly catalog?: readonly import('@circular/protocol/actor-query').ActorCatalogItem[];
  readonly admissions?: ReadonlyMap<string, import('@circular/protocol/actor-query').ActorCreateAdmissionItem>;
}
/** Reconstructs a complete compacted log into main.ts and Concrete scope modules; unsupported commands identify their location. */
export declare function generateProgram(
  commands: readonly unknown[],
  options: ProgramGeneratorOptions,
): SdkOperationResult<GeneratedStructure, AuthoringDiagnostic>;

export declare const generatorEnvironment: Readonly<Pick<ProgramGeneratorOptions, 'sdkVersion' | 'bindings'>>;
