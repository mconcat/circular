# Hermes Dashboard template

A clonable graph that tails the logs of a running Hermes agent fleet, normalizes every line into
one envelope, and turns that into a per-agent stall signal, a per-agent activity count and a
contention feed. It is a specialization of the `agent-session-monitor` rail rather than a second
system: it introduces no new actor kind, no new view kind and no new authoring verb, and
everything it knows about Hermes lives in two regular expressions and two tables inside
`source-adapter.mjs`.

```text
hermes_logs(listener, one file_tail over the whole fleet)
  -> [parse(path) -> parse(body) -> normalize(map)]      (source-adapter.mjs owns all three)
  -> line_match(match)
       err -> [map] -> unparsed_lines(tap)               banner art and diagnostic dumps, observed
       ok  -> by_agent(route at ["agent"])
                route_<agent> -> [map 1.0] -> activity_<agent>(windowed_reduce)
                route_<agent>              -> quiet_<agent>(debounce)
                unmatched     -> unknown_agents(tap)     an agent nobody listed, observed
       ok  -> [filter(contention) -> map] -> conflicts(tap)

beat(timer every=emission_period) -> [map 0.0] -> activity_<agent>(sample)
activity_<agent>.aggregate -> [map] -> fleet_activity(tap)
quiet_<agent>.event        -> [map] -> stalls(tap)
```

The real actors are the existing `listener`, `timer`, `match`, `route`, `windowed_reduce`,
`debounce` and `tap`. `parse`, `map` and `filter` are **destination-inlet preprocessing on a
wire**, not actors.

Where an actor is cited by name below, that names the actor's reference page —
`reference/actors/<actor>.md` (in this checkout, `docs/public/reference/actors/`).

## Two lanes — installation and source build

An installation made by the one-line installer (`scripts/install.sh`, QUICKSTART §1) keeps the
published repository's source tree, and this template with it; `circular template list` shows it
beside `agent-session-monitor` and `incident-autopilot`. Run the one block that matches your lane
first.

```sh
# Installed with the one-line installer (scripts/install.sh)
PREFIX="$HOME/.local/opt/circular/current"
BIN="$PREFIX/bin"
TPL="$PREFIX/src/sdk/typescript/templates/hermes-dashboard"
```

```sh
# Source build — contributor lane (from the repository root, after
# cargo build -p engine --bin circular-daemon)
BIN="$PWD/target/debug"
TPL="$PWD/sdk/typescript/templates/hermes-dashboard"
```

## Layout

| File | Role |
|---|---|
| `source-adapter.mjs` | **The only source/format replacement boundary** — the listener declaration, the two patterns, the level table, the contention-marker table and the offline oracle |
| `graph.mjs` | The per-agent window, debounce, classification and surfaces that read the normalized envelope only, plus the public SDK declaration commands |
| `deploy.mjs` | Preflight before the daemon is contacted, plus owner-local public declaration/commit deployment |
| `samples/fleet-lines.json` | Scrubbed measured lines together with their **hand-written** expected envelopes |

As with the other templates, the offline tests are not published.
`samples/fleet-lines.json` travels with the repository, and so with an installation.

## Stage by stage — what each one uses

| Stage | What it uses | What it does here |
|---|---|---|
| 1. Source | `listener` (`source.kind = file_tail`, `capabilities.FsRead`) | One reader tails `profiles/*/logs/<file>` across the whole fleet |
| 2. Normalization | wire preprocessing `parse`(regex, `path`) → `parse`(regex, `body`) → `map` | Takes the agent out of the path and the timestamp, level, session, logger and message out of the line, and builds one normalized envelope |
| 2b. The failure door | `match` (`err` outlet) | An arrival that either pattern failed to match goes to `unparsed_lines` |
| 3. Distribution | `route` (`at: ["agent"]`, `cases`) | One lane per configured agent. Anything outside the list lands on `unmatched` |
| 4. Stall onset | `debounce` (`quiet_window`) | Emits the last line **once**, at the moment that agent has been quiet for `quiet_window` |
| 5. Activity | `windowed_reduce` (`window_length`, `emission_period`, `reduce`, `seed`) plus `timer` | Emits the number of lines in the window, every period. A zero means that agent is still stopped |
| 6. Contention | wire preprocessing `filter` → `map` | Selects the lines carrying a contention marker and labels them normal, warning or error |
| 7. Surfaces | five `tap`s plus `UpsertExportMount` and `SetPresentation(view)` | Draws with the already-registered view kinds `table` and `feed` only |

## Why both `windowed_reduce` and `debounce`

They answer different questions and neither substitutes for the other.

- `debounce(quiet_window)` is **stall onset**. At the moment a quiet window elapses it emits the
  last line once, and does not emit again until a new line arrives. At most one emission is ever
  pending, so a burst of lines produces no flapping — suppressing that flap is the element's own
  state, not something layered on top of it.
- `windowed_reduce` is **activity**. Stalled or not, every period it says how many lines arrived
  in the last window.

## Why the `beat` timer exists

`windowed_reduce` emits nothing for an empty window. Without a beat, a stalled agent would produce
no aggregate at all, and on the surface **a stopped fleet would look exactly like a quiet healthy
one**. The `beat` feeds every window one 0.0 sample so no window is empty, and each agent's
row therefore keeps arriving with `events: 0` while it is stalled. Health and death must not look
identical.

This beat is an element authored by this pipeline measuring an *external* agent's output. It is
not a supervisor heartbeat inferring whether a Circular actor is alive.

## Where the Hermes format comes from

- **Version**: Hermes Agent **0.20.6** (`hermes-agent/pyproject.toml` `version = "0.20.6"`, Nous
  Research `hermes-agent`). The installs this was read against were a stock tree and an OAuth
  build. Both were installed from a tarball and carry no git history, so no commit can pin them —
  **the version string is the only fixed point this template holds.**
- **The authority for the line shape**: `hermes-agent/hermes_logging.py`

  ```python
  _LOG_FORMAT = "%(asctime)s %(levelname)s%(session_tag)s %(name)s: %(message)s"
  ```

  The file handlers use that format: `logs/agent.log` (INFO and above), `logs/errors.log` (WARNING
  and above) and `logs/gateway.log` (INFO and above, for the `gateway`, `hermes_plugins` and
  `plugins.platforms` logger prefixes). `%(session_tag)s` is injected by a
  `logging.setLogRecordFactory()` shim and expands to `" [<session id>]"` while a session is alive
  and to the empty string otherwise. No `datefmt` is given to the file handlers, so `%(asctime)s`
  is Python's stdlib default, `YYYY-MM-DD HH:MM:SS,mmm`. The value space of `%(levelname)s` is the
  Python `logging` standard (DEBUG/INFO/WARNING/ERROR/CRITICAL), and this template invents no
  spelling outside it.
- **Agent identity**: the log directory is `<HERMES_HOME>/logs`, and a fleet gives each agent its
  own `HERMES_HOME` under a `profiles/` root (`hermes_constants.get_hermes_home()` and
  `named_profile_home()`). So **the agent name is the profile directory in the file path**, which
  is why the first `parse` reads `path` rather than the line body.

### What was measured and what was not

- **Measured** (724 lines across five fleet profiles and one personal home): the levels `INFO` and
  `WARNING`; the timestamp, logger and message layout; the four contention markers below; and the
  fact that **102 of those 724 lines (14%) carry no log prefix at all** — start-up banner art and
  `gateway-*-diag.log` dumps. That last number is why the normalization chain ends at a `match`
  actor: it does not assume every tailed line parses.
- **Not measured**: a session-tagged line (no session was running while these logs were written)
  and an `ERROR` or `CRITICAL` line. Both shapes come from the format string and the Python
  standard above rather than from a sample, and the fixtures mark them `origin: "format-source"`.
- **Absent, therefore not invented**: `conflict`, `exception`, `Traceback`, `timeout`, `busy`,
  `in use`, `retry`, `throttle` and `degraded` occur zero times in the measured corpus.
  Git-conflict and worktree-contention spellings are absent from this surface entirely. New
  spellings are not added on speculation.

## Classification — a severity axis and a contention axis

**Severity is decided by the `level` field and not by message text.** `agent-session-monitor`
reads marker text because the OTLP severity it receives cannot be trusted — a Codex error arrives
there carrying `severityText: "INFO"`. This source has no such gap: `%(levelname)s` is stamped on
every record and was present on all 622 parseable measured lines. Folding text into severity here
would instead be wrong: `Mattermost WS error: — reconnecting in 4s` and `payment / credit error`
are both WARNING records whose text contains `error`.

| Result | Grounds |
|---|---|
| `error` | `level` in {ERROR, CRITICAL} |
| `warning` | `level` = WARNING |
| `normal` | `level` in {DEBUG, INFO} |
| `unclassified` | any other level |

The priority (error → warning → normal → unclassified) and the evidence booleans a downstream
reader sees are **exactly `agent-session-monitor`'s**. This template specializes that rail; it does
not introduce a second severity vocabulary.

Contention is a separate axis, and every marker is a measured substring of a measured message.

| Field | Marker | Measured occurrence |
|---|---|---|
| `lock` | `lock` | `kanban dispatcher: holding singleton dispatcher lock (…/.dispatcher.lock)` |
| `blocked` | `BLOCKED` | `Job '…': BLOCKED by pre-dispatch config validation — …` |
| `unhealthy` | `unhealthy` | `Auxiliary: marking openrouter unhealthy for 60s (payment / credit error).` |
| `reconnect` | `reconnecting` | `Mattermost WS error:  — reconnecting in 4s` |

Only the lines whose `contention.any` is true reach the `conflicts` surface.

## Deployment

The daemon state must be an owner-private absolute path. This template uses no push ingress and
binds no capability in `config.toml`, so the state needs no `config.toml`: the daemon starts on
its default operating values and logs each one it uses as a `config default used` line.

```sh
STATE="$HOME/.circular/hermes-dashboard"
mkdir -p -m 700 "$STATE"

"$BIN/circular-daemon" --state "$STATE" &
# Wait for the "circular-daemon: serving" log line.

node "$TPL/deploy.mjs" --state "$STATE" \
  --root "$HOME/.hermes-gecko/profiles" \
  --agents beaver,coyote,gecko,meerkat,quokka
```

An install that runs a single agent out of a personal `~/.hermes` has no profile layer, so the name
the path pattern yields is the home directory's own name:

```sh
node "$TPL/deploy.mjs" --state "$STATE" --root "$HOME" --glob "$HOME/.hermes/logs/gateway.log" \
  --agents hermes   # the path yields the name `.hermes` here, so the route sends it to
                    # unmatched — see the limits below
```

Options come only from arguments. Product behaviour is not set through environment variables.

| Argument | Default | Meaning |
|---|---|---|
| `--root` | (required) | The profiles directory holding one subdirectory per agent. It is also the FsRead capability root |
| `--agents` | (required) | The agents to build lanes for. These become `route` case names, so each must be a port-safe name |
| `--log-file` | `gateway.log` | The file name the derived glob tails |
| `--glob` | derived | Given explicitly, it replaces `--log-file` |
| `--poll-ms` | 2000 | The `file_tail` poll interval |
| `--window-ms` | 300000 | The activity window length (half-open, `[f − w, f)`) |
| `--emission-period-ms` | 60000 | The aggregate emission period, and the beat period |
| `--stall-quiet-ms` | 300000 | A stall fires after this much quiet |

Every number is an **example operating value** for a five-agent laptop fleet, not a product
default.

## Offline tests

The offline tests are not published.

The expected values are **written by hand** beside the fixtures; they are not produced by running
this template's own normalizer. The tests hold: the normalized envelope of each measured line; that
a measured line which does not parse goes through the `match.err` door; that each of the four
contention markers has a measured witness of its own; that every agent gets one window and one
debounce, and that the window's sample inlet carries **both** the line wire and the beat wire; that
every declaration survives the public codec and uses published ports only; and that no retired
actor kind stands in the graph.

## A measured live run

Five profiles were created under one temporary state, the measured lines were appended to their
`gateway.log`, and the template was deployed with
`--poll-ms 500 --window-ms 20000 --emission-period-ms 3000 --stall-quiet-ms 6000`.

- Deployment: 19 actors and 62 commands were accepted and the run stood up. Every CEL snippet (the
  normalization transform, the contention predicate, the `1.0` and `0.0` sample maps, the window
  row map and the stall row map) and all five `route` cases passed activation against the real
  engine.
- **25 seconds with nothing appended**: 40 `fleet_activity` arrivals — five agents times eight
  periods — every body `0`. The surface keeps refreshing while the whole fleet is quiet. Without
  the beat those 40 arrivals are 0.
- With 12 measured lines injected, 3 of them unparseable: `line_match` 12 arrivals, `by_agent` 9,
  `unparsed_lines` 3, `stalls` 5 (one per agent).

## Honest limits

- **The agent list is configuration.** The `route` cases come from config, so a new agent needs a
  new entry before it gets a lane. Until then its lines are **not dropped**: they collect on the
  `unknown_agents` surface. A dynamic lane per key is a `replicator` cell per key; this template
  does not use one, because its lanes are per-agent configuration rather than minted cells. That
  is a known gap, not something this template works around.
- **The name the path yields is the agent name.** It is read out of `profiles/<name>/logs/<file>`,
  so a personal single-agent install (`~/.hermes/logs/…`) yields the name `.hermes`. A `route` case
  name must be port-safe, so that name cannot be a case and the line goes to `unmatched`.
- **Thresholds are config, not a form.** Changing the window, the period or the quiet window is an
  **edit** of an actor's config, and that edit is both an SDK verb and a canvas gesture. Adding a
  `form` that sets the same values by injection would be standing up a second configuration system
  for something the canvas can already draw.
- **Secrets are assumed to have been removed on the Hermes side.** The log formatter is a
  redacting formatter, and the measured lines carry `Secret redaction: ENABLED (tool output, logs,
  and chat responses are scrubbed before delivery)`. No second scrubber is placed in front of this
  template: a second rule in front of an already-scrubbed source leaves no place to ask which one
  is authoritative. The samples in this repository had only absolute paths substituted on top of
  that.
- **A line count is a proxy for output.** One line is not one turn, and loggers differ in how
  talkative they are. What `events: 0` states for certain is "this agent wrote no log line in this
  window", not "it did nothing".
- **The observation feed shows recorded arrivals from *before* preprocessing.** `actor.events` and
  `display.frames` count the **original arrival recorded at the destination inlet**. Preprocessing
  then runs over that recorded original, which is why `conflicts` shows 9 arrivals in the run
  above: the filter did fire, but arrivals are recorded first. The post-preprocessing value is in
  the tap's emission, that is, the export mount's `result`, and neither of those two subscription
  targets carries it — **the selectivity of the contention filter and the output of the display map
  are not measured live.**
- **A multi-line record is one arrival per line.** Continuation lines such as a stack trace carry
  no prefix and go to `unparsed_lines`. That is 14% of the measured corpus, and the contract here
  is that they are not silently discarded. Joining them back together is not something this
  template does.

## Replacing the source

There is **one file to change: `source-adapter.mjs`**. It owns the `listener` declaration, the two
patterns, the level table and the contention-marker table together, and it emits exactly one
contract downstream:

```text
{agent, log, at, level, logger, message, session, recognized, evidence{}, contention{}}
```

`graph.mjs` reads no Hermes spelling, no log format and no file path. Moving to a different harness,
or to push ingress, or to a `timer` → `request` poll, means replacing that file's `hermesSource()`
topology and its two patterns. The classification, the window, the debounce and the surfaces
beneath them are unchanged.
