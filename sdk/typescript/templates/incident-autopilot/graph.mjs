/**
 * The Incident Autopilot template, as one authored program and the commands that deploy it.
 *
 *   webhook mount ─▶ parse/map ─▶ match.ok ─▶ route ─▶ remediate ──┬─(exit != 0)─▶ message ─▶ notify (Slack)
 *                                         └─(exit == 0)─▶ verifycall ─▶ verify
 *                                verify ──┬─(exit == 0)─▶ resolved ─▶ rmessage ─▶ notify (Slack)
 *                                         └─(exit != 0)─▶ vmessage ─▶ notify (Slack)
 *
 *   match.err / route.unmatched / remediate._error ─▶ invalid_incident (alert door)
 *
 * `resolved` is still the terminal observation, and it is now also where the success result is
 * read: the tap republishes the verifier's result unchanged and `rmessage` turns it into the
 * notification an operator sees when the autopilot closed the incident by itself. Before this
 * the success result stopped at the tap, so only failure results ever reached `notify`.
 *
 * Grafana raw notifications enter through wire
 * preprocessing. A step of that chain can fail — malformed JSON fails `parse`, an unreadable
 * `alerts` fails `map` — and such a failure is an envelope-level `Err` that skips the remaining
 * steps and reaches `input_match.event`, where `match` splits the tag. **Wiring `match.err` into
 * the alert door is how this template disposes of those inputs**: the engine does not fold a
 * preprocessing failure into a tool call, and a rejected input must not read as an ordinary one,
 * so the door tells the three lanes apart by their `tool` field. `README.md`'s
 * *Disposition of invalid input* holds that table and the arrivals each lane leaves behind.
 *
 * **One source, two consumers** — the offline test checks every command encodes and names only
 * published ports; `deploy.mjs` sends the same commands to a running daemon. This is the same
 * discipline as the other shipped graphs, for the same reason: writing the graph twice lets the thing
 * that is checked and the thing that is deployed drift apart.
 */

import { edgeKeyFromDeclaration } from "@circular/protocol/declaration";
import { deriveBoundaryPortId } from "@circular/protocol";
import {
  EDGE_ATTRS,
  FLAGS,
  PLACEHOLDER_ENVIRONMENT,
  address,
} from "@circular/protocol/authoring-values";

/**
 * The scope this template authors into — the plan root.
 *
 * Root-flatten migration: a named scope now requires a container actor to stand with it,
 * and a nested frame's exports are not visible at the plan root. This template's vertical
 * lives at the root until the container contract is part of the template story
 * (`crates/engine/tests/circular_daemon.rs`'s incident graphs made the same move).
 */
export const INCIDENT_SCOPE = Object.freeze([]);

const actorKey = (local) => ({ local, scope: INCIDENT_SCOPE.map((segment) => ({ ...segment })) });

/** The mount the webhook ingress injects into: `POST /v1/ingress/incidents`. */
export const INGRESS_MOUNT = "incidents";

const INPUT_ACTOR = "incident_input";
export const INPUT_PORT = deriveBoundaryPortId("inlet", actorKey(INPUT_ACTOR), 0n);

/** The mount whose arrivals are the resolved terminal observations (`arrival.scan` takes it). */
export const RESOLVED_MOUNT = "resolved-door";

/** Invalid ingress is visible even without a notification capability binding. */
export const INVALID_MOUNT = "invalid-incident-door";

export const GRAFANA_TRANSFORM = "'alerts' in event ? "
  + "{'id': string(event.alerts[0].fingerprint), 'tool': 'remediate', 'arguments': "
  + "string(event.alerts[0].labels.fault_flag) + ' ' + string(event.status) + ' ' + string(event.alerts[0].labels.alertname)} : {'id': 'invalid-incident', 'tool': 'unmatched', 'arguments': ''}";

/**
 * The read-only observation mount over the escalation lane (`arrival.scan` takes it).
 *
 * A `result` role is a read-only ref — it observes the notify actor's arrivals without adding
 * an injection point, so watching escalations does not change what the graph means.
 *
 * The door observes the inlet, so every notification the graph produces passes it: the two
 * failure escalations and the resolution. The name is kept because it is the operator-facing
 * lane's name and QA measurements cite it; what changed is that a success now has a
 * notification of its own instead of arriving here only as a record a filter then discarded.
 */
export const ESCALATION_MOUNT = "escalation-door";

/**
 * The logical notification channel. The daemon owns the sink resolution: bind it to Slack with
 * The daemon's `[[notify.channel]]` config binds that name to a Slack vault reference or a
 * local system program. The graph is identical either way.
 */
export const SLACK_CHANNEL = "slack";

/** The tool names the graph declares; the daemon-side allowlist must cover their programs. */
export const TOOL_NAMES = Object.freeze({ remediate: "remediate", verify: "verify" });

/**
 * The actors, parameterized by the two allowlisted programs.
 *
 * `remediator` and `verifier` are absolute paths on the daemon's host. The verifier takes no
 * arguments and inspects the state of the world (what the remediation was supposed to change),
 * not a payload — a verifier that trusts a payload verifies the claim, not the fix.
 */
import { preprocessRecipe } from "../agent-session-monitor/preprocessing.mjs";

function incidentSteps({ remediator, verifier }) {
  return [
    { id: INPUT_ACTOR, actorType: "input", config: { label: INGRESS_MOUNT } },
    {
      id: "decode",
      actorType: "parse",
      config: { decoder: "json", field: "body", arguments: {} },
    },
    { id: "grafana", actorType: "map", config: { transform: GRAFANA_TRANSFORM } },
    { id: "input_match", actorType: "match", config: null },
    { id: "input_route", actorType: "route", config: { at: ["tool"], cases: { remediate: "remediate" } } },
    { id: "invalid_incident", actorType: "alert", config: { predicate: "true", firing_delay: 1n, recovery_delay: 1n } },
    {
      id: "remediate",
      actorType: "tool_executor",
      config: {
        capabilities: { ProcessSpawn: { approval: "none" } },
        tools: {
          [TOOL_NAMES.remediate]: { effect: "spawn", program: remediator, arguments: [] },
        },
      },
    },
    { id: "failed", actorType: "filter", config: { predicate: "event.value.exit != 0" } },
    {
      id: "message",
      actorType: "map",
      config: {
        transform:
          "{'title': 'Incident remediation failed', 'body': 'remediator exited ' + string(event.value.exit)}",
      },
    },
    {
      id: "notify",
      actorType: "notify",
      config: { capabilities: { UserNotify: { approval: "none" } }, channel: SLACK_CHANNEL, minimum_interval: 0n, during_interval: "suppress" },
    },
    { id: "succeeded", actorType: "filter", config: { predicate: "event.value.exit == 0" } },
    {
      id: "verifycall",
      actorType: "map",
      config: {
        transform: "{'id': 'verify-call', 'tool': 'verify', 'arguments': ''}",
      },
    },
    {
      id: "verify",
      actorType: "tool_executor",
      config: {
        capabilities: { ProcessSpawn: { approval: "none" } },
        tools: {
          [TOOL_NAMES.verify]: { effect: "spawn", program: verifier, arguments: [] },
        },
      },
    },
    { id: "verified", actorType: "filter", config: { predicate: "event.value.exit == 0" } },
    { id: "vfailed", actorType: "filter", config: { predicate: "event.value.exit != 0" } },
    {
      id: "vmessage",
      actorType: "map",
      config: {
        transform:
          "{'title': 'Incident verification failed', 'body': 'verifier exited ' + string(event.value.exit)}",
      },
    },
    { id: "resolved", actorType: "tap", config: null },
    {
      id: "rmessage",
      actorType: "map",
      config: {
        transform:
          "{'title': 'Incident resolved', 'body': 'verifier confirmed the remediation: ' + string(event.value.stdout)}",
      },
    },
  ];
}

/** The edges. Parallel fan-out from `remediate.result` and fan-in into `notify.notification`. */
const RECIPE_EDGES = Object.freeze(
  [
    { from: INPUT_ACTOR, fromPort: INPUT_PORT, to: "decode", toPort: "event" },
    { from: "decode", fromPort: "event", to: "grafana", toPort: "event" },
    { from: "grafana", fromPort: "event", to: "input_match", toPort: "event" },
    { from: "input_match", fromPort: "ok", to: "input_route", toPort: "event" },
    { from: "input_match", fromPort: "err", to: "invalid_incident", toPort: "event",
      preprocess: [{ kind: "map", config: { transform: "{'id': 'invalid-incident', 'tool': 'preprocess', 'arguments': event.code}" } }] },
    { from: "input_route", fromPort: "route_remediate", to: "remediate", toPort: "call" },
    { from: "input_route", fromPort: "unmatched", to: "invalid_incident", toPort: "event" },
    { from: "remediate", fromPort: "_error", to: "invalid_incident", toPort: "event",
      preprocess: [{ kind: "map", config: { transform: "{'id': 'invalid-incident', 'tool': 'remediate', 'arguments': event}" } }] },
    { from: "remediate", fromPort: "result", to: "failed", toPort: "event" },
    { from: "failed", fromPort: "event", to: "message", toPort: "event" },
    { from: "message", fromPort: "event", to: "notify", toPort: "notification" },
    { from: "remediate", fromPort: "result", to: "succeeded", toPort: "event" },
    { from: "succeeded", fromPort: "event", to: "verifycall", toPort: "event" },
    { from: "verifycall", fromPort: "event", to: "verify", toPort: "call" },
    { from: "verify", fromPort: "result", to: "verified", toPort: "event" },
    { from: "verified", fromPort: "event", to: "resolved", toPort: "event" },
    { from: "resolved", fromPort: "event", to: "rmessage", toPort: "event" },
    { from: "rmessage", fromPort: "event", to: "notify", toPort: "notification" },
    { from: "verify", fromPort: "result", to: "vfailed", toPort: "event" },
    { from: "vfailed", fromPort: "event", to: "vmessage", toPort: "event" },
    { from: "vmessage", fromPort: "event", to: "notify", toPort: "notification" },
  ].map(Object.freeze),
);

export function incidentActors(options) { return preprocessRecipe(incidentSteps(options), RECIPE_EDGES).actors; }
export const EDGES = Object.freeze(preprocessRecipe(incidentSteps({}), RECIPE_EDGES).edges.map(Object.freeze));

function beginEpochCommand() {
  return {
    kind: "BeginEpoch",
    scope: address(INCIDENT_SCOPE.map((segment) => ({ ...segment }))),
    commitId: new Uint8Array(16),
    expectedRevision: null,
    expectedEnvironment: PLACEHOLDER_ENVIRONMENT,
  };
}

/**
 * Every command the template's epoch sends, in order — without a `CommitEpoch`: the epoch
 * identifier is the daemon's to issue, and the deployer sends the commit with the answer's epoch.
 */
export function incidentCommands(options) {
  const commands = [{ verb: "BeginEpoch", command: beginEpochCommand() }];
  for (const actor of incidentActors(options)) {
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
  for (const [mountName, role, actor, port] of [
    [INGRESS_MOUNT, "request", INPUT_ACTOR, INPUT_PORT],
    [RESOLVED_MOUNT, "result", "resolved", "event"],
    [ESCALATION_MOUNT, "result", "notify", "notification"],
    [INVALID_MOUNT, "result", "invalid_incident", "event"],
  ]) {
    commands.push({
      verb: "UpsertExportMount",
      command: {
        kind: "UpsertExportMount",
        mount: address({ local: mountName, scope: INCIDENT_SCOPE.map((segment) => ({ ...segment })) }),
        declaration: { roles: { [role]: { actor: actorKey(actor), port } } },
      },
    });
  }
  return commands;
}

/** The graph, in the shape the shared deployment loop takes — with no commit of its own. */
export function incidentGraph(options) {
  return {
    label: "incident-autopilot",
    commands: incidentCommands(options),
  };
}
