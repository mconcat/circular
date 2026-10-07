# Incident Autopilot template

A vertical path that performs real external work, moved into a form you can copy and run.

```text
webhook mount ─▶ parse/map ─▶ match.ok ─▶ route ─▶ remediate ──┬─(exit != 0)─▶ Slack escalation
                                      └─(exit == 0)─▶ verify ──┬─(exit == 0)─▶ resolved observation ─▶ Slack resolution notice
                                                               └─(exit != 0)─▶ Slack escalation
```

A successful result becomes a notification too. The `resolved` tap is the terminal
observation, and one map reading its output builds the *"Incident resolved"* notification.

The route's unmatched inputs and bad tool calls go to the alert at `invalid-incident-door`.
Input that fails the inlet preprocessing itself — malformed JSON, for instance — goes to the
same door, because this template wires `match.err` into it (see
[Disposition of invalid input](#disposition-of-invalid-input)). This door lets you observe
arrivals without binding an external notification channel.

This graph's runtime acceptance scenario and its deployment evidence are held by integration
tests and probes that are not published. The public repository, and so an installation,
carries no tests and no contract vectors.

## Two lanes — installation and source build

The alpha path is the **installation** the one-line installer makes (`scripts/install.sh`,
QUICKSTART §1). The source build is the contributor path. Every command below
uses only two variables and one shell function. Run the one block that matches your lane first,
and the rest can be copied as-is with no substitution.

```sh
# Installed with the one-line installer (scripts/install.sh)
PREFIX="$HOME/.local/opt/circular/current"
BIN="$PREFIX/bin"
circular() { "$PREFIX/bin/circular" "$@"; }
TPL="$PREFIX/src/sdk/typescript/templates/incident-autopilot"
```

```sh
# Source build — contributor lane (from the repository root; first run
# cargo build -p engine --bin circular-daemon)
BIN="$PWD/target/debug"
SDK="$PWD/sdk/typescript"
circular() { node "$SDK/circular.mjs" "$@"; }
TPL="$PWD/sdk/typescript/templates/incident-autopilot"
```

`circular` is a shell function, not a variable: an unquoted variable holding two words is split
by bash but not by zsh, the default macOS shell, and a function behaves the same in both. Run
it as `circular edit …`.
An installation keeps this directory as the published repository has it: `graph.mjs`,
`deploy.mjs`, `edit-task.mjs`, `bin/remediate`, `bin/verify`, `sample-incident.json` and this
README. The offline tests and goldens are not published.

Where this document cites an actor by name below, it means that actor's reference page —
`reference/actors/<actor>.md` (in this checkout, `docs/public/reference/actors/`).

## Contents

| File | Role |
|---|---|
| `graph.mjs` | The graph declaration — actors, edges and mounts in one piece (the offline test and `deploy.mjs` read the same thing) |
| `deploy.mjs` | Preflight diagnostics plus deployment to a running daemon |
| `sample-incident.json` | A Grafana-shaped sample injection payload (the route picks the remediate call from it) |
| `bin/remediate`, `bin/verify` | Sample programs meant to be replaced (a dry run works with them as they are) |

## The daemon config contract

A mode-0600 `<state>/config.toml` binds the process allowlist, the sink for the logical
notification channel, and the webhook bind address and bearer resource name all at once. URLs
and bearer values are not written into the config; they live in mode-0600 vault resources
under `<state>/secrets/`.

## Tool paths and environment

In `tool_executor`'s `tools`, the `program` of a tool with `effect: "spawn"` must be an
executable absolute path, and it must appear in the daemon's `[process] allowlist`.
The environment of a spawned child is scrubbed. It does not inherit HOME or PATH, so write the
interpreter of your tool script and any program it calls internally as absolute paths too, and
do not depend on `$HOME` or on shell startup files. Check a tool you wrote yourself before you
deploy it by giving it the intended input under `env -i /absolute/path/to/tool`. This rule
applies to process tools, not to the editing CLI harness.

## Local dry run

Run the `BIN`, `circular` and `TPL` lines from "Two lanes" above first, then continue.

```sh
STATE="$HOME/.circular-incident-demo"
mkdir -p "$STATE"
chmod 700 "$STATE"
mkdir -m 700 "$STATE/secrets"
printf %s "local-secret" > "$STATE/secrets/webhook.bearer"
chmod 600 "$STATE/secrets/webhook.bearer"
cat > "$STATE/config.toml" <<TOML
# Only the capability bindings are written here. deadline_secs, http_timeout_secs and the
# runtime_arrivals keys take the daemon's defaults, and the daemon logs each default it uses;
# write one of them only to change it.
[process]
max_concurrent = 2                                    # explicit operating limit; no default
allowlist = ["$TPL/bin/remediate", "$TPL/bin/verify"]

# The daemon refuses to start if a secret vault exists without an explicit custody choice;
# it does not choose a backend by itself. file_vault_v0 is the plaintext vault created above.
[secrets]
custody = { file_vault_v0 = {} }

[webhook]
bind = "127.0.0.1:32180"
bearer = { secret = "webhook.bearer" }
TOML
chmod 600 "$STATE/config.toml"
# Start without Slack and the notifications settle as EndpointGone — in a dry run that is the
# honest ending. The success path emits a notification too, so with the sample programs left as
# they are, the one notification that settles in a dry run is the resolution notice.

"$BIN/circular-daemon" --state "$STATE" &   # wait for the "circular-daemon: serving" log line
node "$TPL/deploy.mjs" --state "$STATE" \
  --remediator "$TPL/bin/remediate" --verifier "$TPL/bin/verify"

curl -sS -X POST "http://127.0.0.1:32180/v1/ingress/incidents" \
  -H "Authorization: Bearer local-secret" \
  -H "Idempotency-Key: incident-1" \
  --data @"$TPL/sample-incident.json"
# → 202. incident.txt and fix.txt appear in the daemon workspace ($STATE/workspace) and one
#   arrival is recorded at resolved-door (confirm with the arrival diagnostics on daemon stderr).
```

The preflight explains programs that cannot be executed and a missing daemon socket or config
document. The document's schema, its permissions, its vault references and the capability
assembly are judged once, by the daemon.

## Real smoke without credentials — the local notification center

To see the escalation vertical for real without Slack credentials, add the following binding to
`config.toml`. It passes if a *"Incident verification failed"* notification appears in the macOS
notification center after a POST that makes verification fail. The graph does not differ by a
single line — the channel is logical and resolving the sink belongs to the daemon.

The approved config accepts only an absolute path for `system_program`. That executable takes
the title as its first argument and the body as its second, and its standard input is empty —
no JSON body is handed to it on stdin (see the `notify` reference page).

This release ships no notifier program. Set `NOTIFIER` to the absolute path of an executable
of your own that takes a title and a body as its two arguments and posts the notification.

```sh
# Common to both lanes — NOTIFIER is an absolute-path executable taking a title and a body.
cat >> "$STATE/config.toml" <<TOML

[[notify.channel]]
name = "slack"
sink = { system_program = "$NOTIFIER" }
TOML
```

## Real-credential smoke run

The same as the dry run, except:

1. Create an Incoming Webhook in Slack, write its URL to `$STATE/secrets/slack.webhook_url`
   with mode 0600, and add the following binding to `config.toml`.

   ```toml
   [[notify.channel]]
   name = "slack"
   sink = { slack_webhook = { secret = "slack.webhook_url" } }
   ```

2. Replace `bin/verify` with a copy that always fails (`exit 5`), or inject after removing the
   `fix.txt` write from `bin/remediate`.
3. After the same POST, the smoke passes when *"Incident verification failed — verifier exited
   5"* arrives in the Slack channel. Make the remediation itself fail (`exit 7`) and *"Incident
   remediation failed — remediator exited 7"* arrives instead. Leave the sample programs alone
   (both succeed) and the same channel receives *"Incident resolved — verifier confirmed the
   remediation: fix marker present"*. The title and the body are separate fields; they are
   joined here only for readability.

The URL stays in the daemon vault and is not carried in a config, an effect, a receipt or a
diagnostic.

## Honest limits

- The notification body is not constant text. The two failures carry the measured
  `value.exit`, and the resolution notification carries the verifier's `value.stdout`. An
  effect result's `stdout` and `stderr` are Bytes on the wire, so the existing CEL `string(...)`
  conversion is what produces the String body. That conversion is not total over bytes that are
  not UTF-8 — if the verifier emits stdout that is not UTF-8, that notification is recorded as a
  domain failure (Err) rather than as a silent true or false, and it is not delivered. `exit` is
  an Int and has no such limit.
- `stderr` is not carried in any body. Putting it in a failure notification would let the
  same UTF-8 limit cost you the escalation itself, so only `exit`, an Int, is carried.
- This template does not include agent triage.
- Two further limits: the allowlist is not a sandbox, and a `remediate` or `verify` run that
  fails is not retried. A run is ended at `process.deadline_secs`, and a notification that
  fails transiently is retried on the `effects.retry_delays` schedule (QUICKSTART §2).

## Grafana contact point: the default webhook payload

In a Grafana contact point, choose Webhook and set the URL to the daemon's
`http://<webhook-host>:<webhook-port>/v1/ingress/incidents`. Set the Authorization scheme to
`Bearer` and the credentials to the same value as the daemon's webhook bearer. Name this contact
point as the receiver of your alert rule. **Do not set a custom payload or a payload template.**
Send Grafana's default JSON unchanged. These setting names were measured on Grafana 13.1.0.

The template reads the webhook envelope's `body` with the existing JSON parse, and a map on the
same wire turns the first alert's `fingerprint` into `id` and sets `tool` to `remediate`.
`arguments` is the first alert's `labels.fault_flag`, the top-level `status`, and the first
alert's `labels.alertname`, joined by single spaces. The demo rule must carry a `fault_flag`
label (for example `paymentFailure`). The remaining alert fields, `annotations` included, are
not put into the call arguments. When several alerts arrive batched together, one call is made
from the first alert. This scope is the same as the first fingerprint call that was measured
working.

A 202 means the webhook was accepted. Execution success is confirmed by an arrival at
`resolved-door`. `escalation-door` observes the notify inlet, so every notification this graph
emits passes through it — the two remediation and verification failures and the one resolution.
The route's unmatched inputs and tool call errors are confirmed as arrivals on the
alert's input at `invalid-incident-door`. That alert can observe an input arrival immediately,
and the firing delay of its true predicate is 1 millisecond. All three lanes emit an object with
the same `id`, `tool` and `arguments` String fields. Apply the existing CEL `string(...)`
conversion to the Grafana fields: if one branch alone emits a different field type, the whole
merges to Any and the tool input connection is refused. A non-Grafana object with no `alerts`
becomes `{id: "invalid-incident", tool: "unmatched", arguments: ""}` and goes to
`invalid-incident-door`. An existing direct `{id, tool, arguments}` call also belongs to that
lane. `sample-incident.json` is provided in the Grafana shape, and the raw input is visible in
the flight recorder.

### Disposition of invalid input

Four classes of input split across the three wires into the door, and **which lane it was is
distinguishable at the door.**

| Input | Where it splits | Body recorded at the door | Call the alert sees |
|---|---|---|---|
| Non-Grafana JSON that decodes | the else branch of the ingress map → `route.unmatched` | `{id, tool: "unmatched", arguments: ""}` | the same object |
| Malformed JSON | failure of the wire preprocessing `parse` (position 0) → `match.err` | `{code: "processing", detail: [3, {failure_point: {edge, step}}]}` | `{id, tool: "preprocess", arguments: "processing"}` |
| JSON whose `alerts` cannot be read | failure of the wire preprocessing `map` (position 1) → `match.err` | the same shape, with `step.kind` equal to `map` | the same |
| Tool call error | `remediate._error` | the failure reason string | `{id, tool: "remediate", arguments: <reason>}` |

**Malformed JSON is not folded into an unmatched call.** A combinator is preprocessing on the
destination inlet, and its failure stays an envelope-level `Err` that skips the remaining steps
and reaches `input_match.event` — `match` splits that tag and emits the reason on `err` (see the
`match` reference page; an envelope-level `Err` is governed by the wire's error port rules).
Wiring that `err` into the door is **this template's own sequence of verbs**, and there is no
place where the engine turns a preprocessing failure into a tool call. A rejection must not look
like an ordinary call, which is why the `tool` values of the two lanes differ.

**`processing` identifies a processing rejection.** The code applies regardless of whether
an error outlet is wired. Its `detail` retains the processing cause and optional preprocessing
failure location. This template wires the failure lane as `match.err → invalid_incident`.

**The body recorded at the door is that door's pre-preprocessing original.** An arrival record
holds the value before preprocessing, and only the `Ok` that passed the chain goes on to the
main logic. That is why the third and fourth columns of the table above differ — `arrival.scan`
answers the third column, and the alert sees the fourth.

**A door's arrival sequence holds more than deliveries.** The mount hangs on the
`invalid_incident` actor and `arrival.scan` answers that actor's arrivals, so besides the
delivery rows (kind 1) the same sequence holds one result-summary row (kind 3) for the
`Schedule` the alert arms for its own firing delay, and one `_timer` row (kind 2) for the wake-up
of that schedule. A true predicate arms once and does not arm again (see the `alert` reference
page). This sequence is cross-checked by fixtures, offline witnesses and a live daemon probe. None of
the three are published.

This ingress is the daemon's **general webhook mount**, so the ceiling on a serialized request
body is **1 MiB (1,048,576 bytes)** and anything larger gets a 413. That is a different value
from the 16 MiB of the dedicated `otlp` receiver (see the `otlp` reference page).

Offline verification and the arrival-verification slot on an isolated daemon are not
published. The published artifacts contain neither tests nor contract vectors.
Bind the absolute paths of this directory's `bin/remediate` and `bin/verify` into the daemon's
process allowlist.
A complete raw payload capture is not on hand, so the Grafana values in the current tests are
independently synthesized literals of the documented fields. Comparison against a measured raw
payload is not done.

A spawn result's `value.exit` is an Int. The four success and failure predicates compare it
against the CEL Int literal `0`. `0u` is a UInt, and comparing it against an Int exit yields Err.
On the SDK wire an Int is a bigint, and the JavaScript spelling `0n` is not evidence of a UInt.

The current public port shape of `tool_executor` is an open object, and the `value` shape of an
effect result is Any. Generated types therefore cannot narrow `value.exit` to Int. Given an
explicit Int shape the generator prints `bigint`. Without a shape at the source, not every tool
result is treated as a process result. This finding and the Int/UInt distinction are recorded in
`exit-types.golden`, which is not published.

## Approval for effect execution

Approval sits on the declaration that produces the effect. `request`, `file`, `notify` and
`listener` take it on each grant (`capabilities.<grant>.approval`), `agent` on the `approval` of
its actor config, and process execution on `tools.<name>.approval` inside `tool_executor`. The
only values are `"none"` and `"required"`. A grant's approval must be written; the agent's and a
tool's may be omitted, which means `"none"`, and the SDK does not append a field you omitted.
A tool's approval is declared per tool and inherits neither the executor root's value nor
another tool's.

To require approval for the remediate execution, put it in that tool's declaration in the
authoring code like this. Replace the program path with the real path in your deployment
environment.

```ts
import { toolExecutor } from "@circular/core";

export let remediate = toolExecutor({
  tools: {
    remediate: {
      effect: "spawn",
      program: "/path/to/remediate",
      arguments: [],
      approval: "required",
    },
  },
  capabilities: { ProcessSpawn: { approval: "none" } },
});
```

An actor that holds grants has no top-level `approval`; a declaration that writes one is
refused. The SDK's validation and reconstruction preserve omission, explicit `none` and
`required` exactly as written.
