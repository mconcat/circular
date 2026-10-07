import type { DeclarationCommand, EpochAddressDomain, Result as ProtocolResult } from "@circular/protocol";
import type { OwnerLocalSession } from './owner-local-session.js';

/** A refusal during an authored epoch's lifetime. */
export declare class EpochRunnerError extends Error {
  constructor(code: string, message: string);
  readonly code: string;
}

/** Reads the wire revision bytes; accepted genesis is Absent and rejected snapshots throw. */
export declare function currentAuthoringRevision(
  session: { authoringSnapshot(scope: unknown, pageLimit: number): Promise<unknown> },
  scope: unknown,
  options?: { readonly errorFor?: (code: string, message: string) => Error },
): Promise<
  { readonly kind: "Absent" }
  | { readonly kind: "At"; readonly revision: Uint8Array }
>;

/**
 * Runs a hand-authored BeginEpoch and content list through Validate and Commit (or Abort).
 * Every attempt receives a fresh 16-byte commit id. An omitted expectedRevision is read
 * from the root snapshot; an explicit edit revision is preserved.
 * After commit, waits for recorded adoption before afterCommit or returning success.
 * A failed or closed wait throws with commitAccepted: true and the coded result in adoption.
 *
 * A CommitEpoch whose answer is lost is read back from the authoring-commits record. When
 * the record does not show it, the thrown error's code is `EPOCH_COMMIT_UNKNOWN` (through
 * `errorFor("COMMIT_UNKNOWN", …)`), with `commitUnknown: true` and the `commitId` to look
 * for — the daemon may have committed; it is not a rejection.
 */
export declare function runValidatedEpoch<Result extends ProtocolResult<unknown> = ProtocolResult<unknown>, Witness = unknown>(
  session: Pick<OwnerLocalSession, 'hold'> & {
    authoringSnapshot(scope: unknown, pageLimit: number): Promise<unknown>;
    declare(command: DeclarationCommand<EpochAddressDomain>): Promise<Result>;
  },
  commands: readonly {
    readonly verb: string;
    readonly command: { readonly kind: string; readonly [key: string]: unknown };
  }[],
  options?: {
    readonly errorFor?: (code: string, message: string) => Error;
    readonly expectedRevision?: Awaited<ReturnType<typeof currentAuthoringRevision>>;
    readonly commit?: boolean;
    readonly beforeCommit?: () => unknown;
    readonly afterCommit?: (committed: Result) => Witness | Promise<Witness>;
  },
): Promise<Readonly<{
  validated: true;
  committed: false;
  commands: number;
  acceptedCommands: number;
} | {
  validated: true;
  committed: true;
  commands: number;
  acceptedCommands: number;
  commit: Result;
  witness: NonNullable<Witness> | null;
}>>;
