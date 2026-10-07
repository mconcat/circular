import type {
  AuthoringRevision,
  CircularValue,
  ActorId,
} from "@circular/protocol";
import type { ActorInputName, ActorOutputName } from "@circular/specs";
import type {
  AuthoringConfigFor,
  BoundaryInterface,
  CanonicalActorTypeFor,
  PublicActorSpelling,
  ReplicatorConfig,
} from "./config.js";
import type {
  LayoutActorReference,
  ActorAuthoringSurface,
  Presentation,
} from "./presentation.js";
import type {
  CurrentHandleMode,
  HandleMode,
  NewHandleMode,
  SourceEndpoint,
  TargetEndpoint,
  WritableBoundaryEndpoint,
  InletObservation,
} from "./wiring.js";

type OverridePortName<Ports> = Ports extends object ? Extract<keyof Ports, string> : never;
type OverridePortValue<Ports, Name extends string> = [Ports] extends [never] ? CircularValue : Ports extends object
  ? Name extends keyof Ports
    ? Ports[Name]
    : CircularValue
  : CircularValue;
type InputPortValue<Name, Ports, Port extends string> = Name extends "agent"
  ? Port extends "turn" ? string | Uint8Array | { readonly op: never } : OverridePortValue<Ports, Port>
  : OverridePortValue<Ports, Port>;
export type InputNameFor<Name extends PublicActorSpelling, Overrides = never> =
  Name extends "agent"
    ? Exclude<ActorInputName<CanonicalActorTypeFor<Name>> | OverridePortName<Overrides>, "control">
    : ActorInputName<CanonicalActorTypeFor<Name>> | OverridePortName<Overrides>;
type OutputNameFor<Name extends PublicActorSpelling, Overrides> =
  | ActorOutputName<CanonicalActorTypeFor<Name>>
  | OverridePortName<Overrides>;
type BoundaryValues<Names extends string> = {
  readonly [Name in Names]: CircularValue;
};

import type { SourceOnlyActorSpelling, FlowActorSpelling, TerminalActorSpelling, MultiOutletActorSpelling, DynamicPortActorSpelling } from "./surface.generated.js";
import type { DownstreamActorMethods } from "./catalog.generated.js";
export type { SourceOnlyActorSpelling, FlowActorSpelling, TerminalActorSpelling, MultiOutletActorSpelling, DynamicPortActorSpelling } from "./surface.generated.js";

/** Provides actor identity, named endpoint selection, and authored non-edge operations. */
export interface ActorHandle<
  Name extends PublicActorSpelling = PublicActorSpelling,
  Mode extends HandleMode = HandleMode,
  Inputs = never,
  Outputs = never,
> extends LayoutActorReference,
    ActorAuthoringSurface {
  /** Names the public constructor spelling that created or resolved this handle. */
  readonly actorType: Name;
  /** Distinguishes new and anchored-current handles. */
  readonly mode: Mode;
  /** Endpoints named by the pinned catalog or program admission. */
  readonly in: { readonly [P in InputNameFor<Name, Inputs>]: TargetEndpoint<InputPortValue<Name, Inputs, P>, Mode> & InletObservation };
  /** Endpoints named by the pinned catalog or program admission. */
  readonly out: { readonly [P in OutputNameFor<Name, Outputs>]: SourceEndpoint<OverridePortValue<Outputs, P>, Mode> };
  /** Claims one observed fixed export role without defining a full export mount. */
  export(role: "progress" | "result" | "error"): this;
  /** Replaces the complete generated actor configuration (every handle, new or current). */
  replaceConfig(config: AuthoringConfigFor<Name>): void;
  /** Replaces the complete normalized presentation value. */
  replacePresentation(presentation: Presentation): void;
  /**
   * Retires the bound actor: first each export mount that names it in any role, then the actor. The
   * mount verbs are the ones a canvas Delete sends; the actor's wires go with the actor in the
   * daemon. A handle made by this program names no mount yet.
   */
  remove(): void;
}

/** Represents a source-only actor handle. */
export interface SourceActorHandle<
  Name extends SourceOnlyActorSpelling,
  Mode extends HandleMode,
  Value = CircularValue,
> extends ActorHandle<Name, Mode>,
    SourceEndpoint<Value, Mode> {}

/** Represents an actor with default input and output endpoints. */
export interface FlowActorHandle<
  Name extends FlowActorSpelling,
  Mode extends HandleMode,
  Input = CircularValue,
  Output = CircularValue,
> extends ActorHandle<Name, Mode>,
    TargetEndpoint<InputPortValue<Name, { turn: Input }, "turn">, Mode>,
    SourceEndpoint<Output, Mode> {}

/** Represents a target-only terminal actor. */
export interface TerminalActorHandle<
  Name extends TerminalActorSpelling,
  Mode extends HandleMode,
  Input = CircularValue,
> extends ActorHandle<Name, Mode>,
    TargetEndpoint<Input, Mode> {}

/** Represents an actor without primary endpoints; named ports require explicit selection. */
export interface MultiOutletActorHandle<
  Name extends MultiOutletActorSpelling,
  Mode extends HandleMode,
  Inputs = never,
  Outputs = never,
  Input = CircularValue,
> extends ActorHandle<Name, Mode, Inputs, Outputs> {}

/** Represents a child-module ingress boundary with export-injection write semantics. */
export interface WritableBoundaryActorHandle<
  Name extends "input" | "project_input",
  Mode extends HandleMode,
  Value = CircularValue,
> extends SourceActorHandle<Name, Mode, Value>,
    WritableBoundaryEndpoint<Value, Mode> {
  /** Claims the writable fixed export role for this boundary. */
  export(role: "request" | "progress" | "result" | "error"): this;
}

/** Represents a nested pipeline with literal config-derived endpoint names. */
export interface PipelineActorHandle<
  Mode extends HandleMode,
  InputNames extends string,
  OutputNames extends string,
  TemplateNames extends string = never,
> extends ActorHandle<
    "pipeline_actor",
    Mode,
    BoundaryValues<InputNames>,
    BoundaryValues<OutputNames>
  > {
  /** Preserves the template-name parameter for exact config-derived typing. */
  readonly templateNames?: TemplateNames;
}

export type ReplicatorActorHandle<
  Name extends "replicator",
  Mode extends HandleMode,
  Config extends ReplicatorConfig,
> = ActorHandle<Name, Mode, never, BoundaryValues<Config["out"][number]>>
  & TargetEndpoint<CircularValue, Mode>;

/** Holds the anchor of a current actor: its stable identity and the revision it is bound to. */
export interface CurrentActorOperations<Name extends PublicActorSpelling> {
  /** Holds the stable authoring actor identity and never a runtime actor identity. */
  readonly actorId: ActorId;
  /** Holds the authoring revision to which this handle is bound. */
  readonly revision: AuthoringRevision;
}

/** Represents a newly declared source-only actor. */
export interface NewSourceActorHandle<
  Name extends SourceOnlyActorSpelling,
  Value = CircularValue,
> extends SourceActorHandle<Name, NewHandleMode, Value> {}

/** Represents a newly declared child ingress boundary writable through an export request role. */
export interface NewWritableBoundaryActorHandle<
  Value = CircularValue,
> extends WritableBoundaryActorHandle<"project_input", NewHandleMode, Value> {}

/** Represents a newly declared primary-input/primary-output actor. */
export interface NewFlowActorHandle<
  Name extends FlowActorSpelling,
  Input = CircularValue,
  Output = CircularValue,
> extends FlowActorHandle<Name, NewHandleMode, Input, Output> {}

/** Represents a newly declared terminal actor. */
export interface NewTerminalActorHandle<
  Name extends TerminalActorSpelling,
  Input = CircularValue,
> extends TerminalActorHandle<Name, NewHandleMode, Input> {}

/** Represents a newly declared actor with explicitly selected outputs. */
export interface NewMultiOutletActorHandle<
  Name extends MultiOutletActorSpelling,
  Inputs = never,
  Outputs = never,
  Input = CircularValue,
> extends MultiOutletActorHandle<Name, NewHandleMode, Inputs, Outputs, Input> {}

/** Represents an anchored current source-only actor. */
export interface CurrentSourceActorHandle<
  Name extends SourceOnlyActorSpelling,
  Value = CircularValue,
> extends SourceActorHandle<Name, CurrentHandleMode, Value>,
    CurrentActorOperations<Name> {}

/** Represents an anchored child ingress boundary writable through an export request role. */
export interface CurrentWritableBoundaryActorHandle<
  Value = CircularValue,
> extends WritableBoundaryActorHandle<"project_input", CurrentHandleMode, Value>,
    CurrentActorOperations<"project_input"> {}

/** Represents an anchored current primary-input/primary-output actor. */
export interface CurrentFlowActorHandle<
  Name extends FlowActorSpelling,
  Input = CircularValue,
  Output = CircularValue,
> extends FlowActorHandle<Name, CurrentHandleMode, Input, Output>,
    CurrentActorOperations<Name> {}

/** Represents an anchored current terminal actor. */
export interface CurrentTerminalActorHandle<
  Name extends TerminalActorSpelling,
  Input = CircularValue,
> extends TerminalActorHandle<Name, CurrentHandleMode, Input>,
    CurrentActorOperations<Name> {}

/** Represents an anchored current actor with explicitly selected outputs. */
export interface CurrentMultiOutletActorHandle<
  Name extends MultiOutletActorSpelling,
  Inputs = never,
  Outputs = never,
  Input = CircularValue,
> extends MultiOutletActorHandle<Name, CurrentHandleMode, Inputs, Outputs, Input>,
    CurrentActorOperations<Name> {}

/** Selects the new-handle category associated with one public constructor spelling. */
export type NewActorHandle<Name extends PublicActorSpelling, Inputs = never, Outputs = never> =
  Name extends "input" | "project_input"
    ? WritableBoundaryActorHandle<Name, NewHandleMode>
    : Name extends SourceOnlyActorSpelling
    ? NewSourceActorHandle<Name>
    : Name extends TerminalActorSpelling
      ? NewTerminalActorHandle<Name>
      : Name extends MultiOutletActorSpelling
        ? NewMultiOutletActorHandle<Name, Inputs, Outputs>
        : Name extends FlowActorSpelling
          ? NewFlowActorHandle<Name>
          : Name extends DynamicPortActorSpelling ? ActorHandle<Name, NewHandleMode, Inputs, Outputs> & DownstreamActorMethods<CircularValue> & SourceEndpoint<CircularValue, NewHandleMode> & TargetEndpoint<CircularValue, NewHandleMode>
          : Name extends "pipeline_actor" ? PipelineActorHandle<NewHandleMode, never, never> : never;

/** Selects the anchored current-handle category associated with one public spelling. */
export type CurrentActorHandle<Name extends PublicActorSpelling, Inputs = never, Outputs = never> =
  Name extends "input" | "project_input"
    ? WritableBoundaryActorHandle<Name, CurrentHandleMode> & CurrentActorOperations<Name>
    : Name extends SourceOnlyActorSpelling
    ? CurrentSourceActorHandle<Name>
    : Name extends TerminalActorSpelling
      ? CurrentTerminalActorHandle<Name>
      : Name extends MultiOutletActorSpelling
        ? CurrentMultiOutletActorHandle<Name, Inputs, Outputs>
        : Name extends FlowActorSpelling
          ? CurrentFlowActorHandle<Name>
          : Name extends DynamicPortActorSpelling ? ActorHandle<Name, CurrentHandleMode, Inputs, Outputs> & DownstreamActorMethods<CircularValue> & SourceEndpoint<CircularValue, CurrentHandleMode> & TargetEndpoint<CircularValue, CurrentHandleMode> & CurrentActorOperations<Name>
          : Name extends "pipeline_actor" ? PipelineActorHandle<CurrentHandleMode, never, never> & CurrentActorOperations<Name> : never;
