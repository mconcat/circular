import type { CircularValue } from "@circular/protocol";
import type { SourceEndpoint, TargetEndpoint, PendingEndpoint, NewHandleMode, InletObservation } from "./wiring.js";
import type { LayoutActorReference, ActorAuthoringSurface } from "./presentation.js";
export declare function join(config: { readonly at: readonly (string | bigint)[] },
  inputs?: { readonly [P in "event" | "state" | "remove"]?: SourceEndpoint<unknown> | PendingEndpoint<unknown>
    | readonly (SourceEndpoint<unknown> | PendingEndpoint<unknown>)[] }):
  LayoutActorReference & ActorAuthoringSurface
  & SourceEndpoint<{ readonly event: CircularValue; readonly state: CircularValue }, NewHandleMode>
  & TargetEndpoint<CircularValue, NewHandleMode> & {
    readonly actorType: "join";
    readonly mode: NewHandleMode;
    readonly in: { readonly [P in "event" | "state" | "remove"]: TargetEndpoint<CircularValue> & InletObservation };
    readonly out: { readonly event: SourceEndpoint<{ readonly event: CircularValue; readonly state: CircularValue }, NewHandleMode> };
  };
