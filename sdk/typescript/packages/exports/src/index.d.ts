/**
 * Fixed-role export surface builders for the installed Circular authoring host; no standalone mount export.
 *
 * @packageDocumentation
 */

import type {
  CircularValue,
  ExportName,
  OperationDeclaration,
} from "@circular/protocol";
import type {
  BoundaryTypeExpression,
  ObservedEndpoint,
  StaticViewSelection,
  WritableBoundaryEndpoint,
} from "@circular/core";

declare const exportRoleReferenceBrand: unique symbol;
declare const exportDefinitionBrand: unique symbol;
declare const exportInstanceBrand: unique symbol;
declare const exportMountBrand: unique symbol;
declare const staticReferenceBrand: unique symbol;
declare const structuralBindingBrand: unique symbol;

/** A symbolic value supplied only while an allowlisted structural builder is evaluated. */
export interface StaticReference<Value> {
  /** Carries the referenced value type without exposing a constructible runtime representation. */
  readonly [staticReferenceBrand]: Value;
}

/** A literal or symbolic value accepted by a static surface field. */
export type StaticValue<Value> = Value | StaticReference<Value>;

/** A symbolic binding that a static surface mark may read from its projection context. */
export interface StructuralBinding<Value = unknown> {
  /** Carries the projected value type without exposing a constructible runtime representation. */
  readonly [structuralBindingBrand]: Value;
}

/** A non-negative grid origin and positive grid span authored with `.cell(...)`. */
export interface CellPlacement {
  /** Zero-based grid column. */
  readonly col: number;
  /** Zero-based grid row. */
  readonly row: number;
  /** Positive grid-column span. */
  readonly width: number;
  /** Positive grid-row span. */
  readonly height: number;
}

/** Common immutable surface implemented by every authored surface mark. */
export interface SurfaceMark<Kind extends string = string> extends StaticViewSelection {
  /** Stable allowlisted mark discriminator consumed by the structural host. */
  readonly mark: Kind;

  /** Returns a copy placed at one grid cell rectangle. */
  cell(col: number, row: number, width: number, height: number): this;
}

/** A closed declaration for one primitive static surface parameter. */
export type SurfaceParameterDeclaration =
  | Readonly<{ type: "number"; default?: number }>
  | Readonly<{ type: "string"; default?: string }>
  | Readonly<{ type: "boolean"; default?: boolean }>;

/** A name-indexed, value-closed parameter schema inferred from a definition literal. */
export type SurfaceParameterSchema = Readonly<Record<string, SurfaceParameterDeclaration>>;

/** Resolves one parameter declaration to the value accepted by a surface instance. */
export type SurfaceParameterValue<Declaration extends SurfaceParameterDeclaration> =
  Declaration extends Readonly<{ type: "number" }>
    ? number
    : Declaration extends Readonly<{ type: "string" }>
      ? string
      : boolean;

/** The configuration accepted by a parameterized static definition. */
export type SurfaceConfiguration<Schema extends SurfaceParameterSchema> = Readonly<{
  [Name in keyof Schema]?: SurfaceParameterValue<Schema[Name]>;
}>;

/** Symbolic references exposed only to a structural definition callback. */
export type SurfaceParameterReferences<Schema extends SurfaceParameterSchema> = Readonly<{
  [Name in keyof Schema]: StaticReference<SurfaceParameterValue<Schema[Name]>>;
}>;

/** The complete and closed export-role vocabulary. */
export type ExportRole = "request" | "progress" | "result" | "error";

/** The three export roles that observe data and cannot accept injection. */
export type ObservedExportRole = Exclude<ExportRole, "request">;

/** Declares the value domain of one present fixed export role without restating its direction. */
export interface ExportRoleDeclaration {
  /** The boundary type checked against the endpoint bound by `mount`. */
  readonly type: BoundaryTypeExpression;
}

/** A symbolic reference usable only while a static export surface is built. */
export interface ExportRoleReference<
  Role extends ExportRole = ExportRole,
  Value extends CircularValue = CircularValue,
> extends StructuralBinding<Value> {
  /** The fixed role represented by this symbolic reference. */
  readonly role: Role;
  /** Prevents construction outside the authoring host. */
  readonly [exportRoleReferenceBrand]: Role;
}

/** The symbolic writable role accepted by native request controls. */
export type RequestRoleReference<Value extends CircularValue = CircularValue> =
  ExportRoleReference<"request", Value>;

/** A symbolic read-only role accepted by observed export marks. */
export type ObservedRoleReference<
  Role extends ObservedExportRole = ObservedExportRole,
  Value extends CircularValue = CircularValue,
> = ExportRoleReference<Role, Value>;

/** A closed partial record selecting which fixed roles an export definition uses. */
export interface ExportRoleSelection {
  /** Declares the externally writable ingress-boundary role. */
  readonly request?: ExportRoleDeclaration;
  /** Declares the observed in-flight progress role. */
  readonly progress?: ExportRoleDeclaration;
  /** Declares the observed terminal-success role. */
  readonly result?: ExportRoleDeclaration;
  /** Declares the observed terminal-failure role. */
  readonly error?: ExportRoleDeclaration;
}

type ClosedRoleSelection<Selection extends ExportRoleSelection> = Selection & Readonly<{
  [Name in Exclude<keyof Selection, ExportRole>]?: never;
}>;

type SelectedRoleName<Selection extends ExportRoleSelection> = {
  [Name in keyof Selection]-?: Exclude<Selection[Name], undefined> extends ExportRoleDeclaration
    ? Name
    : never;
}[keyof Selection] & ExportRole;

type ExportRoleValue<
  Selection extends ExportRoleSelection,
  Role extends SelectedRoleName<Selection>,
> = Role extends keyof Selection
  ? Exclude<Selection[Role], undefined> extends { readonly type: "text" }
    ? string
    : CircularValue
  : CircularValue;

/** Symbolic references exposed to an export's allowlisted structural surface callback. */
export type ExportRoleReferences<Selection extends ExportRoleSelection> = Readonly<{
  [Role in SelectedRoleName<Selection>]: ExportRoleReference<Role, ExportRoleValue<Selection, Role>>;
}>;

/** Direction-safe endpoint bindings required by one selected fixed-role record. */
export type ExportRoleBindings<Selection extends ExportRoleSelection> = Readonly<{
  [Role in SelectedRoleName<Selection>]: Role extends "request"
    ? WritableBoundaryEndpoint<ExportRoleValue<Selection, Role>>
    : ObservedEndpoint<ExportRoleValue<Selection, Role>>;
}>;

/** Common immutable surface implemented by every export-specific mark. */
export interface ExportMark<Kind extends string = string> extends SurfaceMark<Kind> {}

/** A titled top-level tab surface. */
export interface TabSurface extends ExportMark<"tab"> {}

/** A titled top-level window surface. */
export interface WindowSurface extends ExportMark<"window"> {}

/** Static grid configuration for an export surface. */
export interface GridSpec {
  /** Positive column count or a symbolic static parameter. */
  readonly cols?: StaticValue<number>;
  /** Positive row height or a symbolic static parameter. */
  readonly row_h?: StaticValue<number>;
}

/** An immutable grid layout used inside a top-level export surface. */
export interface GridMark extends ExportMark<"grid"> {}

/** An immutable observed message-stream mark. */
export interface MessagesMark extends ExportMark<"messages"> {}

/** An immutable observed terminal-stream mark. */
export interface TerminalMark extends ExportMark<"terminal"> {}

/** An immutable literal label mark. */
export interface LabelMark extends ExportMark<"label"> {}

/** An immutable observed transcript mark. */
export interface TranscriptMark extends ExportMark<"transcript"> {}

/** An immutable writable text control. */
export interface TextInputMark extends ExportMark<"textInput"> {
  /** Returns a copy with placeholder text. */
  placeholder(value: StaticValue<string>): TextInputMark;
}

/** Static options for a writable button control. */
export interface ButtonSpec {
  /** Optional visible button label. */
  readonly label?: StaticValue<string>;
  /** Optional literal payload emitted by activation. */
  readonly send?: StaticValue<string>;
}

/** An immutable writable button control. */
export interface ButtonMark extends ExportMark<"button"> {}

/** Static options for a writable toggle control. */
export interface ToggleSpec {
  /** Optional payload binding used to display current state. */
  readonly bind?: string;
  /** Optional visible toggle label. */
  readonly label?: StaticValue<string>;
}

/** An immutable writable toggle control. */
export interface ToggleMark extends ExportMark<"toggle"> {}

/** Static options for a writable selection control. */
export interface SelectSpec {
  /** Non-empty list of literal values that the control may emit. */
  readonly options: readonly [string, ...string[]];
  /** Optional visible control label. */
  readonly label?: StaticValue<string>;
}

/** An immutable writable selection control. */
export interface SelectMark extends ExportMark<"select"> {}

/** Static options for the compound prompt-and-submit control. */
export interface ComposerSpec {
  /** Optional prompt placeholder. */
  readonly placeholder?: StaticValue<string>;
  /** Optional submit-button label. */
  readonly label?: StaticValue<string>;
}

/** An immutable compound prompt-and-submit control. */
export interface ComposerMark extends ExportMark<"composer"> {}

/** Creates a titled top-level tab around one immutable static child tree. */
export declare function tab(title: StaticValue<string>, child: SurfaceMark): TabSurface;

/** Creates a titled top-level window around one immutable static child tree. */
export declare function window(title: StaticValue<string>, child: SurfaceMark): WindowSurface;

/** Creates an empty immutable grid layout. */
export declare function grid(spec?: GridSpec): GridMark;

/** Creates an immutable grid layout containing the supplied static children. */
export declare function grid(spec: GridSpec, children: readonly SurfaceMark[]): GridMark;

/** Creates an immutable grid layout with default configuration. */
export declare function grid(children: readonly SurfaceMark[]): GridMark;

/** Creates a read-only message-stream mark bound to one observed fixed role. */
export declare function messages(role: ObservedRoleReference): MessagesMark;

/** Creates a read-only terminal-stream mark bound to one observed fixed role. */
export declare function terminal(role: ObservedRoleReference): TerminalMark;

/** Creates a static literal label mark. */
export declare function label(value: StaticValue<string>): LabelMark;

/** Creates a read-only transcript, optionally including a second observed stream as the local side. */
export declare function transcript(
  role: ObservedRoleReference,
  own?: ObservedRoleReference,
): TranscriptMark;

/** Creates a writable text control bound only to the fixed request role. */
export declare function textInput(role: RequestRoleReference): TextInputMark;

/** Creates a writable button bound only to the fixed request role. */
export declare function button(role: RequestRoleReference, spec?: ButtonSpec): ButtonMark;

/** Creates a writable toggle bound only to the fixed request role. */
export declare function toggle(role: RequestRoleReference, spec?: ToggleSpec): ToggleMark;

/** Creates a writable closed-choice control bound only to the fixed request role. */
export declare function select(role: RequestRoleReference, spec: SelectSpec): SelectMark;

/** Creates a writable prompt control bound only to the fixed request role. */
export declare function prompt(
  role: RequestRoleReference,
  placeholder?: StaticValue<string>,
): TextInputMark;

/** Creates a writable compound prompt-and-submit control bound only to the fixed request role. */
export declare function composer(role: RequestRoleReference, spec?: ComposerSpec): ComposerMark;

/** A non-empty list of top-level static export surfaces. */
export type ExportSurfaceList = readonly [TabSurface | WindowSurface, ...(TabSurface | WindowSurface)[]];

/** Static source description accepted by `defineExport`. */
export interface ExportDefinitionSpec<
  Selection extends ExportRoleSelection,
  Parameters extends SurfaceParameterSchema,
> {
  /** Closed partial selection of the four fixed roles; direction is intrinsic to each key. */
  readonly roles?: ClosedRoleSelection<Selection>;
  /** Optional closed schema for literal surface parameters. */
  readonly params?: Parameters;
  /** Optional opaque operation declaration owned by the pinned authoring environment. */
  readonly operations?: OperationDeclaration;
  /**
   * An allowlisted surface list or a structural callback over symbolic role and parameter references.
   * It is not a dataflow function and receives no runtime value or capability.
   */
  readonly surfaces:
    | ExportSurfaceList
    | ((
      roles: ExportRoleReferences<Selection>,
      parameters: SurfaceParameterReferences<Parameters>,
    ) => ExportSurfaceList);
}

/** A configured occurrence of one static export definition. */
export interface ExportInstance<
  Selection extends ExportRoleSelection = ExportRoleSelection,
  Parameters extends SurfaceParameterSchema = SurfaceParameterSchema,
> {
  /** Prevents construction outside the structural authoring host. */
  readonly [exportInstanceBrand]: true;
  /** Definition that owns this instance's fixed roles and parameter schema. */
  readonly definition: ExportDefinition<Selection, Parameters>;
  /** Literal configuration captured for this occurrence. */
  readonly config: SurfaceConfiguration<Parameters>;
}

/** A callable static export definition created by `defineExport`. */
export interface ExportDefinition<
  Selection extends ExportRoleSelection = ExportRoleSelection,
  Parameters extends SurfaceParameterSchema = SurfaceParameterSchema,
> {
  /** Creates an immutable occurrence after closed-schema validation. */
  (config?: SurfaceConfiguration<Parameters>): ExportInstance<Selection, Parameters>;
  /** Prevents construction outside the structural authoring host. */
  readonly [exportDefinitionBrand]: true;
  /** The exact fixed-role selection exposed to the structural surface callback. */
  readonly roles: Selection;
  /** The authored closed parameter schema. */
  readonly params: Parameters;
  /** The resulting immutable top-level surface list. */
  readonly surfaces: ExportSurfaceList;
}

/**
 * Raised before command emission while the existing export-mount payload cannot
 * losslessly carry the authored static surface tree.
 */
export declare function defineExport<
  const Selection extends ExportRoleSelection,
  const Parameters extends SurfaceParameterSchema = Readonly<Record<never, never>>,
>(spec: ExportDefinitionSpec<Selection, Parameters>): ExportDefinition<Selection, Parameters>;

/** A declaration handle for one explicitly named export mount. */
export interface ExportMountHandle {
  /** Prevents construction outside the authoring host. */
  readonly [exportMountBrand]: true;
  /** Stable validated name emitted in the export declaration command. */
  readonly name: ExportName;
}

/**
 * Attaches a surface to a name whose roles are bound through handle.mount in the installed authoring host.
 * This does not create role bindings or provide a standalone mount function.
 */
export declare function surface<Selection extends ExportRoleSelection, Parameters extends SurfaceParameterSchema>(
  name: string,
  definition: ExportDefinition<Selection, Parameters>,
): void;
