# Agent Session Monitor template

A clonable graph that accepts scrubbed, vendor-neutral OTLP/HTTP JSON, normalizes the evidence,
and then classifies it as `normal`, `warning`, `error` or `unclassified`. The trap where a Codex
error arrives carrying `severityText: "INFO"` is pinned down by giving error evidence priority.

```text
agent-otel-logs
  -> input_logs -> [parse -> normalize_logs(map) -> classify_logs(map)]
  -> classify_logs_match(match)
       ok  -> by_class_logs(route) -> observed_logs(tap) -> agent-observed-logs
       err -> [map: classification=warning] -> invalid_logs(tap) -> agent-invalid-logs

agent-otel-metrics
  -> input_metrics -> parse_metrics -> normalize_metrics(map) -> classify_metrics(map)
  -> by_class_metrics(route) -> observed_metrics(tap) -> agent-observed-metrics
```

`route` emits the four structured `classification` values `normal|warning|error|unclassified` on
their own outlets, and an unexpected value still lands in the same observation tap by way of the
fixed `unmatched` outlet. The real actors are the existing `input`, `match`, `route` and `tap`;
`parse` and `map` are wire preprocessing. Logs pass through a single preprocessing chain, so one
JSON failure is not duplicated into several errors. `match.err` preserves the original `code` and
`detail`, attaches the existing `classification: "warning"` marker, and delivers it to the
`invalid_logs` tap and the `agent-invalid-logs` result mount. The failure position inside `detail`
is preserved as well. That door is a warning observation: it does not produce an alert state
transition or a notification effect. Shapes with a missing field go to `unclassified` behind an
explicit `in` guard. Domain failures such as a type mismatch on a field that is present, or broken
JSON, are left as an Err for `match` to receive.

## Two paths — installation and source build

The alpha path is the **installation** the one-line installer makes (`scripts/install.sh`,
QUICKSTART §1). That is the whole path. The source build is the contributor path. Every command below uses only two variables and one shell function. Run the
one block that matches your path first, and the rest can be copied as-is with no substitution.

```sh
# Installed with the one-line installer (scripts/install.sh)
PREFIX="$HOME/.local/opt/circular/current"
BIN="$PREFIX/bin"
circular() { "$PREFIX/bin/circular" "$@"; }
TPL="$PREFIX/src/sdk/typescript/templates/agent-session-monitor"
```

```sh
# Source build — contributor path (from the repository root; first run cargo build -p engine --bin circular-daemon)
BIN="$PWD/target/debug"
SDK="$PWD/sdk/typescript"
circular() { node "$SDK/circular.mjs" "$@"; }
TPL="$PWD/sdk/typescript/templates/agent-session-monitor"
```

`circular` is a shell function, not a variable: an unquoted variable holding two words is split
by bash but not by zsh, the default macOS shell, and a function behaves the same in both. Run
it as `circular edit …`.
An installation keeps this directory as the published repository has it: `graph.mjs`,
`source-adapter.mjs`, `preprocessing.mjs`, `deploy.mjs`, `live-smoke.mjs`, `edit-task.mjs`,
`samples/` and this README. **The offline tests are not published.**

Where an actor is cited by name below, that names the actor's reference page —
`reference/actors/<actor>.md` (in this checkout, `docs/public/reference/actors/`).

## Layout

| File | Role |
|---|---|
| `source-adapter.mjs` | **The only source/schema replacement boundary** — ingress, parse, the normalization map, and the offline oracle |
| `graph.mjs` | The classification, route and tap that read normalized evidence only, plus the public SDK declaration commands |
| `deploy.mjs` | Preflight before the daemon is contacted, plus owner-local public declaration/commit deployment |
| `live-smoke.mjs` | A live check that opens `actor.events` first, then injects a sample or waits for an accepted OTLP arrival |
| `edit-task.mjs` | The SDK editing task text supplied to the shared `circular edit` |
| `samples/` | Normal, warning, unclassified and metrics samples, plus the Codex INFO-error sample |

## The daemon config contract

The daemon state must be an owner-private absolute path. The ingress binding is written into a
mode-0600 `<state>/config.toml`, and the bearer value lives in a mode-0600 vault resource rather
than in that document. The operating values the document leaves out, such as
`process.deadline_secs`, take the daemon's defaults, and the daemon logs each default it uses.

```toml
[webhook]
bind = "127.0.0.1:32180"
bearer = { secret = "webhook.bearer" }
```

The `deploy.mjs` preflight checks that `<state>/daemon.sock` and `config.toml` exist before it ever
connects to the socket. The strict schema, the permissions and the vault references of the config
are judged once, by the daemon. The preflight also emits, as a warning, the fact that metrics are
currently conservatively `unclassified`.

## Classification rules

Every `logRecords[]` entry inside one OTLP logs batch is examined. The source adapter's
normalization map picks the first branch that matches, in the priority order below, and builds
one-hot evidence from it. So even when a single record carries both INFO and error evidence, the
error wins. The downstream generic classifier keeps the same error → warning → normal priority, so
the order does not change even if a different source adapter emits multiple pieces of evidence.

| Priority / result | Normalized evidence | Underlying grounds |
|---:|---|---|
| 1 / `error` | `evidence.error=true` | severity number ≥17, or ERROR/FATAL; `error` or `error.*`; `success=false`; HTTP ≥400; the first matching error/fail/exception/fatal marker in the event or body |
| 2 / `warning` | `evidence.warning=true` | severity 13–16, or WARN/WARNING; HTTP 300–399; the first matching warn/retry/throttle/degraded marker |
| 3 / `normal` | `evidence.normal=true` | severity 1–12, or TRACE/DEBUG/INFO/NOTICE; `success=true`; HTTP 200–299; the first matching request/completed/starts/success marker |
| 4 / `unclassified` | none of the grounds above | not guessed as normal; preserved as its own structured value |

The HTTP aliases are `http.response.status_code`, `http.status_code` and `status_code`. The
`error.message` seen in the measured payloads is caught by `error.*`. These compatibility spellings
and the raw OTLP path are used only in `source-adapter.mjs`. The downstream
`CLASSIFICATION_TRANSFORM` reads only the evidence booleans.

For metrics, the measured runs that failed posted no metrics at all, and the session metrics that
were observed carried no failure dimension. This version therefore preserves the metrics batch but
refuses to turn it into a normal result with no grounds: it sends it to `unclassified`.

## Offline tests

The offline tests, the reproduction scripts and the contract vectors are not published. They are
in neither the public repository nor an installation.

The tests check that every declaration survives the public codec, that edges use only published or
identity-derived ports, that the graph holds no commit of its own, and that the mount names are the
expected ones. `samples/codex-info-error.otlp.json` preserves the core observed shape — a
`severityText=INFO` together with an `error.message` — and pins the expected result to `error`. The
Envoy sample in the reproduction script is an authored sample built from two named fields; it is
not a captured user log. Against the real engine, the tests check one `unclassified` and zero
dead letters for it, and check that broken JSON and a `severityNumber` type mismatch each produce
one warning through the invalid door and zero dead letters.

## Daemon deployment and an end-to-end sample check

Run the lines from "Two paths" above first, then continue here. The state directory must be an
absolute path under your own home directory and must be mode 0700. Replace the token with a real
value.

```sh
STATE="$HOME/.circular/agent-session-monitor"
mkdir -p -m 700 "$STATE" && mkdir -p -m 700 "$STATE/secrets"
printf %s "owner-private-local-token" > "$STATE/secrets/webhook.bearer"
chmod 600 "$STATE/secrets/webhook.bearer"
cat > "$STATE/config.toml" <<'TOML'
# The daemon refuses to start if a secret vault exists without an explicit custody
# choice; it does not choose a backend by itself. `file_vault_v0` is the
# plaintext-file vault created above.
[secrets]
custody = { file_vault_v0 = {} }

[webhook]
bind = "127.0.0.1:32180"
bearer = { secret = "webhook.bearer" }
TOML
chmod 600 "$STATE/config.toml"

"$BIN/circular-daemon" --state "$STATE" &
# Wait for the "circular-daemon: serving" log line.

node "$TPL/deploy.mjs" --state "$STATE"
```

Now push a sample through to confirm the path end to end. The file `--sample` points at is in
this template's `samples/`, in both lanes. You can instead give the absolute path of your own
OTLP/HTTP JSON file, or omit `--sample` and wait for arrivals from a real producer (see the
watch-only section below).

```sh
SAMPLE="$TPL/samples/codex-info-error.otlp.json"
node "$TPL/live-smoke.mjs" \
  --state "$STATE" --signal logs --expect error \
  --bind 127.0.0.1:32180 \
  --bearer-file "$STATE/secrets/webhook.bearer" \
  --sample "$SAMPLE"
```

`live-smoke.mjs` opens the `actor.events` subscription before it POSTs, and it confirms the actual
structured payload of `observed_logs` has `classification=error` rather than settling for a plain
ingress 202. The normal, warning and unclassified samples can be checked the same way by changing
`--expect` and the sample file.

This ingress is the daemon's **generic webhook mount**, so the ceiling on the serialized request
body is **1 MiB (1,048,576 bytes)** and anything larger gets a 413. The dedicated receiver of the
`otlp` actor uses different values: 16 MiB, and 900 KiB for the fragments it splits into after
acceptance. If you point a collector straight at the generic webhook, you must keep the batch size
within 1 MiB — a recommended item count is **undetermined**, because serialized size differs per
signal (see the `otlp` reference page).

## Editing the running SDK code with a harness

`circular edit` reconstructs `current.ts` (scope `current/`) through the same public snapshot →
SDK `generateProgram` path that `circular chat` uses. The template supplies only the task text. Read the
session's first message and the harness instructions, then write the change in `proposal.ts`:
import what stands from `circular:current` and declare only what is new or different. `current.ts`
is read-only and is rewritten from the daemon after each approval. Options are given as arguments only.

```sh
circular edit --state "$STATE" \
  --harness codex --template agent-session-monitor --dry-run
circular edit --state "$STATE" \
  --harness codex --harness-bin /absolute/path/to/codex \
  --template agent-session-monitor --instruction 'change the logs source to an HTTP pull'
# After reviewing the program code it printed; <session> is the session directory that
# command printed, under "$STATE/chat/":
circular edit --state "$STATE" \
  --session "$STATE/chat/<session>" --approve
```

`--dry-run` only creates the files. A normal run has the harness store a code candidate, and only
with `--approve` does the installing host perform admission, ValidateEpoch and CommitEpoch. The
session's `deploy.mjs --approve --program proposal.ts` uses the same approval path. Omitting an
actor from the SDK code does not delete it; an explicit SDK remove is required.

At approval time the state before the commit is reconstructed as code and preserved in the session's
`approval-*/current.ts`. In the output, `previous` is that copy, `current` is the session's current code
rewritten from the daemon after the commit, and `program` is a copy of the code actually submitted. A copy from a failed approval is not accepted as a rollback target.
Rollback, too, reads the current anchor after an explicit approval and re-executes that code
through the installing host.

```sh
circular edit --state "$STATE" \
  --rollback "$STATE/chat/<session>/approval-<id>/current.ts" --approve
```

The preserved files are not the authoring body. The latest state is the log replay, and a rollback
is a new authoring execution. The existing host does not automatically delete actors that are
absent on re-execution, so restoring the full topology of a change that adds or removes actors is a
separate limitation. ReplayRewind observes a past run; it does not restore authoring state.

## Watch-only check against real traffic

You do not need to change any user settings, and there is no separate executable — there is no
`circular-catch` binary, and session catch is done by an authored `listener` actor (a `file_tail`
source plus `capabilities.FsRead`) reading directly within its own run. The boundary of this
template is OTLP acceptance, so here you point an OTLP producer (harness telemetry or a collector)
at the address the authored OTLP source of this state listens on, and then wait for the
observation.

```sh
node "$TPL/live-smoke.mjs" \
  --state "$STATE" --signal logs --expect normal \
  --contains '<measured-session-id>' --timeout-ms 30000 &
MONITOR_PID=$!

# Here, let the OTLP producer emit once to the listen address of the authored source.
wait "$MONITOR_PID"
```

`--contains` requires, from the same `actor.events` subscription, both that the post-scrub payload
of `parse_logs` carries the session or conversation ID and that `observed_logs` carries the
expected classification. Raw payloads, prompts and bearers are not printed. The address of the
ingress binding must point at `webhook.bind` from the config, and the token must be the same as the
vault resource the config names.

## Replacing the source with a Loki/Grafana pull

There is **one file to change: `source-adapter.mjs`**. Its `sourceAdapter()` currently owns both the
`input → [parse → normalize map]` topology and the ingress mount. To move to Loki/Grafana, replace
that topology with the existing `timer → request → map → parse` pull. `request.response.body` is
Bytes, so an intermediate map must emit `{'body': string(event.body)}` for the JSON parse to receive
a String. The final map must emit the same `{signal, recognized, evidence}` contract as today. The
classifier, route, tap and observation mounts in `graph.mjs` are not modified. That is, there is no
place where source query syntax or a vendor field alias can bleed downstream.

## Honest limits

- Case is not folded arbitrarily. The current markers and severity texts follow the spellings that
  were actually measured. New spellings are not added on speculation.
- The HTTP status on the live CEL path assumes samples in which it is a numeric `intValue`, as in
  the measured payloads. If a different OTLP JSON encoder writes a decimal string, the source
  adapter must be updated and new samples are needed.
- A valid OTLP envelope that carries no grounds for a decision, or that does not match the
  currently measured shape (a single resource and scope), shows up as `unclassified`. Shapes with
  no `evidence` at all, such as an Envoy access log, are guarded for field absence. A payload whose
  JSON itself is broken goes to `match.err → invalid_logs(tap)` and is observed at
  `agent-invalid-logs`. An error is not turned into a normal classification.
- Marker-based event/body judgement is not a semantic dictionary. It takes the highest severity in
  the batch, so a normal diagnostic record that merely describes an error can make the batch an
  error batch.
- The observation payload preserves only the classification evidence; it does not duplicate the
  whole original OTLP envelope. The ID join of `live-smoke.mjs --contains` is confirmed on the
  upstream `parse` frame of the same subscription. Looking up the original text requires a separate
  scrubbed storage path. Passing payloads through the engine's pre-ledger scrubber ahead of this
  template remains an operating contract.
