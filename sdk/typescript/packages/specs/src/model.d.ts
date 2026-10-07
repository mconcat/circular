export type { ActorTypeId } from "@circular/protocol";

declare const constructorExportNameBrand: unique symbol;
declare const unresolvedGeneratedBrand: unique symbol;

/** Names one exported constructor in a pinned physical provider module. */
export type ConstructorExportName = string & {
  readonly [constructorExportNameBrand]: "ConstructorExportName";
};

/**
 * Marks a generated registration aspect that v2 has not closed yet.
 *
 * This marker is intentionally not constructible by ordinary SDK consumers and must
 * never be widened to a free-form configuration object.
 */
export interface UnresolvedGeneratedRegistration<
  Name extends string,
  Aspect extends string,
> {
  /** Names the catalog entry whose generated registration remains incomplete. */
  readonly actorType: Name;
  /** Names the incomplete generated section without inventing its eventual shape. */
  readonly aspect: Aspect;
  /** Prevents unresolved registrations from being fabricated as ordinary values. */
  readonly [unresolvedGeneratedBrand]: true;
}

export { ACTOR_TYPE_NAMES } from "./model.generated.js";
export type { ActorTypeName } from "./model.generated.js";

/** Names the four scalar shapes admitted by the Circular value model. */
export type BaseType = "bool" | "number" | "string" | "null";

/** Describes one named field in an object shape without relying on map order. */
export interface ShapeField {
  /** Holds the canonical field name. */
  readonly name: string;
  /** Holds the field's recursively bounded shape. */
  readonly shape: Shape;
}

/** Describes the bounded item shape carried by an array. */
export interface ArrayShape {
  /** Discriminates the array constructor. */
  readonly kind: "array";
  /** Describes every array member. */
  readonly item: Shape;
}

/** Describes a bounded ordered encoding of an unordered object field set. */
export interface ObjectShape {
  /** Discriminates the object constructor. */
  readonly kind: "object";
  /** Lists unique fields in canonical name order. */
  readonly fields: readonly ShapeField[];
  /** States whether fields outside the declared set remain admissible. */
  readonly open: boolean;
}

/** Describes one scalar shape. */
export interface ScalarShape {
  /** Discriminates the scalar constructor. */
  readonly kind: "base";
  /** Names the represented Circular scalar family. */
  readonly base: BaseType;
}

/** Describes the greatest shape used when a narrower value shape is unavailable. */
export interface AnyShape {
  /** Discriminates the greatest shape constructor. */
  readonly kind: "any";
}

/** Describes a bounded Circular value shape. */
export type Shape = AnyShape | ScalarShape | ArrayShape | ObjectShape;

/** Describes a fixed non-zero logical signal period. */
export interface PeriodRate {
  /** Discriminates the fixed-period constructor. */
  readonly kind: "period";
  /** Holds a positive logical tick count. */
  readonly ticks: number;
}

/** Describes a symbolic rate variable resolved by scope type inference. */
export interface VariableRate {
  /** Discriminates the variable-rate constructor. */
  readonly kind: "variable";
  /** Holds the canonical rate variable name. */
  readonly name: string;
}

/** Describes a signal rate expression. */
export type RateExpression = PeriodRate | VariableRate;

/** Describes a stream of independently stamped items. */
export interface StreamFlow {
  /** Discriminates the stream constructor. */
  readonly kind: "stream";
  /** Describes each stream item. */
  readonly item: Shape;
}

/** Describes a value sampled on one logical signal grid. */
export interface SignalFlow {
  /** Discriminates the signal constructor. */
  readonly kind: "signal";
  /** Describes each sample. */
  readonly item: Shape;
  /** Describes the signal's fixed or inferred logical rate. */
  readonly rate: RateExpression;
}

/** Describes the canonical type of one dataflow port. */
export type Flow = StreamFlow | SignalFlow;

/** Identifies a canonical module specifier recorded by authoring provenance. */
export type ModuleSpecifierOrigin = string;
