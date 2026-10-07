/** @packageDocumentation Stable declaration surface for the anchor-bound `circular:current` module. */

import type { CurrentEdgeHandle, CurrentActorHandle, PublicActorSpelling } from "@circular/core";
import type {
  CurrentAnnotationHandle,
  CurrentExportHandle,
  CurrentLookupNamespace,
  CurrentProjectModule,
  CurrentScopeHandle,
} from "./index.js";

/** The union used by a generated snapshot-specific named actor export. */
export type CurrentNamedActor = CurrentActorHandle<PublicActorSpelling>;

/** Exact edge, scope, export, and annotation lookup for this module's snapshot anchor. */
export declare const current: CurrentLookupNamespace;

/** Re-exports the generated current-actor handle family used by named actor exports. */
export type { CurrentActorHandle };

/** Re-exports the current edge handle with explicit disconnection only. */
export type { CurrentEdgeHandle };

/** Re-exports the closed public actor spelling union used by generated actor exports. */
export type { PublicActorSpelling };

/** Re-exports the stable current lookup namespace contract. */
export type { CurrentLookupNamespace };

/** Re-exports the host record behind the fixed snapshot-specific lookup namespace. */
export type { CurrentProjectModule };

/** Re-exports the current scope handle with explicit removal only. */
export type { CurrentScopeHandle };

/** Re-exports the current export-mount handle with explicit removal only. */
export type { CurrentExportHandle };

/** Re-exports the current annotation handle with explicit removal only. */
export type { CurrentAnnotationHandle };
