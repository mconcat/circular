import type { PortContract, UnresolvedActorPorts } from "./ports.js";
import type { ActorTypeName } from "./model.js";

/** Port names from the pinned daemon catalog; value shapes remain CircularValue. */
export interface ActorInputTable {
  readonly "route": readonly PortContract<"event", "input">[];
  readonly "pipeline_actor": UnresolvedActorPorts<"pipeline_actor", "input">;
  readonly "debounce": readonly PortContract<"event", "input">[];
  readonly "alert": readonly PortContract<"event", "input">[];
  readonly "tap": readonly PortContract<"event", "input">[];
  readonly "input": UnresolvedActorPorts<"input", "input">;
  readonly "output": UnresolvedActorPorts<"output", "input">;
  readonly "replicator": readonly PortContract<"event", "input">[];
  readonly "agent": readonly PortContract<"turn" | "tool_result", "input">[];
  readonly "counter": readonly PortContract<"event", "input">[];
  readonly "ema": readonly PortContract<"sample", "input">[];
  readonly "windowed_reduce": readonly PortContract<"sample", "input">[];
  readonly "timer": readonly PortContract<"bang", "input">[];
  readonly "tool_executor": readonly PortContract<"call", "input">[];
  readonly "notify": readonly PortContract<"notification", "input">[];
  readonly "peer": readonly PortContract<"send" | "refresh", "input">[];
  readonly "listener": readonly PortContract<"control", "input">[];
  readonly "keyed_reduce": readonly PortContract<"event" | "remove", "input">[];
  readonly "request": readonly PortContract<"event", "input">[];
  readonly "file": readonly PortContract<"write" | "read", "input">[];
  readonly "json": readonly PortContract<"set" | "bang", "input">[];
  readonly "otlp": UnresolvedActorPorts<"otlp", "input">;
  readonly "match": readonly PortContract<"event", "input">[];
  readonly "assemble": readonly PortContract<"event", "input">[];
  readonly "join": readonly PortContract<"event" | "state" | "remove", "input">[];
  readonly "form": UnresolvedActorPorts<"form", "input">;
}

/** Port names from the pinned daemon catalog; value shapes remain CircularValue. */
export interface ActorOutputTable {
  readonly "route": readonly PortContract<"unmatched", "output">[];
  readonly "pipeline_actor": UnresolvedActorPorts<"pipeline_actor", "output">;
  readonly "debounce": readonly PortContract<"event", "output">[];
  readonly "alert": readonly PortContract<"event" | "transition", "output">[];
  readonly "tap": readonly PortContract<"event", "output">[];
  readonly "input": UnresolvedActorPorts<"input", "output">;
  readonly "output": UnresolvedActorPorts<"output", "output">;
  readonly "replicator": UnresolvedActorPorts<"replicator", "output">;
  readonly "agent": readonly PortContract<"record" | "tool_request" | "result" | "_error", "output">[];
  readonly "counter": readonly PortContract<"count", "output">[];
  readonly "ema": readonly PortContract<"ema", "output">[];
  readonly "windowed_reduce": UnresolvedActorPorts<"windowed_reduce", "output">;
  readonly "timer": readonly PortContract<"tick" | "_error", "output">[];
  readonly "tool_executor": readonly PortContract<"result" | "_error", "output">[];
  readonly "notify": readonly PortContract<"_error", "output">[];
  readonly "peer": readonly PortContract<"message" | "peers" | "delivery" | "binding" | "_error", "output">[];
  readonly "listener": readonly PortContract<"line" | "_error", "output">[];
  readonly "keyed_reduce": readonly PortContract<"map" | "total" | "count", "output">[];
  readonly "request": readonly PortContract<"response" | "_error", "output">[];
  readonly "file": readonly PortContract<"content" | "written" | "_error", "output">[];
  readonly "json": readonly PortContract<"value", "output">[];
  readonly "otlp": readonly PortContract<"logs" | "metrics", "output">[];
  readonly "match": readonly PortContract<"ok" | "err", "output">[];
  readonly "assemble": readonly PortContract<"event" | "_error", "output">[];
  readonly "join": readonly PortContract<"event", "output">[];
  readonly "form": UnresolvedActorPorts<"form", "output">;
}

/** Unpublished registrations remain unresolved. */
export type ActorInputs<Name extends ActorTypeName> = Name extends keyof ActorInputTable ? ActorInputTable[Name] : UnresolvedActorPorts<Name, "input">;
export type ActorOutputs<Name extends ActorTypeName> = Name extends keyof ActorOutputTable ? ActorOutputTable[Name] : UnresolvedActorPorts<Name, "output">;
/** No config-derived names are inferred here; program admission owns those names. */
export type ActorInputName<Name extends ActorTypeName> = ActorInputs<Name> extends readonly PortContract<infer P, "input">[] ? P : never;
export type ActorOutputName<Name extends ActorTypeName, Config = never> = ActorOutputs<Name> extends readonly PortContract<infer P, "output">[] ? P : never;
