/**
 * Agent Session Monitor template.
 *
 *   agent-otel-{logs,metrics}
 *     -> input -> parse -> normalize(source boundary) -> classify(generic)
 *     -> match (logs: err -> invalid warning door; ok -> route)
 *     -> route(normal|warning|error|unclassified|unmatched) -> observed tap
 *
 * The source boundary is wholly owned by source-adapter.mjs. This file only consumes normalized
 * evidence and therefore contains no Claude/Codex or raw OTLP field dependency.
 */

import { edgeKeyFromDeclaration } from "@circular/protocol/declaration";
import {
  EDGE_ATTRS,
  FLAGS,
  PLACEHOLDER_ENVIRONMENT,
  address,
} from "@circular/protocol/authoring-values";
import { SIGNALS, sourceAdapter } from "./source-adapter.mjs";

export const MONITOR_SCOPE = Object.freeze([]);
const actorKey = (local) => ({ local, scope: MONITOR_SCOPE.map((segment) => ({ ...segment })) });

export const OBSERVATION_MOUNTS = Object.freeze({
  logs: "agent-observed-logs",
  metrics: "agent-observed-metrics",
});

export const CLASSIFICATIONS = Object.freeze(["normal", "warning", "error", "unclassified"]);
export const ROUTE_CASES = Object.freeze(Object.fromEntries(CLASSIFICATIONS.map((name) => [name, name])));

export const ERROR_EVIDENCE_FIELDS = Object.freeze(["error"]);
export const WARNING_EVIDENCE_FIELDS = Object.freeze(["warning"]);
export const NORMAL_EVIDENCE_FIELDS = Object.freeze(["normal"]);

const evidence = (field) => `('evidence' in event && !(event.evidence in [null]) && '${field}' in event.evidence && event.evidence.${field} in [true])`;
const disjunction = (fields) => fields.map(evidence).join(" || ");

/** The generic classifier: only normalized evidence crosses this boundary. */
export const CLASSIFICATION_TRANSFORM = [
  "{'signal': ('signal' in event ? string(event.signal) : ''), 'recognized': ('recognized' in event && event.recognized in [true]), 'classification': ",
  `((${disjunction(ERROR_EVIDENCE_FIELDS)}) ? 'error' : `,
  `((${disjunction(WARNING_EVIDENCE_FIELDS)}) ? 'warning' : `,
  `((${disjunction(NORMAL_EVIDENCE_FIELDS)}) ? 'normal' : 'unclassified'))), `,
  `'evidence': {'error': ${evidence("error")}, 'warning': ${evidence("warning")}, 'normal': ${evidence("normal")}}}`,
].join("");

export function classifyNormalized(normalized) {
  const has = (fields) => fields.some((field) => normalized?.evidence?.[field] === true);
  const classification = has(ERROR_EVIDENCE_FIELDS)
    ? "error"
    : has(WARNING_EVIDENCE_FIELDS)
      ? "warning"
      : has(NORMAL_EVIDENCE_FIELDS)
        ? "normal"
        : "unclassified";
  return { ...normalized, classification };
}

import { preprocessRecipe } from "./preprocessing.mjs";

const adapter = sourceAdapter({ actorKey });
export const SOURCE_ADAPTER = adapter;

const CLASSIFIER_ACTORS = SIGNALS.flatMap((signal) => [
  {
    id: `classify_${signal}`,
    actorType: "map",
    config: { transform: CLASSIFICATION_TRANSFORM },
  },
  {
    id: `by_class_${signal}`,
    actorType: "route",
    config: { at: ["classification"], cases: ROUTE_CASES },
  },
  { id: `observed_${signal}`, actorType: "tap", config: null },
]);

const RECIPE_STEPS = [...adapter.actors, ...CLASSIFIER_ACTORS,
  { id: "classify_logs_match", actorType: "match", config: null },
  { id: "invalid_logs", actorType: "tap", config: null },
];

const CLASSIFIER_EDGES = SIGNALS.flatMap((signal) => [
  { from: `classify_${signal}`, fromPort: "event", to: signal === "logs" ? "classify_logs_match" : `by_class_${signal}`, toPort: "event" },
  ...CLASSIFICATIONS.map((classification) => ({
    from: `by_class_${signal}`,
    fromPort: `route_${classification}`,
    to: `observed_${signal}`,
    toPort: "event",
  })),
  {
    from: `by_class_${signal}`,
    fromPort: "unmatched",
    to: `observed_${signal}`,
    toPort: "event",
  },
]);

const recipe = preprocessRecipe(RECIPE_STEPS, [...adapter.edges, ...CLASSIFIER_EDGES,
  { from: "classify_logs_match", fromPort: "ok", to: "by_class_logs", toPort: "event" },
  { from: "classify_logs_match", fromPort: "err", to: "invalid_logs", toPort: "event",
    preprocess: [{ kind: "map", config: {
      transform: "{'classification': 'warning', 'code': event.code, 'detail': event.detail}",
    } }],
  },
]);
export const ACTORS = Object.freeze(recipe.actors.map(Object.freeze));
export const EDGES = Object.freeze(recipe.edges.map(Object.freeze));

function beginEpochCommand() {
  return {
    kind: "BeginEpoch",
    scope: address(MONITOR_SCOPE.map((segment) => ({ ...segment }))),
    commitId: new Uint8Array(16),
    expectedRevision: null,
    expectedEnvironment: PLACEHOLDER_ENVIRONMENT,
  };
}

export function agentSessionMonitorCommands() {
  const commands = [{ verb: "BeginEpoch", command: beginEpochCommand() }];
  for (const actor of ACTORS) {
    commands.push({
      verb: "UpsertActor",
      command: {
        kind: "UpsertActor",
        actor: address(actorKey(actor.id)),
        declaration: { actorType: actor.actorType, config: actor.config, flags: FLAGS },
      },
    });
  }
  for (const edge of EDGES) {
    const declaration = {
      from: { actor: actorKey(edge.from), port: edge.fromPort },
      to: { actor: actorKey(edge.to), port: edge.toPort },
      ordinal: 0,
      attrs: edge.preprocess?.length ? { ...EDGE_ATTRS, preprocess: structuredClone(edge.preprocess) } : EDGE_ATTRS,
    };
    commands.push({
      verb: "UpsertEdge",
      command: { kind: "UpsertEdge", edge: address(edgeKeyFromDeclaration(declaration)), declaration },
    });
  }
  for (const mount of adapter.mounts) {
    commands.push({
      verb: "UpsertExportMount",
      command: {
        kind: "UpsertExportMount",
        mount: address({ local: mount.name, scope: [] }),
        declaration: { roles: { [mount.role]: { actor: actorKey(mount.actor), port: mount.port } } },
      },
    });
  }
  for (const signal of SIGNALS) {
    commands.push({
      verb: "UpsertExportMount",
      command: {
        kind: "UpsertExportMount",
        mount: address({ local: OBSERVATION_MOUNTS[signal], scope: [] }),
        declaration: { roles: { result: { actor: actorKey(`observed_${signal}`), port: "event" } } },
      },
    });
  }
  commands.push({
    verb: "UpsertExportMount",
    command: {
      kind: "UpsertExportMount",
      mount: address({ local: "agent-invalid-logs", scope: [] }),
      declaration: { roles: { result: { actor: actorKey("invalid_logs"), port: "event" } } },
    },
  });
  return commands;
}

export function agentSessionMonitorGraph() {
  return { label: "agent-session-monitor", commands: agentSessionMonitorCommands() };
}
