/** ExactPayloadPath, as in join.at and keyed_reduce.value: String keys or nonnegative Int indices. */
export type ViewFieldPath = readonly (string | bigint)[];

/** The closed semantic roles within a recorded body. */
export type ViewFieldRole = "title" | "status" | "value";

/** Selects an already recorded outlet value; this does not compute a sum or count. */
export interface ViewTotal {
  /** The declared outlet id; its existing port label supplies the display name. */
  readonly outlet: string;
  /** A path within that outlet's recorded body; [] selects the whole body. */
  readonly path: ViewFieldPath;
}

/**
 * The keys whose value is one closed String choice, each with its value space.
 * `validateViewConfig` and the `ViewVocabulary` member types read this one table.
 */
export declare const VIEW_CHOICES: {
  readonly side: readonly ["emitted", "arrivals", "both"];
  readonly rows: readonly ["latest", "outlets"];
  readonly spark: readonly ["none", "samples"];
};

/** One vocabulary for type defaults and actor presentation.view.config overrides. */
export interface ViewVocabulary {
  /** The card body heading, including a table's title; distinct from actor.label(). */
  readonly heading?: string | null;
  /** Text accompanying an observed count; it does not select or compute the count. */
  readonly count_label?: string | null;
  /** Ordered column projection; paths are unique and relative to each row. Null restores derivation. */
  readonly columns?: readonly { readonly path: ViewFieldPath; readonly label: string }[] | null;
  /** Authored caption, without replacing the type's description. */
  readonly caption?: string | null;
  /** An observed total selected by outlet and body path. */
  readonly total?: ViewTotal | null;
  /** Body-relative field paths; the entire role map replaces the default map. */
  readonly fields?: Readonly<Partial<Record<ViewFieldRole, ViewFieldPath>>> | null;
  /** Which recorded rows the view reads: this actor's emissions, its arrivals, or both. */
  readonly side?: (typeof VIEW_CHOICES.side)[number] | null;
  /**
   * Whether the rows are the latest emission's entries or the actor's declared outlets.
   * An outlet row shows that outlet's latest emission, its time and whether the outlet is wired;
   * it does not show a count of rows in the screen window.
   */
  readonly rows?: (typeof VIEW_CHOICES.rows)[number] | null;
  /** Whether the view draws the sparkline of its inlet samples. */
  readonly spark?: (typeof VIEW_CHOICES.spark)[number] | null;
}

/** The keys shared by every supply point; view-specific keys remain owned by that view. */
export type ViewVocabularyKey = keyof ViewVocabulary;

/** A config record may also contain opaque data owned by the selected view. */
export interface ViewConfig extends ViewVocabulary {
  readonly [key: string]: unknown;
}

/** Non-record Values carry no vocabulary keys; they remain available to their view. */
export type ViewConfigValue = ViewConfig | null | boolean | number | bigint | string
  | Uint8Array | readonly unknown[];

/** Type-level config slot text, carried directly on each actor.create-inputs slot. */
export interface ConfigSlotMetadata {
  /** Human-readable slot name, or null when undeclared. */
  readonly label: string | null;
  /** Human-readable help, or null when undeclared. */
  readonly description: string | null;
  /** Human-readable form group, or null when undeclared; not an actor layout group. */
  readonly group: string | null;
}

/**
 * Validates shared keys without interpreting the view kind or view-specific data.
 * Throws TypeError with code CIRCULAR_VIEW_CONFIG_INVALID through the SDK's coded exception path.
 */
export declare function validateViewConfig(config: unknown): void;

/**
 * Resolves top-level keys: actor declaration, then type default, then absence.
 * An own null removes the key, including an inherited default; omission inherits.
 * Arrays and nested records replace as a whole, without concatenation or deep merging.
 * Non-record configs have no vocabulary keys. Inputs are not mutated.
 */
export declare function resolveViewConfig(actorConfig?: unknown, typeDefault?: unknown): ViewConfig;
