import type { BaseShape, CircularRecord, CircularValue } from "@circular/protocol";
import type { Flow, ActorConfig, ActorTypeName } from "@circular/specs";

import type { PublicActorSpelling } from "./surface.generated.js";
export type { PublicActorSpelling } from "./surface.generated.js";

/** Maps a public constructor spelling to its canonical engine actor type. */
export type CanonicalActorTypeFor<Name extends PublicActorSpelling> =
  Name extends "project_input"
    ? "input"
    : Name extends "project_output"
      ? "output"
      : Name extends ActorTypeName
        ? Name
        : never;

/** Selects the generated engine configuration behind one public constructor spelling. */
export type GeneratedConfigFor<Name extends PublicActorSpelling> =
  Name extends "replicator" ? ReplicatorConfig
    : Name extends "form" ? Omit<ActorConfig<"form">, "fields"> & {
      /** Field names in object key order, e.g. { question: "string", count: "int" }.
       * A canonical array carries shapes outside this closed object-of-base-types shorthand.
       */
      readonly fields: Readonly<Record<string, BaseShape>> | readonly CanonicalTypeValue[];
    } : Name extends "input" ? Omit<ActorConfig<"input">, "shape"> & {
      /**
       * The type this input injects, one base type, e.g. "float". Left out, no `shape` key is written
       * and the boundary takes the `absent` Flow the daemon publishes for this slot
       * (`actor.create-inputs`, requirement `[3, absent]`).
       */
      readonly shape?: BaseShape;
    } : ActorConfig<CanonicalActorTypeFor<Name>>;

type CanonicalTypeValue = string | bigint | boolean | readonly CanonicalTypeValue[]
  | { readonly [key: string]: CanonicalTypeValue };

/** Describes a v2 boundary Flow directly or through one pinned source shorthand. */
export type BoundaryTypeExpression = Flow | "json" | "text";

/** Describes one child-module ingress boundary projection. */
export interface ProjectInputConfig {
  /** Names the child boundary topic that must match its parent interface entry. */
  readonly topic: string;
  /**
   * The type this input injects, one base type, e.g. "float". Left out, no `shape` key is written
   * and the boundary takes the `absent` Flow the daemon publishes for this slot
   * (`actor.create-inputs`, requirement `[3, absent]`).
   */
  readonly shape?: BaseShape;
}

/** Describes one child-module egress boundary projection. */
export interface ProjectOutputConfig {
  /** Names the child boundary topic that must match its parent interface entry. */
  readonly topic: string;
}

export type BoundaryInterface<Names extends string = string> = readonly Names[];

/** Describes a finite literal-keyed template module map. */
export type TemplateReferenceMap<Names extends string = never> = Readonly<{
  [Name in Names]: string;
}>;

/** A bundle source declares an editable child scope; a named function supplies a stored
 * Template value whose generated children are absent from the authoring snapshot.
 */
export type PipelineActorConfig<
  InputNames extends string = string,
  OutputNames extends string = string,
  TemplateNames extends string = never,
> = {
  readonly in: BoundaryInterface<InputNames>;
  readonly out: BoundaryInterface<OutputNames>;
} & (
  | { readonly source: string; readonly template?: never }
  | { readonly template: () => void; readonly source?: never }
);

/** A replicator mints its cells from one stored Template value; policy and boundary fields
 * travel together, and there is no policy-only call.
 */
export type ReplicatorConfig = ActorConfig<"replicator"> & {
  readonly template: () => void;
  readonly in: BoundaryInterface;
  readonly out: BoundaryInterface;
};

/** A CEL string or source arrow lowered by semanticPrepass; never executed as JavaScript. */
export type TransformExpression<
  Input = CircularValue,
  Output = CircularValue,
> = string | ((event: Input) => Output);

/** A CEL string or source predicate arrow lowered by semanticPrepass; receives payload only. */
export type PredicateExpression<Input = CircularValue> = string | ((event: Input) => boolean);

/**
 * Selects the public authoring configuration associated with one constructor spelling.
 * Generated unresolved entries remain unresolved instead of becoming open dictionaries.
 */
export type AuthoringConfigFor<Name extends PublicActorSpelling> =
  Name extends "project_input"
    ? ProjectInputConfig
    : Name extends "project_output"
      ? ProjectOutputConfig
      : Name extends "pipeline_actor"
        ? PipelineActorConfig
        : GeneratedConfigFor<Name>;

/** Restricts provider-neutral actor replacement input to first-class Circular records. */
export type ReplacementConfig = CircularRecord;

export type { InputConfig, JsonConfig, EmaConfig } from "@circular/specs";
