/** One owner-local authoring epoch lifetime, shared by edits and template deployers. */

import { EpochLifetime } from "./internal/epoch-lifetime.js";
import { CommitUnanswered, unansweredCommitOutcome } from "./internal/commit-outcome.js";
import { waitForAdoption } from './adoption.js';
const SNAPSHOT_PAGE_LIMIT = 256;

export class EpochRunnerError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

const defaultErrorFor = (code, message) => new EpochRunnerError(`EPOCH_${code}`, message);
const isAccepted = (result) => result?.status === "accepted";
const acceptedEpochOf = (result) => result?.value?.epoch;
const jsonable = (_key, value) => {
  if (typeof value === "bigint") return value.toString(10);
  if (value instanceof Uint8Array) return "<bytes>";
  return value;
};

function rejectionText(result) {
  try {
    return JSON.stringify(result, jsonable);
  } catch {
    return String(result);
  }
}

function reject(errorFor, code, message) {
  throw errorFor(code, message);
}

function rootScopeOf(begin, errorFor) {
  if (begin?.scope?.arm !== "absolute" || !Array.isArray(begin.scope.value)) {
    reject(errorFor, "BEGIN_SCOPE", "BeginEpoch must carry an absolute scope address");
  }
  return begin.scope.value;
}

export async function currentAuthoringRevision(session, scope, options = {}) {
  const errorFor = options.errorFor ?? defaultErrorFor;
  const snapshot = await session.authoringSnapshot(scope, SNAPSHOT_PAGE_LIMIT);
  if (snapshot.status === "rejected") {
    reject(errorFor, "SNAPSHOT_REJECTED", rejectionText(snapshot));
  }
  if (snapshot.status !== "accepted") {
    reject(
      errorFor,
      "SNAPSHOT_PARTIAL",
      snapshot.diagnostic?.message ?? "authoring snapshot ended without Complete",
    );
  }
  const revision = snapshot.value?.anchor?.authoringRevision;
  if (revision?.kind !== "Absent"
    && !(revision?.kind === "At" && revision.revision instanceof Uint8Array)) {
    reject(errorFor, "SNAPSHOT_REVISION", "authoring snapshot carried no current revision");
  }
  return revision;
}

function splitEpoch(commands, errorFor) {
  if (!Array.isArray(commands) || commands.length === 0) {
    reject(errorFor, "COMMAND_SEQUENCE", "an epoch needs one leading BeginEpoch command");
  }
  const [opening, ...content] = commands;
  if (opening?.verb !== "BeginEpoch" || opening.command?.kind !== "BeginEpoch") {
    reject(errorFor, "COMMAND_SEQUENCE", "the first authored command must be BeginEpoch");
  }
  for (const entry of content) {
    if (entry?.verb !== entry?.command?.kind) {
      reject(errorFor, "COMMAND_SEQUENCE", "each authored verb must match its command kind");
    }
    if (["BeginEpoch", "ValidateEpoch", "CommitEpoch", "AbortEpoch"].includes(entry.verb)) {
      reject(errorFor, "COMMAND_SEQUENCE", `${entry.verb} is owned by the epoch runner`);
    }
  }
  return { opening, content };
}

/**
 * Runs an authored command list through its whole epoch lifetime.
 *
 * A deploy leaves `expectedRevision` unspecified, so this runner snapshots the current CAS word.
 * An edit supplies the revision from the document it is editing; a newer concurrent commit then
 * remains a conflict instead of being silently adopted. Every attempt receives a fresh commit id.
 */
export async function runValidatedEpoch(session, commands, options = {}) {
  const errorFor = options.errorFor ?? defaultErrorFor;
  const { opening, content } = splitEpoch(commands, errorFor);
  const expectedRevision = Object.hasOwn(options, "expectedRevision")
    ? options.expectedRevision
    : await currentAuthoringRevision(session, rootScopeOf(opening.command, errorFor), { errorFor });
  const begin = {
    ...opening.command,
    commitId: globalThis.crypto.getRandomValues(new Uint8Array(16)),
    expectedRevision,
  };
  const commit = options.commit ?? true;
  const opened = await session.declare(begin);
  const epoch = acceptedEpochOf(opened);
  if (!isAccepted(opened) || !(epoch instanceof Uint8Array)) {
    reject(errorFor, "BEGIN_REJECTED", rejectionText(opened));
  }
  const lifetime = new EpochLifetime(async kind => {
    let result;
    try {
      result = await session.declare({ kind, epoch });
    } catch (error) {
      if (kind === "CommitEpoch") throw new CommitUnanswered(error);
      throw error;
    }
    if (!isAccepted(result)) {
      reject(errorFor, `${kind.slice(0, -5).toUpperCase()}_REJECTED`, rejectionText(result));
    }
    return result;
  }, { unanswered: error => unansweredCommitOutcome(session, begin.commitId, error) });
  try {
    const committed = await lifetime.complete(async () => {
      for (const { verb, command } of content) {
        const result = await session.declare(command);
        if (!isAccepted(result)) {
          reject(errorFor, "DECLARATION_REJECTED", `${verb}: ${rejectionText(result)}`);
        }
      }
    }, { commit, beforeCommit: () => options.beforeCommit?.() });
    if (!commit) {
      return Object.freeze({
        validated: true,
        committed: false,
        commands: content.length,
        acceptedCommands: content.length + 3,
      });
    }
    if (committed?.status === "unknown") {
      const error = errorFor("COMMIT_UNKNOWN", committed.diagnostics[0].message);
      error.commitUnknown = true;
      error.commitId = committed.commitId;
      throw error;
    }

    const adoption = await waitForAdoption(session, committed.value.metadata.cursor);
    if (adoption.status !== 'adopted') {
      const error = errorFor(`ADOPTION_${adoption.status.toUpperCase()}`,
        `commit accepted; adoption ${adoption.status} (code ${adoption.code})`);
      error.adoption = adoption;
      throw error;
    }
    const witness = await options.afterCommit?.(committed) ?? null;
    return Object.freeze({
      validated: true,
      committed: true,
      commands: content.length,
      acceptedCommands: content.length + 3,
      commit: committed,
      witness,
    });
  } catch (error) {
    if (lifetime.state === "committed") error.commitAccepted = true;
    throw error;
  }
}
