import type { PortId } from "@circular/protocol";
import type { Flow, UnresolvedGeneratedRegistration } from "./model.js";

/** Names the direction of one generated graph port. */
export type PortDirection = "input" | "output";

/** Describes whether one generated port accepts one edge or many parallel edges. */
export type PortArity = "one" | "many";

/** Describes whether an input port must receive a value. */
export type InputPresence = "required" | "optional";

/** Describes one fully generated fixed or config-expanded graph port. */
export interface PortContract<
  Name extends string = string,
  Direction extends PortDirection = PortDirection,
> {
  /** Holds the canonical generated port id. */
  readonly id: PortId;
  /** Holds the source-level canonical port name. */
  readonly name: Name;
  /** Holds the port direction. */
  readonly direction: Direction;
  /** Holds the generated Circular flow type. */
  readonly flow: Flow;
  /** Holds the edge arity admitted by the port. */
  readonly arity: PortArity;
  /** Holds input presence and is absent for output ports. */
  readonly presence?: InputPresence;
  /** Marks the unique generated default in this direction when one exists. */
  readonly primary: boolean;
}

/** Marks an incomplete generated input or output registration. */
export type UnresolvedActorPorts<
  Name extends string,
  Direction extends PortDirection,
> = UnresolvedGeneratedRegistration<Name, `${Direction}-ports`>;

export type { ActorInputTable, ActorOutputTable, ActorInputs, ActorOutputs, ActorInputName, ActorOutputName } from "./ports.generated.js";
