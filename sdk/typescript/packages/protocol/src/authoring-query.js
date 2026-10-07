import { scopeIdentityFromValue, scopeIdentityValue } from "./establishment.js";
import { authoringCommitFrameFromValue } from "./declaration.js";
import {
  authoringEnvironmentFromValue,
  currentAuthoringRevisionFromValue,
} from "./declaration-values.js";
import { reconstructiveCommandFromValue } from "./declaration.js";

function fail(code, message) {
  const error = new TypeError(`${code}: ${message}`);
  error.code = code;
  throw error;
}

function currentRevision(value) {
  const revision = currentAuthoringRevisionFromValue(value);
  return revision === null
    ? Object.freeze({ kind: "Absent" })
    : Object.freeze({ kind: "At", revision });
}

function rejection(value) {
  if (value === null || typeof value !== "object" || typeof value.code !== "bigint") {
    fail("QUERY_REJECTION", "query rejection is malformed");
  }
  return Object.freeze({
    status: "rejected",
    reason: "Invalid",
    diagnostics: Object.freeze([Object.freeze({
      code: Number(value.code),
      message: String(value.message),
      hint: value.hint ?? null,
      at: value.at ?? null,
    })]),
  });
}

export function authoringSnapshotArgumentsValue(scope) {
  return Object.freeze({ scope: scopeIdentityValue(scope) });
}

export function authoringCommitArgumentsValue(scope, after) {
  if (typeof after !== "bigint" || after < 0n) {
    fail("AUTHORING_COMMIT_AFTER", "after is a non-negative StructureCursor Int");
  }
  return Object.freeze({ after, scope: scopeIdentityValue(scope) });
}

export { authoringCommitFrameFromValue };

export function authoringSnapshotAnchorFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail("SNAPSHOT_ANCHOR", "authoring snapshot anchor is an object");
  }
  return Object.freeze({
    scope: Object.freeze(scopeIdentityFromValue(value.scope)),
    authoringRevision: currentRevision(value.authoring_revision),
    topologyRevision: currentRevision(value.topology_revision),
    cursor: value.cursor,
    environment: Object.freeze(authoringEnvironmentFromValue(value.environment)),
  });
}

export function authoringSnapshotQueryResultFromValue(value) {
  if (!Array.isArray(value) || value.length !== 2 || typeof value[0] !== "bigint") {
    fail("QUERY_RESULT", "QueryResult is a two-part sum");
  }
  if (value[0] === 2n) return rejection(value[1]);
  if (value[0] !== 1n || value[1] === null || typeof value[1] !== "object") {
    fail("QUERY_RESULT", "QueryResult page arm is malformed");
  }
  const page = value[1];
  if (!Array.isArray(page.items)) fail("QUERY_ITEMS", "query page items is an Array");
  const base = {
    anchor: authoringSnapshotAnchorFromValue(page.anchor),
    items: Object.freeze(page.items.map(reconstructiveCommandFromValue)),
  };
  if (page.terminal === 2n) {
    return Object.freeze({ status: "accepted", value: Object.freeze({ ...base, terminal: "Complete" }) });
  }
  if (Array.isArray(page.terminal) && page.terminal.length === 2 && page.terminal[0] === 1n) {
    return Object.freeze({
      status: "accepted",
      value: Object.freeze({ ...base, terminal: "More", next: page.terminal[1] }),
    });
  }
  if (Array.isArray(page.terminal) && page.terminal.length === 2 && page.terminal[0] === 3n) {
    return Object.freeze({
      status: "accepted",
      value: Object.freeze({
        ...base,
        terminal: "Diagnostic",
        diagnostic: Object.freeze({
          code: Number(page.terminal[1]),
          message: "The authoring snapshot page ended diagnostically.",
          hint: null,
          at: null,
        }),
      }),
    });
  }
  return fail("QUERY_TERMINAL", "query page terminal is unassigned");
}
