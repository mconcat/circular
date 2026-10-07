/**
 * The only Hermes-specific boundary in the Hermes Dashboard template.
 *
 * Everything this file knows about Hermes lives in two regular expressions, one level table and
 * one contention-marker table — all of them authored `parse`/`map` configuration on a wire, never
 * an engine primitive. `graph.mjs` downstream reads the normalized envelope only and contains no
 * Hermes spelling. Replacing Hermes with another log-writing harness means replacing this file's
 * `hermesSource()` topology and its two patterns while preserving the normalized contract
 *
 *   {agent, log, at, level, logger, message, session, recognized, evidence{}, contention{}}
 *
 * ## Format source
 *
 * Measured on this host, 2026-09-19: Hermes Agent 0.20.6
 * (`hermes-agent/pyproject.toml` version = "0.20.6"; Nous Research `hermes-agent`), installed at
 * `~/hermes-fleet/hermes-agent` (stock) and `~/hermes-meerkat/hermes-agent` (OAuth build).
 *
 * The line shape is not inferred from samples alone. `hermes-agent/hermes_logging.py` holds
 *
 *     _LOG_FORMAT = "%(asctime)s %(levelname)s%(session_tag)s %(name)s: %(message)s"
 *
 * and attaches it to the rotating file handlers for `logs/agent.log` (INFO+),
 * `logs/errors.log` (WARNING+) and `logs/gateway.log` (INFO+, gateway-prefixed loggers).
 * `%(session_tag)s` is injected by a `logging.setLogRecordFactory()` shim and expands to
 * `" [<session id>]"` while a session is active and to the empty string otherwise. No `datefmt`
 * is passed for the file handlers, so `%(asctime)s` is Python's stdlib default —
 * `YYYY-MM-DD HH:MM:SS,mmm`. `%(levelname)s` is the stdlib level name set
 * (DEBUG/INFO/WARNING/ERROR/CRITICAL); this file does not invent spellings outside it.
 *
 * The log directory is `<HERMES_HOME>/logs`, and a fleet gives each agent its own
 * `HERMES_HOME` under a `profiles/` root (`hermes_constants.get_hermes_home()` /
 * `named_profile_home()`), so the agent identity is the profile directory in the file path —
 * which is why the first `parse` reads `path` and not the line body.
 *
 * ## What was measured and what was not
 *
 * - **Measured** (724 lines across five fleet profiles and the personal home, 2026-09-19):
 *   levels `INFO` and `WARNING`; the timestamp, logger and message layout; the four contention
 *   markers below; and that 102 of those 724 lines (14%) carry no log prefix at all (start-up
 *   banner art and the `gateway-*-diag.log` dumps). That last number is why the normalization
 *   chain ends at a `match` actor instead of assuming every tailed line parses.
 * - **Not measured**: a session-tagged line (no session ran while these logs were written), and
 *   an `ERROR`/`CRITICAL` line. Both shapes come from the format string and Python's stdlib,
 *   cited above, not from a sample.
 * - **Not present, therefore not invented**: `conflict`, `exception`, `Traceback`, `timeout`,
 *   `busy`, `in use`, `retry`, `throttle`, `degraded` occur zero times in the measured corpus.
 *   Git-conflict and worktree-contention spellings are absent from this surface entirely.
 */

/** Hermes' own `parse` patterns, in the engine's RE2 dialect (no lookaround, no backreferences). */
export const PATH_PATTERN = "(?P<agent>[^/]+)/logs/(?P<log>[^/]+)$";
export const LINE_PATTERN =
  "^(?P<at>[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2},[0-9]{3})"
  + " (?P<level>[A-Z]+)(?: \\[(?P<session>[^\\]]*)\\])?"
  + " (?P<logger>[^ :]+): (?P<message>.*)$";

/**
 * Python `logging` level names, grouped onto the `agent-session-monitor` severity axis.
 *
 * **The level field is the authority here, and message text is not.** `agent-session-monitor`
 * reads marker text because the OTLP severity it receives is untrustworthy (a Codex error arrives
 * there carrying `severityText: "INFO"`).
 * This source has no such gap: `%(levelname)s` is emitted by the logging module for every record,
 * and it was present on every one of the 622 parseable measured lines. Folding message text into
 * severity here would instead misclassify — `WS error … reconnecting` and
 * `payment / credit error` are both WARNING records whose text contains `error`.
 */
export const LEVELS = Object.freeze({
  error: Object.freeze(["ERROR", "CRITICAL"]),
  warning: Object.freeze(["WARNING"]),
  normal: Object.freeze(["DEBUG", "INFO"]),
});

/**
 * Contention markers, each one a measured substring of a measured message.
 *
 * | field | marker | measured occurrence |
 * |---|---|---|
 * | `lock` | `lock` | `kanban dispatcher: holding singleton dispatcher lock (…/.dispatcher.lock)` |
 * | `blocked` | `BLOCKED` | `Job '…': BLOCKED by pre-dispatch config validation — …` |
 * | `unhealthy` | `unhealthy` | `Auxiliary: marking openrouter unhealthy for 60s (payment / credit error).` |
 * | `reconnect` | `reconnecting` | `Mattermost WS error:  — reconnecting in 4s` |
 *
 * These are the four contention shapes the measured corpus actually holds. A marker that never
 * appeared is not added on the guess that it might — the same rule `agent-session-monitor` states
 * for its own spellings.
 */
export const CONTENTION_MARKERS = Object.freeze({
  lock: "lock",
  blocked: "BLOCKED",
  unhealthy: "unhealthy",
  reconnect: "reconnecting",
});

export const CONTENTION_FIELDS = Object.freeze(Object.keys(CONTENTION_MARKERS));

const celString = (value) => JSON.stringify(value);
const anyEqual = (expression, values) => (values.length === 1
  ? `${expression} == ${celString(values[0])}`
  : `${expression} in [${values.map(celString).join(", ")}]`);

const levelEvidence = (band) => `(${anyEqual("event.level", LEVELS[band])})`;
const markerEvidence = (field) => `event.message.contains(${celString(CONTENTION_MARKERS[field])})`;

/**
 * The one transform that turns two decoded field sets into the normalized envelope.
 *
 * Everything below this line in the graph reads `evidence` and `contention` booleans, `agent`,
 * and the three display fields. It never reads a Hermes spelling.
 */
export const NORMALIZATION_TRANSFORM = [
  "{'agent': event.agent, 'log': event.log, 'at': event.at, 'level': event.level,",
  " 'logger': event.logger, 'message': event.message,",
  " 'session': ('session' in event ? event.session : ''),",
  " 'recognized': true,",
  ` 'evidence': {'error': ${levelEvidence("error")}, 'warning': ${levelEvidence("warning")},`,
  ` 'normal': ${levelEvidence("normal")}},`,
  ` 'contention': {${CONTENTION_FIELDS.map((field) => `'${field}': ${markerEvidence(field)}`).join(", ")},`,
  ` 'any': (${CONTENTION_FIELDS.map(markerEvidence).join(" || ")})}}`,
].join("");

/** The predicate the conflict wire carries: one contention marker is enough. */
export const CONTENTION_PREDICATE = "event.contention.any";

/**
 * The listener declaration and the wire that normalizes what it tails.
 *
 * The source is one `listener` for the whole fleet, not one per agent: the glob spans every
 * profile and the agent identity comes back in `path`. That keeps adding an agent a matter of the
 * `agents` list this template routes on, with no second reader of the same directory tree.
 */
export function hermesSource({ root, glob, pollMs, capabilityRoots }) {
  return Object.freeze({
    actors: [
      {
        id: "hermes_logs",
        actorType: "listener",
        config: {
          source: { kind: "file_tail", value: { glob, poll: pollMs } },
          capabilities: { FsRead: { approval: "none", roots: capabilityRoots ?? [root] } },
        },
      },
      {
        id: "parse_path",
        actorType: "parse",
        config: { decoder: "regex", field: "path", arguments: { pattern: PATH_PATTERN } },
      },
      {
        id: "parse_line",
        actorType: "parse",
        config: { decoder: "regex", field: "body", arguments: { pattern: LINE_PATTERN } },
      },
      { id: "normalize", actorType: "map", config: { transform: NORMALIZATION_TRANSFORM } },
    ],
    edges: [
      { from: "hermes_logs", fromPort: "line", to: "parse_path", toPort: "event" },
      { from: "parse_path", fromPort: "event", to: "parse_line", toPort: "event" },
      { from: "parse_line", fromPort: "event", to: "normalize", toPort: "event" },
      { from: "normalize", fromPort: "event", to: "line_match", toPort: "event" },
    ],
  });
}
