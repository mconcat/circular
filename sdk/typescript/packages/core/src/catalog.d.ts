export * from "./catalog.generated.js";
export { join } from "./join.js";
import type { DownstreamActorMethods as CapturedDownstreamActorMethods } from "./catalog.generated.js";
import type { CircularValue } from "@circular/protocol";
import type { SourceEndpoint } from "./wiring.js";
import type { NewFlowActorHandle } from "./handles.js";
import type { join } from "./join.js";
type SourceValue<Source> = Source extends SourceEndpoint<infer Value> ? Value : never;
export declare function merge<
  const Sources extends readonly [SourceEndpoint<unknown>, SourceEndpoint<unknown>, ...SourceEndpoint<unknown>[]],
>(
  ...sources: Sources
): NewFlowActorHandle<"tap", SourceValue<Sources[number]>, SourceValue<Sources[number]>>;

export declare function assemble(config: {
  readonly at: readonly (string | bigint)[];
  readonly inactivity_timeout: bigint;
  readonly max_window: bigint;
  readonly capacity: bigint;
}, inputs?: { readonly event?: SourceEndpoint<unknown> | import("./wiring.js").PendingEndpoint<unknown>
  | readonly (SourceEndpoint<unknown> | import("./wiring.js").PendingEndpoint<unknown>)[] }):
  import("./presentation.js").LayoutActorReference & import("./presentation.js").ActorAuthoringSurface
  & SourceEndpoint<{ readonly key: string; readonly events: readonly import("@circular/protocol").CircularRecord[]; readonly stuck: boolean }, import("./wiring.js").NewHandleMode>
  & import("./wiring.js").TargetEndpoint<import("@circular/protocol").CircularRecord, import("./wiring.js").NewHandleMode> & {
    readonly actorType: "assemble";
    readonly mode: import("./wiring.js").NewHandleMode;
    readonly in: { readonly event: import("./wiring.js").TargetEndpoint<import("@circular/protocol").CircularRecord> & import("./wiring.js").InletObservation };
    readonly out: { readonly _error: NewFlowActorHandle<"assemble">["out"]["_error"]; readonly event: SourceEndpoint<{ readonly key: string; readonly events: readonly import("@circular/protocol").CircularRecord[]; readonly stuck: boolean }, import("./wiring.js").NewHandleMode> };
  };
export interface DownstreamActorMethods<Input = import("@circular/protocol").CircularValue> extends Omit<CapturedDownstreamActorMethods<Input>, "assemble" | "join"> {
  join(config: Parameters<typeof join>[0], options?: { readonly at?: "event" | "state" | "remove" }): ReturnType<typeof join>;
  assemble(config: Parameters<typeof assemble>[0], options?: { readonly at?: "event" }): ReturnType<typeof assemble>;
}
