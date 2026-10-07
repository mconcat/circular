import type { ViewConfigValue } from "./view-config.js";

declare const layoutActorReferenceBrand: unique symbol;
declare const staticViewSelectionBrand: unique symbol;
declare const viewKindBrand: unique symbol;

/** Identifies an actor that may participate in authored relative layout constraints. */
export interface LayoutActorReference {
  /** Prevents arbitrary application objects from masquerading as authored actors. */
  readonly [layoutActorReferenceBrand]: true;
}

/** Marks an immutable static view value; `@circular/exports` `SurfaceMark` is the one that exists. */
export interface StaticViewSelection {
  /** Separates static view values from arbitrary host objects. */
  readonly [staticViewSelectionBrand]: true;
}

/** Names one canonical GUI view kind without granting renderer authority. */
export type ViewKind = string & { readonly [viewKindBrand]: "ViewKind" };

/** Selects a source-level view name or an immutable authored static view value. */
export type ViewSelection = string | StaticViewSelection;

/** Describes the three closed live actor switches. */
export interface ActorFlags {
  /** Bypasses the actor's calculation through generated default ports. */
  readonly bypass: boolean;
  /** Prevents the actor's emissions from entering the router. */
  readonly mute: boolean;
  /** Prevents the actor from consuming new input. */
  readonly pause: boolean;
}

/**
 * Describes an authored `fixed` position in layout units, the unit the canvas shows.
 * Each component is an integer, and the wire carries that same integer.
 */
export interface AuthoredPoint {
  /** Holds the horizontal coordinate, positive to the right. */
  readonly x: number;
  /** Holds the vertical coordinate, positive downward. */
  readonly y: number;
}

/**
 * Describes an authored actor size in layout units, the unit of `AuthoredPoint`.
 * Each component is a non-negative integer, and the wire carries that same integer.
 */
export interface AuthoredSize {
  /** Holds the non-negative authored width. */
  readonly w: number;
  /** Holds the non-negative authored height. */
  readonly h: number;
}

/** Describes an authored grid-board placement and extent. */
export interface BoardPlacement {
  /** Holds the unsigned starting column. */
  readonly col: number;
  /** Holds the unsigned starting row. */
  readonly row: number;
  /** Holds the non-negative column span. */
  readonly w: number;
  /** Holds the non-negative row span. */
  readonly h: number;
}

/** Names a rank relationship, independent of screen direction. */
export type RelativeRelation = "before" | "after";

/** Names one of the two wire alignment axes. */
export type AlignmentRelation = "align-horizontal" | "align-vertical";

/** Describes one authored relative layout anchor. */
export type AuthoredAnchor =
  | {
      /** Discriminates a rank relationship. */
      readonly kind: "relative";
      /** Identifies the referenced actor in the same authored scope. */
      readonly target: LayoutActorReference;
      /** Names the rank relationship. */
      readonly relation: RelativeRelation;
      /** Refused: gap is not part of the canonical layout hints. */
      readonly gap?: number;
    }
  | {
      /** Discriminates an alignment relationship. */
      readonly kind: "align";
      /** Identifies the referenced actor in the same authored scope. */
      readonly target: LayoutActorReference;
      /** Names the alignment axis. */
      readonly relation: AlignmentRelation;
    };

/** Describes one complete authored actor presentation replacement. */
export interface Presentation {
  /** Holds the optional display label. */
  readonly label?: string;
  /** Holds the optional canonical group name. */
  readonly group?: string;
  /** Holds the optional relative anchor. */
  readonly anchor?: AuthoredAnchor;
  /** Holds the optional authored absolute point. */
  readonly fixed?: AuthoredPoint;
  /** Holds the optional explicit authored size. */
  readonly size?: AuthoredSize;
  /** Holds the optional board placement and extent. */
  readonly board?: BoardPlacement;
  /** Holds the optional authored view selection. */
  readonly view?: { readonly kind: string; readonly config: ViewConfigValue } | null;
  /** Holds the complete collapse intent. */
  readonly collapsed: boolean;
}

/** Provides immutable fluent presentation and flag operations. */
export interface ActorAuthoringSurface {
  /** Sets the actor's authored display label. */
  label(text: string): this;
  /**
   * Sets an authored absolute position in layout units, the unit the canvas shows.
   * A component that is not an integer is refused with CIRCULAR_LAYOUT_COORD_UNREPRESENTABLE.
   */
  at(x: number, y: number): this;
  /**
   * Sets an explicit authored actor size in layout units, the unit `at` uses.
   * A component that is not a non-negative integer is refused with CIRCULAR_LAYOUT_COORD_UNREPRESENTABLE.
   */
  size(width: number, height: number): this;
  /** Places the actor in a grid board without selecting an export surface. */
  board(col: number, row: number, width: number, height: number): this;
  /** Assigns the actor to one canonical presentation group. */
  group(name: string): this;
  /** Carries shared vocabulary and view-owned data; a null key removes its type default. */
  view(kind: string, config?: ViewConfigValue): this;
  /** Sets the complete authored collapse intent. */
  collapsed(value: boolean): this;
  /** Places this actor before a same-scope actor in rank order; gap is refused. */
  before(target: LayoutActorReference, gap?: number): this;
  /** Places this actor after a same-scope actor in rank order; gap is refused. */
  after(target: LayoutActorReference, gap?: number): this;
  /** Emits the wire Horizontal alignment with a same-scope actor. */
  alignHorizontal(target: LayoutActorReference): this;
  /** Emits the wire Vertical alignment with a same-scope actor. */
  alignVertical(target: LayoutActorReference): this;
  /** Replaces all three closed live actor flags. */
  setFlags(flags: ActorFlags): this;
}

/** Describes shared options for row and column layout groups. */
export interface LinearFlowOptions {
  /** Sets the non-negative logical gap between adjacent actors. */
  readonly gap?: number;
}

/** Describes shared options for a grid layout group. */
export interface GridFlowOptions {
  /** Sets both row and column gaps when a specific axis is omitted. */
  readonly gap?: number;
  /** Sets the non-negative logical gap between rows. */
  readonly row_gap?: number;
  /** Sets the non-negative logical gap between columns. */
  readonly col_gap?: number;
}

/** Provides authored group-level relative layout constraints. */
export interface FlowLayout {
  /** Constrains a non-empty actor list from left to right and returns the same list. */
  row<const Actors extends readonly [LayoutActorReference, ...LayoutActorReference[]]>(
    actors: Actors,
    options?: LinearFlowOptions,
  ): Actors;
  /** Constrains a non-empty actor list from top to bottom and returns the same list. */
  column<const Actors extends readonly [LayoutActorReference, ...LayoutActorReference[]]>(
    actors: Actors,
    options?: LinearFlowOptions,
  ): Actors;
  /** Constrains a non-empty matrix of non-empty actor rows and returns the same matrix. */
  grid<
    const Rows extends readonly [
      readonly [LayoutActorReference, ...LayoutActorReference[]],
      ...(readonly [LayoutActorReference, ...LayoutActorReference[]])[],
    ],
  >(rows: Rows, options?: GridFlowOptions): Rows;
}

/** Emits authored row, column, and grid constraints without running a layout solver. */
export declare const flow: FlowLayout;
