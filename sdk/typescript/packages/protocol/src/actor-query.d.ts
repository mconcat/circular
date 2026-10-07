/** Existing runtime wrapper used by structural PortFlow hints; type-only export. */
export type { CircularUInt } from "./value.js";
import type { Result, CircularValue, PortFlowAvailability } from './index.js';
/** Exact daemon row; field names and values are not reinterpreted. */
export interface ActorCatalogItem {
  readonly actor_type: string;
  readonly label: string;
  readonly description: string;
  readonly presentation_role: readonly CircularValue[];
  /** Opaque type defaults; resolve with @circular/core resolveViewConfig. Null when undeclared. */
  readonly view_config: CircularValue;
  readonly source: boolean;
  readonly config_schema: CircularValue;
  readonly creatable: boolean;
  readonly template_config: CircularValue;
  readonly in_ports: readonly ActorQueryPort[];
  readonly out_ports: readonly ActorQueryPort[];
  readonly ports_unavailable_reason: string | null;
  readonly unavailable_reason: string | null;
}
export interface ActorQueryPort { readonly id: string; readonly primary: boolean }
export interface ActorCreateAdmissionItem {
  readonly authored_actor: CircularValue;
  readonly config: CircularValue;
  readonly in_ports: readonly ActorQueryPort[];
  readonly out_ports: readonly ActorQueryPort[];
  readonly actor_type: string;
}
export interface AuthoringActorPortsItem {
  readonly actor: CircularValue;
  readonly in_ports: readonly { readonly id: string; readonly flow: PortFlowAvailability; readonly label: string | null }[];
  readonly out_ports: readonly { readonly id: string; readonly flow: PortFlowAvailability; readonly label: string | null }[];
}
export declare function actorCatalogItemFromValue(value: unknown): ActorCatalogItem;
export declare function actorCreateAdmissionItemFromValue(value: unknown): ActorCreateAdmissionItem;
export declare function authoringActorPortsItemFromValue(value: unknown): AuthoringActorPortsItem;
export declare function actorCreateAdmissionArgumentsValue(actorType: string, config: unknown, authoredActor: unknown): unknown;
export declare function authoringActorPortsArgumentsValue(scope: unknown): unknown;
export declare function actorQueryResultFromValue<T>(value: unknown, decoder: (item: unknown) => T): Result<{ readonly anchor: unknown; readonly items: readonly T[] }>;
