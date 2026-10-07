/** @packageDocumentation Internal host-adapter boundary for an implicit authored-code epoch. */

import type {
  BeginEpochCommand,
  CorrelationId,
  DeclarationContentCommand,
  EpochAborted,
  EpochAddressDomain,
  EpochCommitted,
  EpochContentRequest,
  EpochId,
  Result,
  ValidateEpochCommand,
  CommitEpochCommand,
  AbortEpochCommand,
  CommitId,
  Diagnostic,
  NonEmptyReadonlyArray,
} from "@circular/protocol";
import type { Session } from "../index.js";

/** Existing declaration RPC requests emitted by one code execution, in transport order. */
export type ExecutionEpochRequest =
  | BeginEpochCommand<EpochAddressDomain>
  | EpochContentRequest
  | ValidateEpochCommand
  | CommitEpochCommand
  | AbortEpochCommand;

/**
 * A CommitEpoch that was sent and never answered, and that the durable commit record did not
 * show within one request window. The daemon may still have committed it — this is not a
 * rejection. `commitId` is the identity to look for in the authoring-commits feed.
 */
export interface CommitOutcomeUnknown {
  readonly status: "unknown";
  readonly commitId: CommitId;
  readonly diagnostics: NonEmptyReadonlyArray<Diagnostic>;
}

/**
 * Host-owned epoch driver. It is not imported by authored Circular programs.
 * `commands` and `trace` expose exact existing protocol values, never a second IR.
 */
export interface ExecutionEpoch {
  readonly epoch: EpochId;
  readonly state: "open" | "completing" | "committed" | "rejected" | "unknown" | "aborting" | "aborted";
  readonly commands: readonly DeclarationContentCommand<EpochAddressDomain>[];
  readonly trace: readonly ExecutionEpochRequest[];
  /** Enqueues one exact content request before returning to synchronous SDK evaluation. */
  emit(command: DeclarationContentCommand<EpochAddressDomain>): CorrelationId;
  /**
   * Awaits emitted requests in order, then validates and commits. A CommitEpoch whose answer
   * is lost is read back from the commit record; when the record does not show it, the
   * outcome is `unknown`, never a rejection.
   */
  complete(): Promise<Result<EpochCommitted> | CommitOutcomeUnknown>;
  /** Aborts the wire epoch; `reason` is local diagnostic context and is not added to the wire payload. */
  abort(reason?: unknown): Promise<Result<EpochAborted>>;
}

/** Opens the mandatory-baseline epoch before authored module evaluation begins. */
export declare function createExecutionEpoch(
  session: Pick<Session, "declarations">,
  begin: BeginEpochCommand<EpochAddressDomain>,
): Promise<Result<ExecutionEpoch>>;
