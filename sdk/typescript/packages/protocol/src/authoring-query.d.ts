import type { AuthoringCommitFrame, AuthoringSnapshotAnchor, ReconstructiveDeclarationCommand, RelativeAddressDomain, Result, ScopeId, StructureCursor } from "./index.js";

export declare function authoringSnapshotArgumentsValue(scope: ScopeId): unknown;
export declare function authoringCommitArgumentsValue(scope: ScopeId, after: StructureCursor): unknown;
export declare function authoringCommitFrameFromValue(value: unknown): AuthoringCommitFrame;
export declare function authoringSnapshotAnchorFromValue(value: unknown): AuthoringSnapshotAnchor;
export declare function authoringSnapshotQueryResultFromValue(value: unknown): Result<{
  readonly anchor: AuthoringSnapshotAnchor;
  readonly items: readonly ReconstructiveDeclarationCommand<RelativeAddressDomain>[];
  readonly terminal: "More" | "Complete" | "Diagnostic";
  readonly next?: unknown;
}>;
