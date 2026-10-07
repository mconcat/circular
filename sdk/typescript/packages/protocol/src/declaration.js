import {
  annotationDeclarationFromValue,
  annotationDeclarationValue,
  authoringEnvironmentFromValue,
  authoringEnvironmentValue,
  currentAuthoringRevisionFromValue,
  currentAuthoringRevisionValue,
  edgeDeclarationFromValue,
  edgeDeclarationValue,
  edgeKeyFromDeclaration,
  exportDeclarationFromValue,
  exportDeclarationValue,
  actorDeclarationFromValue,
  actorDeclarationValue,
  actorFlagsValue,
  presentationFromValue,
  presentationValue,
  scopeDeclarationFromValue,
  scopeDeclarationValue,
} from "./declaration-values.js";
import {
  declarationAddressFromValue,
  declarationAddressValue,
} from "./declaration-address.js";
import { scopeIdentityFromValue } from "./establishment.js";
import { rejectionFromValue } from "./internal/result-values.js";

const FIELDS = Object.freeze({
  BeginEpoch: ["commit_id", "expected_environment", "expected_revision", "scope"],
  ValidateEpoch: ["epoch"],
  CommitEpoch: ["epoch"],
  AbortEpoch: ["epoch"],
  UpsertActor: ["declaration", "actor"],
  RetireActor: ["actor"],
  UpsertEdge: ["declaration", "edge"],
  RetireEdge: ["edge"],
  UpsertTemplate: ["name", "commands"],
  RetireTemplate: ["name"],
  UpsertScope: ["declaration", "scope"],
  RetireScope: ["scope"],
  MoveToScope: ["actors", "target"],
  UpsertExportMount: ["declaration", "mount"],
  RetireExportMount: ["mount"],
  UpsertAnnotation: ["annotation", "declaration"],
  RetireAnnotation: ["annotation"],
  SetPresentation: ["owner", "presentation"],
  SetFlags: ["flags", "actor"],
  ReplaceAuthoringEnvironment: ["replacement"],
});

function fail(code, message) {
  const error = new TypeError(`${code}: ${message}`);
  error.code = code;
  throw error;
}

function assertBytes(value, label) {
  if (!(value instanceof Uint8Array)) fail("CARRIER_NOT_BYTES", `${label} must be a Uint8Array`);
  if (value.length === 0) fail("CARRIER_EMPTY", `${label} carries at least one byte`);
  return value;
}

function deepEqual(left, right) {
  if (left === right) return true;
  if (left === null || right === null || typeof left !== "object" || typeof right !== "object") return false;
  if (Array.isArray(left) !== Array.isArray(right)) return false;
  const keys = Object.keys(left);
  return keys.length === Object.keys(right).length
    && keys.every(key => Object.hasOwn(right, key) && deepEqual(left[key], right[key]));
}

function revisionValue(value) {
  if (value?.kind === "Absent") return currentAuthoringRevisionValue(null);
  if (value?.kind === "At") return currentAuthoringRevisionValue(value.revision);
  return fail("REVISION_SHAPE", "a current revision is Absent or At(revision)");
}

function revisionFromValue(value) {
  const revision = currentAuthoringRevisionFromValue(value);
  return revision === null
    ? Object.freeze({ kind: "Absent" })
    : Object.freeze({ kind: "At", revision });
}

function exactKeys(value, keys, code) {
  if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) {
    fail(code, "value is an object");
  }
  const expected = new Set(keys);
  for (const key of expected) if (!(key in value)) fail(code, `${key} is absent`);
  for (const key of Object.keys(value)) if (!expected.has(key)) fail(code, `${key} is not published`);
}

function revisionTransitionFromValue(value) {
  exactKeys(value, [
    "authoring_after", "authoring_before", "scope", "topology_after", "topology_before",
  ], "REVISION_TRANSITION");
  return Object.freeze({
    scope: Object.freeze(scopeIdentityFromValue(value.scope)),
    authoringBefore: revisionFromValue(value.authoring_before),
    authoringAfter: revisionFromValue(value.authoring_after),
    topologyBefore: revisionFromValue(value.topology_before),
    topologyAfter: revisionFromValue(value.topology_after),
  });
}

/** Reads the durable physical metadata shared by CommitEpoch and authoring-commits. */
export function acceptedCommitMetadataFromValue(value) {
  exactKeys(value, [
    "after_environment", "before_environment", "cursor", "revisions", "target_scope",
  ], "COMMIT_METADATA");
  if (typeof value.cursor !== "bigint" || value.cursor < 0n || !Array.isArray(value.revisions)) {
    fail("COMMIT_METADATA", "cursor is non-negative Int and revisions is an Array");
  }
  const transitions = value.revisions.map(revisionTransitionFromValue);
  if (transitions.length === 0) fail("COMMIT_METADATA", "revisions is non-empty");
  const revisions = new Map();
  for (const transition of transitions) {
    const keyBytes = JSON.stringify(transition.scope, (_key, item) => (
      typeof item === "bigint" ? `${item}n` : item
    ));
    if ([...revisions.keys()].some((scope) => JSON.stringify(scope, (_key, item) => (
      typeof item === "bigint" ? `${item}n` : item
    )) === keyBytes)) {
      fail("COMMIT_METADATA", "revisions repeats a scope");
    }
    revisions.set(transition.scope, Object.freeze({
      authoringBefore: transition.authoringBefore,
      authoringAfter: transition.authoringAfter,
      topologyBefore: transition.topologyBefore,
      topologyAfter: transition.topologyAfter,
    }));
  }
  return Object.freeze({
    cursor: value.cursor,
    targetScope: Object.freeze(scopeIdentityFromValue(value.target_scope)),
    revisions,
    beforeEnvironment: Object.freeze(authoringEnvironmentFromValue(value.before_environment)),
    afterEnvironment: Object.freeze(authoringEnvironmentFromValue(value.after_environment)),
  });
}

const RECONSTRUCTIVE_KINDS = Object.freeze([
  "UpsertTemplate", "UpsertScope", "UpsertActor", "UpsertEdge", "UpsertExportMount", "UpsertAnnotation", "SetPresentation",
]);
const DELTA_ROWS = new Set([
  ...RECONSTRUCTIVE_KINDS,
  "RetireTemplate", "RetireScope", "RetireActor", "RetireEdge", "RetireExportMount", "RetireAnnotation",
]);

/** Reads one accepted semantic epoch emitted by the lossless authoring feed. */
export function authoringCommitFrameFromValue(value) {
  exactKeys(value, ["delta", "epoch", "metadata"], "AUTHORING_COMMIT_FRAME");
  exactKeys(value.epoch, ["begin", "content", "terminal"], "AUTHORING_COMMIT_EPOCH");
  if (!Array.isArray(value.delta)) fail("AUTHORING_COMMIT_FRAME", "delta is an Array");
  if (!Array.isArray(value.epoch.content)) {
    fail("AUTHORING_COMMIT_EPOCH", "content is an Array");
  }
  const begin = declarationCommandFromValue("BeginEpoch", value.epoch.begin, {
    context: "acceptedHistory", includesKind: true,
  });
  const content = value.epoch.content.map((command) => {
    if (typeof command?.kind !== "string") fail("AUTHORING_COMMIT_EPOCH", "content kind is absent");
    return declarationCommandFromValue(command.kind, command, {
      context: "acceptedHistory", includesKind: true,
    });
  });
  const terminal = declarationCommandFromValue("CommitEpoch", value.epoch.terminal, {
    context: "acceptedHistory", includesKind: true,
  });
  const delta = value.delta.map((row) => {
    if (typeof row?.kind !== "string") fail("AUTHORING_COMMIT_FRAME", "delta row kind is absent");
    if (!DELTA_ROWS.has(row.kind)) fail("AUTHORING_COMMIT_FRAME", `${row.kind} is not a delta row`);
    return declarationCommandFromValue(row.kind, row, {
      context: "acceptedHistory", includesKind: true,
    });
  });
  return Object.freeze({
    epoch: Object.freeze({ begin, content: Object.freeze(content), terminal }),
    metadata: acceptedCommitMetadataFromValue(value.metadata),
    delta: Object.freeze(delta),
  });
}

function exactObject(value, arm, includesKind) {
  if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) {
    fail("DECLARATION_PAYLOAD", `${arm} payload is an object`);
  }
  const expected = new Set(FIELDS[arm] ?? []);
  if (includesKind) expected.add("kind");
  for (const key of expected) {
    if (!(key in value)) fail("DECLARATION_FIELD_MISSING", `${arm}.${key} is absent`);
  }
  for (const key of Object.keys(value)) {
    if (!expected.has(key)) fail("DECLARATION_FIELD_UNKNOWN", `${arm}.${key} is not published`);
  }
}

/** Converts one public declaration command to its shared physical payload Value. */
export function declarationPayloadValue(command, options = {}) {
  const context = options.context ?? "mutation";
  const includeKind = options.includeKind === true;
  const arm = command?.kind;
  if (!(arm in FIELDS)) fail("DECLARATION_ARM", `${String(arm)} is not a declaration arm`);
  let payload;
  switch (arm) {
    case "BeginEpoch": payload = {
      commit_id: assertBytes(command.commitId, "commitId"),
      expected_environment: authoringEnvironmentValue(command.expectedEnvironment),
      expected_revision: revisionValue(command.expectedRevision),
      scope: declarationAddressValue(command.scope, "scope", context),
    }; break;
    case "ValidateEpoch":
    case "CommitEpoch":
    case "AbortEpoch": payload = { epoch: assertBytes(command.epoch, "epoch") }; break;
    case "UpsertActor": payload = {
      declaration: actorDeclarationValue(command.declaration),
      actor: declarationAddressValue(command.actor, "actor", context),
    }; break;
    case "RetireActor": payload = { actor: declarationAddressValue(command.actor, "actor", context) }; break;
    case "UpsertEdge": payload = {
      declaration: edgeDeclarationValue(command.declaration),
      edge: declarationAddressValue(command.edge, "edge", context),
    }; break;
    case "RetireEdge": payload = { edge: declarationAddressValue(command.edge, "edge", context) }; break;
    case "UpsertTemplate": {
      exactObject(command, "UpsertTemplate", true);
      if (typeof command.name !== "string" || !Array.isArray(command.commands)) fail("DECLARATION_PAYLOAD", "Template requires name text and commands Array");
      const commands = command.commands.map(item => {
        const value = declarationPayloadValue(item, { context: "snapshot", includeKind: true });
        reconstructiveCommandFromValue(value);
        return value;
      });
      payload = { name: command.name, commands }; break;
    }
    case "RetireTemplate":
      exactObject(command, "RetireTemplate", true);
      if (typeof command.name !== "string") fail("DECLARATION_PAYLOAD", "Template name is text");
      payload = { name: command.name }; break;
    case "UpsertScope": payload = {
      declaration: scopeDeclarationValue(command.declaration),
      scope: declarationAddressValue(command.scope, "scope", context),
    }; break;
    case "RetireScope": payload = { scope: declarationAddressValue(command.scope, "scope", context) }; break;
    case "MoveToScope": payload = {
      actors: command.actors.map((actor) => declarationAddressValue(actor, "actor", context)),
      target: declarationAddressValue(command.target, "scope", context),
    }; break;
    case "UpsertExportMount": payload = {
      declaration: exportDeclarationValue(command.declaration),
      mount: declarationAddressValue(command.mount, "exportMount", context),
    }; break;
    case "RetireExportMount": payload = { mount: declarationAddressValue(command.mount, "exportMount", context) }; break;
    case "UpsertAnnotation": payload = {
      annotation: declarationAddressValue(command.annotation, "annotation", context),
      declaration: annotationDeclarationValue(command.declaration),
    }; break;
    case "RetireAnnotation": payload = { annotation: declarationAddressValue(command.annotation, "annotation", context) }; break;
    case "SetPresentation": payload = {
      owner: declarationAddressValue(command.owner, "presentationOwner", context),
      presentation: presentationValue(command.presentation),
    }; break;
    case "SetFlags": payload = {
      flags: actorFlagsValue(command.flags),
      actor: declarationAddressValue(command.actor, "actor", context),
    }; break;
    case "ReplaceAuthoringEnvironment": payload = {
      replacement: authoringEnvironmentValue(command.replacement),
    }; break;
    default: return fail("DECLARATION_ARM", `${String(arm)} is not implemented`);
  }
  if (arm === "UpsertEdge" && context === "mutation"
    && !deepEqual(command.edge.value, edgeKeyFromDeclaration(command.declaration))) {
    fail("KEY_DISAGREES_WITH_DECLARATION", "UpsertEdge's key must equal the declaration's from, to, and ordinal");
  }
  return Object.freeze(includeKind ? { kind: arm, ...payload } : payload);
}

/** Reads a physical payload or self-describing snapshot item into the public command union. */
export function declarationCommandFromValue(arm, value, options = {}) {
  const context = options.context ?? "mutation";
  const includesKind = options.includesKind === true;
  exactObject(value, arm, includesKind);
  if (includesKind && value.kind !== arm) {
    fail("DECLARATION_KIND_MISMATCH", `item says ${String(value.kind)}, expected ${arm}`);
  }
  switch (arm) {
    case "BeginEpoch": return Object.freeze({
      kind: arm,
      commitId: value.commit_id,
      expectedEnvironment: authoringEnvironmentFromValue(value.expected_environment),
      expectedRevision: revisionFromValue(value.expected_revision),
      scope: declarationAddressFromValue(value.scope, "scope", context),
    });
    case "ValidateEpoch":
    case "CommitEpoch":
    case "AbortEpoch": return Object.freeze({ kind: arm, epoch: value.epoch });
    case "UpsertActor": return Object.freeze({
      kind: arm,
      declaration: actorDeclarationFromValue(value.declaration),
      actor: declarationAddressFromValue(value.actor, "actor", context),
    });
    case "RetireActor": return Object.freeze({ kind: arm, actor: declarationAddressFromValue(value.actor, "actor", context) });
    case "UpsertEdge": return Object.freeze({
      kind: arm,
      declaration: edgeDeclarationFromValue(value.declaration),
      edge: declarationAddressFromValue(value.edge, "edge", context),
    });
    case "RetireEdge": return Object.freeze({ kind: arm, edge: declarationAddressFromValue(value.edge, "edge", context) });
    case "UpsertTemplate":
      if (typeof value.name !== "string" || !Array.isArray(value.commands)) fail("DECLARATION_PAYLOAD", "Template requires name text and commands Array");
      return Object.freeze({ kind: arm, name: value.name, commands: Object.freeze(value.commands.map(reconstructiveCommandFromValue)) });
    case "RetireTemplate":
      if (typeof value.name !== "string") fail("DECLARATION_PAYLOAD", "Template name is text");
      return Object.freeze({ kind: arm, name: value.name });
    case "UpsertScope": return Object.freeze({
      kind: arm,
      declaration: scopeDeclarationFromValue(value.declaration),
      scope: declarationAddressFromValue(value.scope, "scope", context),
    });
    case "RetireScope": return Object.freeze({ kind: arm, scope: declarationAddressFromValue(value.scope, "scope", context) });
    case "MoveToScope":
      if (!Array.isArray(value.actors)) fail("DECLARATION_PAYLOAD", "MoveToScope.actors is an Array");
      return Object.freeze({
        kind: arm,
        actors: Object.freeze(value.actors.map((actor) => declarationAddressFromValue(actor, "actor", context))),
        target: declarationAddressFromValue(value.target, "scope", context),
      });
    case "UpsertExportMount": return Object.freeze({
      kind: arm,
      declaration: exportDeclarationFromValue(value.declaration),
      mount: declarationAddressFromValue(value.mount, "exportMount", context),
    });
    case "RetireExportMount": return Object.freeze({ kind: arm, mount: declarationAddressFromValue(value.mount, "exportMount", context) });
    case "UpsertAnnotation": return Object.freeze({
      kind: arm,
      annotation: declarationAddressFromValue(value.annotation, "annotation", context),
      declaration: annotationDeclarationFromValue(value.declaration),
    });
    case "RetireAnnotation": return Object.freeze({ kind: arm, annotation: declarationAddressFromValue(value.annotation, "annotation", context) });
    case "SetPresentation": return Object.freeze({
      kind: arm,
      owner: declarationAddressFromValue(value.owner, "presentationOwner", context),
      presentation: presentationFromValue(value.presentation),
    });
    case "SetFlags": return Object.freeze({
      kind: arm,
      flags: actorFlagsValue(value.flags),
      actor: declarationAddressFromValue(value.actor, "actor", context),
    });
    case "ReplaceAuthoringEnvironment": return Object.freeze({
      kind: arm,
      replacement: authoringEnvironmentFromValue(value.replacement),
    });
    default: return fail("DECLARATION_ARM", `${String(arm)} is not implemented`);
  }
}

export function reconstructiveCommandFromValue(value) {
  if (typeof value?.kind !== "string") fail("DECLARATION_KIND_MISSING", "snapshot item has no kind");
  if (!RECONSTRUCTIVE_KINDS.includes(value.kind)) fail("DECLARATION_NOT_RECONSTRUCTIVE", `${value.kind} is not reconstructive`);
  return declarationCommandFromValue(value.kind, value, { context: "snapshot", includesKind: true });
}

/** Reads the Declaration partition's one physical CommandResult sum. */
export function declarationResultFromValue(value, requestKind) {
  let fact;
  if (value === 1n) fact = undefined;
  else if (Array.isArray(value) && value.length === 2 && value[0] === 1n) fact = value[1];
  else if (Array.isArray(value) && value.length === 2 && value[0] === 2n) {
    return rejectionFromValue(value[1]);
  } else {
    fail("COMMAND_RESULT_SHAPE", "CommandResult is Accepted or Rejected");
  }
  const accepted = requestKind === "BeginEpoch"
    ? Object.freeze({ epoch: fact })
    : requestKind === "CommitEpoch"
      ? Object.freeze({ metadata: acceptedCommitMetadataFromValue(fact) })
      : fact;
  return Object.freeze({ status: "accepted", value: accepted });
}

export { declarationAddressValue, declarationAddressFromValue };
export { DEFAULT_EDGE_ATTRS, edgeKeyFromDeclaration } from "./declaration-values.js";
