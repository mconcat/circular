import type {
  DeclarationCommand,
  UpsertTemplateCommand,
  RetireTemplateCommand,
  DeclarationContentCommand,
  EpochAddressDomain,
  ReconstructiveDeclarationCommand,
  RelativeAddressDomain,
} from "./index.js";

export type DeclarationAddressContext = "mutation" | "acceptedHistory" | "snapshot";

export type { UpsertTemplateCommand, RetireTemplateCommand } from "./index.js";

/**
 * EpochId and CommitId require non-empty Uint8Array; their widths are not fixed.
 * @throws TypeError with code CARRIER_NOT_BYTES or CARRIER_EMPTY for invalid carriers.
 * @throws TypeError with code KEY_DISAGREES_WITH_DECLARATION when a mutation's
 * UpsertEdge key differs from edgeKeyFromDeclaration(declaration).
 */
export declare function declarationPayloadValue(
  command: DeclarationCommand<EpochAddressDomain> | UpsertTemplateCommand | RetireTemplateCommand,
  options?: { readonly context?: DeclarationAddressContext; readonly includeKind?: boolean },
): unknown;

/** Active payload readers retain their exact command discriminants. */
export declare function declarationCommandFromValue(
  arm: "UpsertTemplate", value: unknown,
  options?: { readonly context?: DeclarationAddressContext; readonly includesKind?: boolean },
): UpsertTemplateCommand;
export declare function declarationCommandFromValue(
  arm: "RetireTemplate", value: unknown,
  options?: { readonly context?: DeclarationAddressContext; readonly includesKind?: boolean },
): RetireTemplateCommand;

export declare function declarationCommandFromValue(
  arm: DeclarationCommand<EpochAddressDomain>["kind"],
  value: unknown,
  options?: { readonly context?: DeclarationAddressContext; readonly includesKind?: boolean },
): DeclarationCommand<EpochAddressDomain> | DeclarationContentCommand<EpochAddressDomain>;

export declare function reconstructiveCommandFromValue(
  value: unknown,
): ReconstructiveDeclarationCommand<RelativeAddressDomain> | UpsertTemplateCommand;

export declare function declarationResultFromValue(
  value: unknown,
  requestKind: DeclarationCommand<EpochAddressDomain>["kind"],
): import("./index.js").Result<unknown>;

export declare function acceptedCommitMetadataFromValue(
  value: unknown,
): import("./index.js").AcceptedCommitMetadata;

export declare function authoringCommitFrameFromValue(
  value: unknown,
): import("./index.js").AuthoringCommitFrame;

/** Existing address encoder, also used for prospective create-admission identities. */
export declare function declarationAddressValue(address: unknown, entity: string, context: string): [bigint, unknown];
export declare function declarationAddressFromValue(value: unknown, entity: string, context: string): { arm: string; value: unknown };
/** The attributes an authored edge carries when its author states none. */
export declare const DEFAULT_EDGE_ATTRS: {
  readonly delay: { readonly num: 1n; readonly den: 2n };
  readonly policy: { readonly delivery: "Lossless"; readonly capacity: 64n };
};
export declare function edgeKeyFromDeclaration<From, To>(declaration: { readonly from: From; readonly to: To; readonly ordinal: number }): { readonly from: From; readonly to: To; readonly ordinal: number };
