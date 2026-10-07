
const SNAPSHOT_PAGE_LIMIT = 256;

/** Marks a CommitEpoch request that was submitted and never answered. */
export class CommitUnanswered extends Error {
  constructor(cause) {
    super(`CommitEpoch was not answered: ${reasonOf(cause)}`);
    this.name = "CommitUnanswered";
    this.cause = cause;
  }
}

/** The cause's own text, with its code once (a SessionError message already starts with it). */
function reasonOf(cause) {
  const message = String(cause?.message ?? cause);
  return cause?.code && !message.startsWith(`${cause.code}:`) ? `${cause.code}: ${message}` : message;
}

const hex = (bytes) => Array.from(bytes ?? [], (byte) => byte.toString(16).padStart(2, "0")).join("");

const sameBytes = (left, right) => left instanceof Uint8Array && right instanceof Uint8Array
  && left.length === right.length && left.every((byte, index) => byte === right[index]);

/** The unknown outcome: no Result arrived, and the record did not show the commit. */
export function commitUnknown(commitId, unanswered) {
  const reason = reasonOf(unanswered?.cause ?? unanswered);
  return Object.freeze({
    status: "unknown",
    commitId,
    diagnostics: Object.freeze([Object.freeze({
      code: 0,
      message: `CommitEpoch was sent but its answer did not arrive (${reason}). `
        + "Whether it committed is not known; it was not rejected. "
        + `Before retrying, look for CommitId ${hex(commitId)} in the authoring-commits feed, `
        + "or read the authoring snapshot: its revision moves when the commit lands.",
      hint: null,
      at: null,
    })]),
  });
}

/**
 * Reads the durable commit record for `commitId`: the accepted CommitEpoch value
 * (`{ status: "accepted", value: { metadata } }`) when the record holds it, otherwise null.
 *
 * It reads the latest committed epoch and then waits one request window for the next one, so
 * a commit that landed just before or lands just after the lost answer is found. Anything the
 * session cannot read here leaves the outcome unknown; this never turns into a rejection.
 */
export async function recordedCommit(session, commitId) {
  if (typeof session?.authoringSnapshot !== "function" || typeof session?.authoringCommits !== "function") {
    return null;
  }
  try {
    const snapshot = await session.authoringSnapshot([], SNAPSHOT_PAGE_LIMIT);
    const cursor = snapshot?.status === "accepted" ? snapshot.value?.anchor?.cursor : undefined;
    if (typeof cursor !== "bigint") return null;
    const feed = await session.authoringCommits([], cursor > 0n ? cursor - 1n : 0n);
    try {
      await feed.grant(2n);
      for (let read = 0; read < 2; read += 1) {
        const next = await feed.next();
        if (next === null || next?.kind !== "Commit") return null;
        if (sameBytes(next.value?.epoch?.begin?.commitId, commitId)) {
          return Object.freeze({
            status: "accepted",
            value: Object.freeze({ metadata: next.value.metadata }),
          });
        }
      }
      return null;
    } finally {
      await Promise.resolve(feed.unsubscribe?.()).catch(() => feed.release?.());
    }
  } catch {
    return null;
  }
}

/** The outcome of an unanswered CommitEpoch: the record's commit, or unknown. */
export async function unansweredCommitOutcome(session, commitId, unanswered) {
  return await recordedCommit(session, commitId) ?? commitUnknown(commitId, unanswered);
}
