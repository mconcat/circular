import type { ActorConfigTable as GeneratedActorConfigTable } from "./config.generated.js";
import type { CircularRecord } from "@circular/protocol";
import type { ActorTypeName, UnresolvedGeneratedRegistration } from "./model.js";

/** Marks one actor configuration whose v2 generated registration is incomplete. */
export type UnresolvedActorConfig<Name extends ActorTypeName | keyof GeneratedActorConfigTable> =
  UnresolvedGeneratedRegistration<Name, "config-schema">;

type BeyondSchemaActorConfigs = {
  readonly tool_executor: Omit<GeneratedActorConfigTable["tool_executor"], "tools"> & {
    readonly tools: Readonly<Record<string, CircularRecord & Partial<Pick<
      NonNullable<NonNullable<GeneratedActorConfigTable["tool_executor"]["capabilities"]>["ProcessSpawn"]>, "approval">>>>;
  };
};

export interface ActorConfigTable extends Omit<GeneratedActorConfigTable, keyof BeyondSchemaActorConfigs>, BeyondSchemaActorConfigs {}

/** Selects the generated configuration entry for one canonical actor type. */
export type ActorConfig<Name extends ActorTypeName | keyof GeneratedActorConfigTable> = ActorConfigTable[Name];

/** Config projected from the pinned actor.create-inputs response. */
export type AgentConfig = ActorConfigTable["agent"];
/** Config projected from the pinned actor.create-inputs response. */
export type PipelineActorConfig = ActorConfigTable["pipeline_actor"];
/** Config projected from the pinned actor.create-inputs response. */
export type ReplicatorConfig = ActorConfigTable["replicator"];
/** Config projected from the pinned actor.create-inputs response. */
export type JsonConfig = ActorConfigTable["json"];
/** Config projected from the pinned actor.create-inputs response. */
export type InputConfig = ActorConfigTable["input"];
/** Config projected from the pinned actor.create-inputs response. */
export type EmaConfig = ActorConfigTable["ema"];
