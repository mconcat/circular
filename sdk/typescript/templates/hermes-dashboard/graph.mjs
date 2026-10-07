/**
 * Hermes Dashboard template.
 *
 *   hermes_logs(listener, one file_tail over the whole fleet)
 *     -> [parse(path) -> parse(body) -> normalize(map)]   (source-adapter.mjs owns all three)
 *     -> line_match(match)
 *          err -> [map] -> unparsed_lines(tap)            banner art and diag dumps, observed
 *          ok  -> by_agent(route at ["agent"])
 *                   route_<agent> -> [map 1.0] -> activity_<agent>(windowed_reduce)
 *                   route_<agent>            -> quiet_<agent>(debounce)
 *                   unmatched -> unknown_agents(tap)      an agent nobody listed, observed
 *          ok  -> [filter(contention) -> map] -> conflicts(tap)
 *
 *   beat(timer every=emission_period) -> [map 0.0] -> activity_<agent>(sample)
 *   activity_<agent>.aggregate -> [map] -> fleet_activity(tap)
 *   quiet_<agent>.event        -> [map] -> stalls(tap)
 *
 * This file reads the normalized envelope only: `agent`, `evidence`, `contention` and three
 * display fields. It holds no Hermes spelling, no log format and no file path — those live
 * wholly in `source-adapter.mjs`, which is the one file a different harness replaces.
 *
 * ## Why the beat exists
 *
 * `windowed_reduce` emits nothing for an empty window, so a silent agent would produce no
 * aggregate at all and a stalled fleet would look exactly like a healthy quiet one on the surface.
 * The `beat` timer feeds every window a 0.0 sample, so each agent's row keeps arriving with
 * `events: 0` while it is stalled: health and death must not look identical. The beat is an
 * authored element of this pipeline measuring an external agent's output — it is not a supervisor
 * heartbeat inferring a Circular actor's liveness.
 *
 * ## Why both `windowed_reduce` and `debounce`
 *
 * They answer two different questions and neither substitutes for the other.
 * `debounce(quiet_window)` emits the last line of an agent that has gone quiet for that long, once
 * per stall episode — that is stall *onset*, and its one-pending-at-a-time state is exactly the
 * flap suppression a burst of log lines needs. `windowed_reduce` answers *how much* each agent
 * produced in the last window, every period, stalled or not — that is the standing dashboard row.
 */

import { edgeKeyFromDeclaration } from "@circular/protocol/declaration";
import {
  EDGE_ATTRS,
  FLAGS,
  PLACEHOLDER_ENVIRONMENT,
  address,
} from "@circular/protocol/authoring-values";
import { preprocessRecipe } from "../agent-session-monitor/preprocessing.mjs";
import {
  CLASSIFICATIONS,
  ERROR_EVIDENCE_FIELDS,
  WARNING_EVIDENCE_FIELDS,
  NORMAL_EVIDENCE_FIELDS,
  classifyNormalized,
} from "../agent-session-monitor/graph.mjs";
import { CONTENTION_PREDICATE, hermesSource } from "./source-adapter.mjs";

export const DASHBOARD_SCOPE = Object.freeze([]);
const actorKey = (local) => ({ local, scope: DASHBOARD_SCOPE.map((segment) => ({ ...segment })) });

export { CLASSIFICATIONS, classifyNormalized };

/**
 * Mount names this template publishes. Five observations and nothing else — there is no ingress
 * mount because the source reads files rather than receiving posts.
 */
export const OBSERVATION_MOUNTS = Object.freeze({
  activity: "hermes-fleet-activity",
  stalls: "hermes-stalls",
  conflicts: "hermes-conflicts",
  unparsed: "hermes-unparsed-lines",
  unknownAgents: "hermes-unknown-agents",
});

/** The view kind each observation carries, all of them already-registered kinds. */
export const OBSERVATION_VIEWS = Object.freeze({
  fleet_activity: "table",
  stalls: "feed",
  conflicts: "table",
  unparsed_lines: "feed",
  unknown_agents: "feed",
});

/**
 * Template defaults. Every number is an example operating value for a five-agent laptop fleet, not
 * a product default: the engine has no silent default for any of these config slots.
 *
 * **The millisecond values are BigInt because the wire distinguishes `Int` from `Float`**, and
 * system-level quantities such as these sit on `Int`. A plain JavaScript number encodes as
 * `Float` and the daemon then refuses activation with `config.window_length is not a millisecond
 * interval` — a refusal that is correct and that this template must not walk into.
 *
 * `root` has no default on purpose — a template that ships one user's absolute path would be a
 * demo string literal, and the glob is the one thing the operator must state.
 */
export const DEFAULTS = Object.freeze({
  logFile: "gateway.log",
  pollMs: 2000n,
  windowMs: 300_000n,
  emissionPeriodMs: 60_000n,
  stallQuietMs: 300_000n,
});

/** The option names the engine reads as `Int` milliseconds. */
export const MILLISECOND_OPTIONS = Object.freeze(["pollMs", "windowMs", "emissionPeriodMs", "stallQuietMs"]);

const AGENT_NAME = /^[a-z][a-z0-9_]*$/;

function milliseconds(name, value) {
  const millis = typeof value === "bigint" ? value : BigInt(value);
  if (typeof value === "number" && !Number.isSafeInteger(value)) {
    throw new Error(`${name} must be a whole number of milliseconds`);
  }
  if (millis <= 0n) throw new Error(`${name} must be a positive number of milliseconds`);
  return millis;
}

/**
 * Normalizes and checks the authored options once, so a bad value is refused here rather than at
 * daemon activation with a config rejection that names a port instead of a template argument.
 */
export function hermesOptions(options) {
  const resolved = { ...DEFAULTS, ...options };
  if (typeof resolved.root !== "string" || resolved.root.length === 0 || !resolved.root.startsWith("/")) {
    throw new Error("hermes-dashboard requires an absolute `root`: the profiles directory holding one subdirectory per agent");
  }
  const agents = [...(resolved.agents ?? [])];
  if (agents.length === 0) throw new Error("hermes-dashboard requires a non-empty `agents` list");
  for (const agent of agents) {
    if (!AGENT_NAME.test(agent)) {
      throw new Error(`agent name is not a port-safe name: ${JSON.stringify(agent)}`);
    }
  }
  if (new Set(agents).size !== agents.length) throw new Error("agent names repeat");
  const root = resolved.root.replace(/\/+$/, "");
  for (const name of MILLISECOND_OPTIONS) resolved[name] = milliseconds(name, resolved[name]);
  return Object.freeze({
    ...resolved,
    root,
    agents: Object.freeze(agents),
    glob: resolved.glob ?? `${root}/*/logs/${resolved.logFile}`,
  });
}

const evidenceOf = (field) => `event.evidence.${field}`;
const disjunction = (fields) => fields.map(evidenceOf).join(" || ");

/**
 * `agent-session-monitor`'s severity priority, spelled over the same evidence booleans, but
 * preserving the rest of the envelope: the conflict surface needs to say *which agent* and *which
 * line*, and that template's own transform drops those because its rail carries no agent.
 */
export const CONFLICT_ROW_TRANSFORM = [
  "{'agent': event.agent, 'at': event.at, 'level': event.level, 'logger': event.logger,",
  " 'message': event.message, 'contention': event.contention, 'classification': ",
  `((${disjunction(ERROR_EVIDENCE_FIELDS)}) ? 'error' : `,
  `((${disjunction(WARNING_EVIDENCE_FIELDS)}) ? 'warning' : `,
  `((${disjunction(NORMAL_EVIDENCE_FIELDS)}) ? 'normal' : 'unclassified')))}`,
].join("");

/** Each tailed line counts once; the reduce below sums these. */
export const LINE_SAMPLE_TRANSFORM = "1.0";
/** Each beat keeps the window non-empty without changing the count. */
export const BEAT_SAMPLE_TRANSFORM = "0.0";
/** `acc` and `sample` are the two bindings a `windowed_reduce` expression may name. */
export const ACTIVITY_REDUCE = "acc + sample";
/** `seed` is mandatory: there is no silent zero. */
export const ACTIVITY_SEED = 0.0;

/** The aggregate is a bare number, so the wire into the surface names the agent it belongs to. */
export const activityRowTransform = (agent) =>
  `{'agent': '${agent}', 'events': event, 'stalled': event <= 0.0}`;

/** Debounce forwards the last line verbatim; the wire into the surface says what it means. */
export const stallRowTransform = (agent, quietMs) =>
  `{'agent': '${agent}', 'quiet_ms': ${quietMs}, 'last_at': event.at,`
  + " 'last_logger': event.logger, 'last_message': event.message}";

/** `match` consumed the envelope tag; keep its published code and detail under a marker. */
export const UNPARSED_TRANSFORM =
  "{'classification': 'unclassified', 'code': event.code, 'detail': event.detail}";

function recipe(options) {
  const { agents, root, glob, pollMs, windowMs, emissionPeriodMs, stallQuietMs } = options;
  const source = hermesSource({ root, glob, pollMs });

  const steps = [
    ...source.actors,
    { id: "beat", actorType: "timer", config: { every: emissionPeriodMs } },
    { id: "line_match", actorType: "match", config: null },
    {
      id: "by_agent",
      actorType: "route",
      config: { at: ["agent"], cases: Object.fromEntries(agents.map((agent) => [agent, agent])) },
    },
    ...agents.flatMap((agent) => [
      {
        id: `activity_${agent}`,
        actorType: "windowed_reduce",
        config: {
          window_length: windowMs,
          emission_period: emissionPeriodMs,
          reduce: ACTIVITY_REDUCE,
          seed: ACTIVITY_SEED,
        },
      },
      { id: `quiet_${agent}`, actorType: "debounce", config: { quiet_window: stallQuietMs } },
    ]),
    { id: "fleet_activity", actorType: "tap", config: null },
    { id: "stalls", actorType: "tap", config: null },
    { id: "conflicts", actorType: "tap", config: null },
    { id: "unparsed_lines", actorType: "tap", config: null },
    { id: "unknown_agents", actorType: "tap", config: null },
  ];

  const edges = [
    ...source.edges,
    { from: "line_match", fromPort: "ok", to: "by_agent", toPort: "event" },
    {
      from: "line_match",
      fromPort: "err",
      to: "unparsed_lines",
      toPort: "event",
      preprocess: [{ kind: "map", config: { transform: UNPARSED_TRANSFORM } }],
    },
    {
      from: "line_match",
      fromPort: "ok",
      to: "conflicts",
      toPort: "event",
      preprocess: [
        { kind: "filter", config: { predicate: CONTENTION_PREDICATE } },
        { kind: "map", config: { transform: CONFLICT_ROW_TRANSFORM } },
      ],
    },
    { from: "by_agent", fromPort: "unmatched", to: "unknown_agents", toPort: "event" },
    ...agents.flatMap((agent) => [
      {
        from: "by_agent",
        fromPort: `route_${agent}`,
        to: `activity_${agent}`,
        toPort: "sample",
        preprocess: [{ kind: "map", config: { transform: LINE_SAMPLE_TRANSFORM } }],
      },
      {
        from: "beat",
        fromPort: "tick",
        to: `activity_${agent}`,
        toPort: "sample",
        preprocess: [{ kind: "map", config: { transform: BEAT_SAMPLE_TRANSFORM } }],
      },
      {
        from: `activity_${agent}`,
        fromPort: "aggregate",
        to: "fleet_activity",
        toPort: "event",
        preprocess: [{ kind: "map", config: { transform: activityRowTransform(agent) } }],
      },
      { from: "by_agent", fromPort: `route_${agent}`, to: `quiet_${agent}`, toPort: "event" },
      {
        from: `quiet_${agent}`,
        fromPort: "event",
        to: "stalls",
        toPort: "event",
        preprocess: [{ kind: "map", config: { transform: stallRowTransform(agent, stallQuietMs) } }],
      },
    ]),
  ];

  return preprocessRecipe(steps, edges);
}

/** The actors and edges for one authored option set, both already folded by `preprocessRecipe`. */
export function hermesDashboardParts(options) {
  const resolved = hermesOptions(options);
  const { actors, edges } = recipe(resolved);
  return Object.freeze({
    options: resolved,
    actors: Object.freeze(actors.map(Object.freeze)),
    edges: Object.freeze(edges.map(Object.freeze)),
  });
}

/** The observation actor each mount publishes. */
const MOUNTED_TAPS = Object.freeze([
  ["activity", "fleet_activity"],
  ["stalls", "stalls"],
  ["conflicts", "conflicts"],
  ["unparsed", "unparsed_lines"],
  ["unknownAgents", "unknown_agents"],
]);

function beginEpochCommand() {
  return {
    kind: "BeginEpoch",
    scope: address(DASHBOARD_SCOPE.map((segment) => ({ ...segment }))),
    commitId: new Uint8Array(16),
    expectedRevision: null,
    expectedEnvironment: PLACEHOLDER_ENVIRONMENT,
  };
}

export function hermesDashboardCommands(options) {
  const { actors, edges } = hermesDashboardParts(options);
  const commands = [{ verb: "BeginEpoch", command: beginEpochCommand() }];
  for (const actor of actors) {
    commands.push({
      verb: "UpsertActor",
      command: {
        kind: "UpsertActor",
        actor: address(actorKey(actor.id)),
        declaration: { actorType: actor.actorType, config: actor.config, flags: FLAGS },
      },
    });
  }
  for (const edge of edges) {
    const declaration = {
      from: { actor: actorKey(edge.from), port: edge.fromPort },
      to: { actor: actorKey(edge.to), port: edge.toPort },
      ordinal: 0,
      attrs: edge.preprocess?.length
        ? { ...EDGE_ATTRS, preprocess: structuredClone(edge.preprocess) }
        : EDGE_ATTRS,
    };
    commands.push({
      verb: "UpsertEdge",
      command: { kind: "UpsertEdge", edge: address(edgeKeyFromDeclaration(declaration)), declaration },
    });
  }
  for (const [mount, local] of MOUNTED_TAPS) {
    commands.push({
      verb: "UpsertExportMount",
      command: {
        kind: "UpsertExportMount",
        mount: address({ local: OBSERVATION_MOUNTS[mount], scope: [] }),
        declaration: { roles: { result: { actor: actorKey(local), port: "event" } } },
      },
    });
  }
  for (const [local, kind] of Object.entries(OBSERVATION_VIEWS)) {
    commands.push({
      verb: "SetPresentation",
      command: {
        kind: "SetPresentation",
        owner: { actor: address(actorKey(local)) },
        presentation: { collapsed: false, label: local, view: { kind, config: null } },
      },
    });
  }
  return commands;
}

export function hermesDashboardGraph(options) {
  return { label: "hermes-dashboard", commands: hermesDashboardCommands(options) };
}
