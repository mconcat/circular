# Circular — Quickstart (macOS)

Circular is two processes: `circular-daemon` (owner-local engine, holds one *state
directory*) and `Circular.app` (the desktop app; it attaches to the daemon, and its
File → Start Daemon starts one through the `circular` command). The daemon is its own
background process: quitting the app leaves it running, and reopening the app attaches again.
Pipelines are authored in the UI or deployed from the TypeScript SDK; what a pipeline does is
recorded, so you can observe it live and replay it. The daemon and the app run on your machine —
there is no account to create and no API key of ours to paste anywhere.

## Two lanes, one document

**Lane A — the one-line installer — is the alpha path.** Lane B builds the same binaries from a
checkout and is the contributor path; there is no `cargo install` path. Step 1 differs by
lane. It ends by setting two shell variables and one shell function that every
later step uses unchanged:

| | **Lane A — one-line installer (alpha)** | **Lane B — build from source (contributors)** |
|---|---|---|
| you have | Circular installed by `install.sh`, with `$PREFIX` = `~/.local/opt/circular/current` | this checkout and a Rust toolchain |
| `$BIN` | `$PREFIX/bin` | `$PWD/target/debug` |
| `circular` (a shell function) | runs `$PREFIX/bin/circular` | runs `node $PWD/sdk/typescript/circular.mjs` |
| `$TPL` | `$PREFIX/src/sdk/typescript/templates/<name>` | `$PWD/sdk/typescript/templates/<name>` |

`circular` is a shell function rather than a variable because lane B's command is two
words. An unquoted variable holding two words is split by bash but not by zsh, the default
macOS shell; a function behaves the same in both, so every later step just runs
`circular chat --help`. Define it again in each new terminal.
In steps 2 through 9, a command that differs by lane is marked with its lane.

**Where the documents are.** An installation keeps the release's source tree at
`$PREFIX/src`, so a lane A reader has this file and everything it points at without a
checkout:

| In an installation | What it is |
|---|---|
| `$PREFIX/src/QUICKSTART.md`, `$PREFIX/src/README.md`, `$PREFIX/src/reference/` | this file and the rest of the public documents, from the source the installation was built from |
| `$PREFIX/share/circular/docs/pages/**.md`, listed by `$PREFIX/share/circular/docs/index.json` | the specification, from the daemon asset built from the same commit |
| `$PREFIX/src/sdk/typescript/INSTALL.md` | what the `circular` CLI component is and is not |
| `$TPL/README.md`, one per shipped template | a runnable end-to-end walkthrough, written for both lanes |

§8 names the actor behind each value rather than a file path. One page per actor lives in
`reference/actors/`, and the specification pages under `share/circular/docs/` carry the
full semantics for an installed reader.

## Prerequisites

Both lanes:

- macOS on Apple Silicon (the daemon's location rules and logs are macOS-specific).
- Node.js 22.12.0 or newer, with npm, on `PATH` (`ui/app/package-lock.json`). The installer
  builds the CLI and the app with it. Lane B and SDK programs you run yourself use it. The
  installed `circular` command needs none: it runs on the Node runtime inside `Circular.app`
  beside it, so it works the same from a terminal and from the app opened in Finder or the
  Dock. The installer does not install Node.
- The Xcode command-line tools (`xcode-select --install`): the installer fetches the source
  with `git`.
- An agent CLI you are logged into, on `PATH`: `claude` (`claude auth login`),
  `codex` (`codex login`) or `pi`. `circular chat` takes one of `--claude`, `--codex` or
  `--pi`, and saves the one you choose as this state's default; omit the flag afterwards.

Lane B also needs the following. A lane A install needs Rust too when the release has no
prebuilt daemon your Mac can use; the installer then builds the daemon, and says so.

- Rust stable, 1.95 or newer (`rustup update stable`); the workspace is edition 2024.
- The SDK's workspace dependencies, once: `(cd sdk/typescript && npm install)`.
- To build the daemon asset a release publishes: Python 3.9 or newer for
  `scripts/build-artifact.py` (the macOS `python3` is enough).

## 1. Get the binaries

### Lane A — install with one line

```sh
curl -fsSL https://raw.githubusercontent.com/mconcat/circular/v0.1.0-alpha.1/scripts/install.sh | sh
PREFIX="$HOME/.local/opt/circular/current"
BIN="$PREFIX/bin"
circular() { "$PREFIX/bin/circular" "$@"; }
```

The command installs release `v0.1.0-alpha.1` and nothing else. The installer downloads that
release's prebuilt daemon and that release's source, and stops if the daemon was not built
from exactly that source's commit. It keeps the source in the installation, installs the
`circular` CLI's dependencies there and builds `Circular.app` from it, with your Node.js. It
prints one line per step. A failure prints `install: <REASON>: …` with what to do. The
installer checks a new build in a staging directory before moving it into place; selecting
it and creating the command and app links are later steps that can also fail.
When the release's daemon cannot be used on your Mac, the installer prints
the reason the same way and builds the daemon as well, which needs Rust.

It installs into `~/.local/opt/circular/<version>-<commit>/` and selects that installation as
`~/.local/opt/circular/current` — the `$PREFIX` above. It links the command to
`~/.local/bin/circular` and the app to `~/Applications/Circular.app`, so Finder and `open`
find it in your Applications folder. The line of a newer release installs it beside the old
one and selects it. The installer opens no state directory and starts no daemon.
`scripts/INSTALL.md` has every step, path and exit code.

The installation has two commands in `$BIN`, `circular-daemon` and the `circular` CLI
launcher, plus the desktop app directory `Circular.app`.

### Lane B — build from source

```sh
cargo build -p engine --bin circular-daemon
BIN="$PWD/target/debug"
SDK="$PWD/sdk/typescript"
circular() { node "$SDK/circular.mjs" "$@"; }
```

A checkout builds no `Circular.app` of its own; the installer builds one (below). From a
checkout, start the daemon by hand (§3, last block), then run the desktop app from its own
directory:

```sh
(cd ui/app && npm install && ./node_modules/.bin/install-electron && npm start)
```

It attaches to the daemon already holding the state you choose.

In lane B, `circular daemon start --state "$STATE"` looks for `circular-daemon` beside the
CLI command and then on `PATH`. If no daemon answers and neither location has the executable,
it reports `install.program.circular-daemon` with the installation and source-checkout
remedies. Put `target/debug` on `PATH`, or use the manual start in §3.

To install from this checkout the way lane A does — the daemon built with Rust, the CLI and
`Circular.app` built with npm, placed where lane A places them:

```sh
scripts/install.sh --source .
```

It installs the committed `HEAD`; uncommitted changes are not included. The daemon asset a
release publishes is built with `python3 scripts/build-artifact.py`, which writes
`dist/circular-daemon-aarch64-apple-darwin.tar.gz` and its `.sha256`;
`scripts/install.sh --source . --daemon <that file>` installs with it instead of building the
daemon again.

## 2. Pick a state directory

The daemon keeps its socket, config, secrets, journals and recorded history there. Rules
(checked by both binaries): an **absolute** path **under your home**, **owned by you**,
mode **0700** (no group/other bits); `<state>/config.toml`, if present, must have no
group/other bits either (**0600**); keep it short so `<state>/daemon.sock` fits in 103 bytes.

```sh
STATE="$HOME/.circular/state"
```

Create it by hand: `mkdir -p -m 700 "$STATE"`. Nothing changes an existing one — a wrong
mode is reported with the fix (`chmod 700 "$STATE"`).

`<state>/config.toml` is read when the daemon boots. When a start finds a new state — one
with no journal (`journal.sqlite3`) yet — and no `config.toml` in it, the daemon first writes
a `config.toml` that holds only the `[http]` table shown further down, and logs
`circular-daemon: wrote the initial config.toml: http.hosts = ["localhost:*", "127.0.0.1:*", "[::1]:*"]`.
It skips this initial document when the state already has a journal; there, a missing
`config.toml` reads as an empty one. For the keys below, the daemon starts on the defaults shown. A
document that sets one of these keys replaces that key's default only; every key it leaves
out keeps its default. The daemon names each
default in force in its health answer (`daemon.health` `anchor.config_defaults`, one
`{key, value}` per default; `circular doctor` shows it as the `daemon.config_defaults` row)
and logs each one as a line, for example
`circular-daemon: config default used: process.deadline_secs=1800`.

| Key | Default |
|---|---|
| `process.deadline_secs` | `1800` |
| `notify.http_timeout_secs` | `20` |
| `runtime_arrivals.arrivals_max_mib` | `256` |
| `runtime_arrivals.arrivals_max_records` | `500000` |
| `runtime_arrivals.total_max_mib` | `2048` |
| `effects.retry_delays` | `[1000, 1000, 1000, 5000, 5000, 5000, 15000, 15000, 15000]` |

The three `runtime_arrivals` values are alarms, not quotas: crossing one refuses no input,
and `daemon.health` reports it under `anchor.journal` with the journal's size now.

`effects.retry_delays` is the waits, in whole milliseconds, between retries of an external call that
failed transiently from a `request` or a `notify` — the call never reached the other side (a
host name that did not resolve, a connection refused or timed out before it was established,
a notifier program that could not start) or the other side answered `429` or `503`. A call
that may already have arrived is not retried: any other `5xx`, a connection that dropped or
timed out after the request was sent, a notifier program that ran and did not exit 0. The length of the array is the number of retries and
`[]` turns retries off. A `request` or `notify` actor that writes its own `retry_delays` uses
that instead. Because this default is a list, it is named in the log line
(`circular-daemon: config default used: effects.retry_delays=[1000, 1000, 1000, 5000, 5000, 5000, 15000, 15000, 15000]`)
but not in `anchor.config_defaults`, whose entries hold one integer each.

For the five integer operating values, `0`, a negative number or a non-integer is refused
with `ConfigRejected: daemon config <path>: <key> …`, and the daemon
releases its claim and exits. The daemon supplies no default `[process] allowlist`,
`[[notify.channel]]` or `[webhook]` binding. A document without `[http]` gives the HTTP
executor an empty host list; it rejects hosts outside that list with `parameter_denied`.
The first document the daemon writes into a new state reads:

```toml
# Hosts a `request` actor may reach. "host:*" allows every port of that host.
[http]
hosts = ["localhost:*", "127.0.0.1:*", "[::1]:*"]
```

To change an operating value, edit its key in `config.toml`, preserving the other tables.
For example, set `deadline_secs` in the existing `[process]` table, or add that table if it
is absent:

```toml
[process]
deadline_secs = 3600                                  # replaces the default 1800
```

If you create `config.toml` before the first start, include the `[http]` table above to
allow loopback requests. The daemon uses an existing document without adding that table.
Set the file's permissions with `chmod 600 "$STATE/config.toml"`.

Step 5 deploys a template whose programs and webhook are capability bindings, so it needs a
config. Pick the template directory for your lane first; the config below binds two programs
that live inside it:

```sh
# Lane A:
TPL="$PREFIX/src/sdk/typescript/templates/incident-autopilot"
# Lane B:
TPL="$PWD/sdk/typescript/templates/incident-autopilot"
```

The next block replaces `config.toml`, including any existing bindings. For an existing
document, merge these entries into its tables instead of running the `cat >` command.

```sh
mkdir -p -m 700 "$STATE" && mkdir -m 700 "$STATE/secrets"
printf %s "local-secret" > "$STATE/secrets/webhook.bearer" && chmod 600 "$STATE/secrets/webhook.bearer"
cat > "$STATE/config.toml" <<TOML
[http]
hosts = ["localhost:*", "127.0.0.1:*", "[::1]:*"]    # the table of a new state's first document

[process]
max_concurrent = 2                                    # explicit operating limit; no default
allowlist = ["$TPL/bin/remediate", "$TPL/bin/verify"]

# The daemon refuses to start if a secret vault exists without an explicit
# custody choice; it does not choose a backend by itself.
# \`file_vault_v0\` is the plaintext-file vault created above.
[secrets]
custody = { file_vault_v0 = {} }

[webhook]
bind = "127.0.0.1:32180"
bearer = { secret = "webhook.bearer" }
TOML
chmod 600 "$STATE/config.toml"
```

For loopback requests, a state with neither a journal nor `config.toml` needs no hand-written
HTTP table: the daemon's first document allows every port of `localhost`, `127.0.0.1` and
`[::1]`. Any other host needs its own entry in `[http] hosts`;
§8 has the matching rule.

## 3. Launch

```sh
# Lane A:
open "$BIN/Circular.app"
# Lane B (after starting the daemon as described below):
(cd ui/app && npm start)
```

In both lanes the state is chosen in the app (lane A can also open `Circular.app` from Finder).
On **Projects**, **Open project** picks `$STATE`, as does **File → Open Project…**,
and the app attaches to the daemon holding it. In lane A, if none is running, **File → Start
Daemon** runs `circular daemon start --state "$STATE"` (the `circular` beside the app),
which starts `circular-daemon` detached, and then attaches; **New project** makes a new
state directory and starts its daemon the same way. Lane B has no `circular` beside the
development Electron, so there Start daemon and New project answer `CLI_UNAVAILABLE`: start
the daemon by hand as below, then open the project. **File → Reconnect** attaches again.
The app remembers the state: after a restart, open it from the recent
list (Projects, or **File → Open Recent**). Quitting the app does **not** stop the daemon, and reopening it attaches to the
same one. `circular daemon stop --state "$STATE"` stops it.
A script that launches the app with `--state` also names its own profile directory with
`--user-data-dir` and removes that directory after the app has exited; without one the app
prints `PROFILE_REQUIRED` on stderr and exits 1 without opening a window.

Without a desktop session, start the engine alone and skip to step 5:

```sh
"$BIN/circular-daemon" --state "$STATE"    # logs to stderr; ready once it prints "circular-daemon: serving"
```

## 4. Bind an agent harness

In the UI, open **Projects** (top bar) and press **Harnesses** below the project cards, or
click **Local machine** at the bottom of the canvas sidebar. Either one opens the same dialog,
titled **Harnesses for** and the name of the project's state folder. The dialog is a table with
one row per harness: `claude`, `codex` and `pi`, and any other name bound or needed in this
project. A row shows the program bound in this project and how many actors need that harness.
Under a row with no program, put the absolute path of that CLI on this machine in **Program**,
or press **Choose…** to pick the file. When the daemon found the CLI installed, the row shows
**Found:** with its path, and **Use this path** puts that path in the field. Then press the
row's **Bind** button, which names the harness (for example **Bind claude**). A row that is
already bound has no Bind form; to change or remove its binding, use the command line below.
A binding is a setting of this state, not an edit: the daemon checks the name against its
adapters and the path, writes the binding into `<state>/config.toml` `[agent] harnesses`, and
hands it to the standing pipeline. Agents that name the harness take the program between
their turns; nothing is restarted, and the journal records nothing of the binding. Binding
does not log you in — log into the CLI itself first.

The same act from the command line:

```sh
circular harness list --state "$STATE"     # bound names (use verbatim in agent({ harness })),
                                           # and where the daemon found each CLI ("found")
circular harness bind claude --program /absolute/path/to/claude --state "$STATE"
circular harness unbind claude --state "$STATE"   # removes the binding again
```

An agent deployed before its harness is bound stands and waits: a harness call it makes
waits until a binding stands, and its `daemon.health` row is `waiting` with reason
`harness_unbound` meanwhile. A binding commits nothing, so the authoring revision the canvas
and step 5 start from is the same before and after it.

For deterministic reference tests, start the daemon explicitly with
`"$BIN/circular-daemon" --state "$STATE" --reference-agent`. This enables the `reference`
harness, which is not an actual provider. It is off by default; `[agent] reference`
is rejected as an unknown config key. The flag takes no value.

### Start and stop the daemon from the command line

```sh
circular daemon start  --state "$STATE"     # detached; logs to ~/Library/Logs/Circular/direct/
circular daemon status --state "$STATE"     # running (a process holds it) vs answering
circular daemon logs   --state "$STATE" -f  # the daemon's operating log, not the arrival journal
circular daemon stop   --state "$STATE"     # SIGTERM, then waits for the process to leave
```

`daemon status --json` prints the same facts as one line of JSON for an agent to read.
The `lifecycle` text line and JSON field carry the pipeline lifecycle reported by
`daemon.health`. Doctor reports the same fact in its `daemon.lifecycle` row.
Stopping the daemon process is **not** pausing a pipeline: that is `Pause`, from the UI
or the SDK. `circular doctor --state "$STATE"` reports installation, state, socket and
agent-CLI rows with a code and one measured line, with a remedy where provided, and changes
nothing.

### Run the daemon as a LaunchAgent (lane A)

`circular daemon install/uninstall` registers or removes the daemon's LaunchAgent
plist. Registration finds `circular-daemon` in the `bin/` of the installation it
runs from, so it registers the installation `$PREFIX` selects. Lane B has no LaunchAgent
path — a
`target/debug` build is started by hand or by the UI.

```sh
"$PREFIX/bin/circular" daemon install --state "$STATE"
```

Registration uses the exact `--state` path from §2. It creates missing state
directories with mode 0700 and refuses an existing state with unsafe permissions;
it does not change the permissions of an existing state. It validates the daemon executable and
the state before writing the plist.
After installing a new version, register again from `$PREFIX`, which now selects it,
to update the executable path.

Registration prints the daemon plist path. It does not start the daemon.
Copy that printed path into the variable below (including the full filename):

```sh
DAEMON_PLIST="/path/printed/for/daemon.plist"
DOMAIN="gui/$(id -u)"
launchctl bootstrap "$DOMAIN" "$DAEMON_PLIST"
DAEMON_LABEL=$(/usr/libexec/PlistBuddy -c 'Print :Label' "$DAEMON_PLIST")
launchctl print "$DOMAIN/$DAEMON_LABEL"
test -S "$STATE/daemon.sock"
# The daemon stands when launchctl reports state = running and the socket exists.
```

To stop the service and remove its registration, use the same state path:

```sh
launchctl bootout "$DOMAIN/$DAEMON_LABEL"
"$PREFIX/bin/circular" daemon uninstall --state "$STATE"
```

Uninstall removes the plist and preserves application state. It also works
after removing an old installation, when invoked from another available installation.
Registering, bootstrapping and tearing down the LaunchAgent requires a logged-in
macOS session; none of it works from a background or sandboxed shell.

## 5. Deploy a pipeline with the SDK

The SDK authors actors from the captured catalog. Use the exact harness name returned
by the daemon when creating an agent, with the required `queue_capacity`.
The authoring instructions `circular chat` generates include those reported names and
runnable authoring examples, and `circular harness list --state "$STATE"` prints them.
An agent whose harness is not bound stands, and its harness calls wait until a binding stands.
The `map`, `filter`, `bang`, `parse` and `flatten` chain methods are wire preprocessing, not
actors; `reference/combinators.md` describes them. Inside a `map` or `filter` string the
arriving value is named `event`.
Actors that reach outside — `request`, `notify`, `file`, `listener`, and a `tool_executor`
whose tools read, write or spawn — must carry a `capabilities` grant with an explicit
`approval`; the daemon refuses the declaration without it. A **file** actor takes an
absolute **path** and `FsRead` and `FsWrite` grants whose `roots` contain that path. A
relative path is accepted by the declaration, and then the actor fails when it activates.

A `tool_executor` tool may also set `approval: "required"` in its own effect template.
The call waits for approval if either that tool or its capability grant requires it;
`tool_executor` has no top-level `approval` key.

```sh
circular template list
circular template deploy incident-autopilot --state "$STATE" \
  --remediator "$TPL/bin/remediate" --verifier "$TPL/bin/verify"
```

`deploy.mjs` runs a preflight (executables, `$STATE/daemon.sock`, `$STATE/config.toml`)
and then commits the graph. A fresh state has no pipeline until its first commit
(`circular daemon status` reports the lifecycle as `none`); that commit starts it, later commits
add to it, and nothing is started separately.
`$TPL/README.md` is the full walkthrough for the template, in both lanes.

For a demo of a whole on-call loop (an alert, an agent's triage, an approval, an action, a
verification loop and a notification) against the OpenTelemetry demo store, read the
`otel-astronomy` template's README (`$TPL/README.md` with `<name>` = `otel-astronomy`). It
needs Docker and deploys into a fresh state directory of its own.

## 6. Feed it, pause and resume

There is no start button: committed actors are already running. The canvas header's
**Pause** holds new consumption for the pipeline and then reads **Resume**; its menu
(the arrow beside it) has **Force pause**, which also cancels work in flight. In-flight
effects can finish during Pause. Their outcomes are recorded but remain unconsumed until
Resume. For the template, feed it an incident:

```sh
curl -sS -X POST "http://127.0.0.1:32180/v1/ingress/incidents" \
  -H "Authorization: Bearer local-secret" -H "Idempotency-Key: incident-1" \
  --data @"$TPL/sample-incident.json"        # → 202
```

While the pipeline is paused the webhook answers `503` with `Retry-After`: `1` for the first
three refusals, `5` for the next three, then `15` for as long as it keeps refusing, and `1`
again after it accepts one. The same values answer an OTLP receiver whose queue is full.

## 7. Observe and replay

Observation is on the canvas itself (top bar: **Projects · Canvas · Outputs**). Each card
shows its actor's state and latest values, the status line at the bottom counts alive and dead
actors and dead letters, and the **Journal** panel under the canvas (expand it) lists recorded
arrivals — **Selection** narrows it to the selected actor. **Outputs** shows the pipeline's
mounts, where **Pin** on a card keeps it first on this device, and **Requests** in the top bar
counts approvals waiting for you. Each request shows the call it would run — the tool, the
command and its input — read from the recorded arrival that called it, and **Show arrival**
opens that arrival. When the wire into the actor transforms the value (`map` and the like),
the request shows the arrived value and the wire's steps instead of a call.

Per-tool approval holds the selected call until an approval decision allows it to run.
A call whose selected tool and capability grant both require no approval needs no approval
request.

The **Time machine** under the canvas is the replay transport over the one standing
pipeline; nothing is picked first. Its bar is the daemon's `timeline.bins` summary of the
recorded range: arrivals and incidents per bin, and marks for edits, restarts, pauses and
resumes. Arrival counts include activation, edit and stop arrivals on `_lifecycle`.
`arrival.scan` omits those lifecycle arrivals. Click a moment on the bar and the app asks
`timeline.at` for that instant, then opens the replay lens at the answer's cut, one recorded
position per actor. The position shown is the instant that cut stands at, which can be
earlier than the moment clicked. With
the bar focused, ←/→ move one second back or forward (Shift: ten seconds). Pick a speed and
press **Resume** (or Space) to play from it, and press **Live** (or Esc) to return. The
recorded past cannot be edited — edits are made in Live. In the SDK,
`session.replay.start({ from, pace })` with the `target` of a `timeline.at` answer opens the
same lens: the accepted start's `value` is a lens handle whose `rewind` moves it and whose
`end` closes it. A refused replay shows its code on the chip and beside the time machine's caption; the canvas
stays Live rather than looking as if the past had opened.

## 8. Values the engine actually enforces

These are the five facts operators have asked for most often. Each names the actor whose
reference page states it: `reference/actors/<actor>.md` in a published checkout.

**HTTP body ceilings.** The webhook and OTLP receivers reject oversized request bodies
with `413`. Accepted OTLP batches are split into fragments of at most 900 KiB; an item
that cannot fit in one fragment is refused with `413`.

| Receiver | Ceiling |
|---|---|
| general webhook mount, `POST /v1/ingress/<mount>` | 1 MiB = 1,048,576 bytes |
| the `otlp` actor's own `/v1/logs`, `/v1/metrics` receiver | 16 MiB = 16,777,216 bytes |
| a scrubbed OTLP fragment after acceptance | 900 KiB = 921,600 bytes |

Pointing an OTLP collector at the **general webhook** therefore means keeping each
serialized request body under 1 MiB — that is the receiver in §6, not the 16 MiB one.
A recommended batch item count is **undecided**: the byte size depends on the signal
content, so no item count is offered as a stand-in for the byte ceiling. The `DRAIN_LIMIT`
of 1 MiB in the OTLP code is how much is read and discarded *after* a rejection; it is not
an acceptance ceiling. Reference: `otlp`.

**A `request` actor's HTTP executor checks `[http] hosts` before sending.** The daemon
assembles this executor with an empty host list when the document omits `[http]` or sets
`hosts = []`. A host outside the list is refused with `parameter_denied` before sending.
A new state's first document (§2) lists every port of the three loopback spellings:

```toml
[http]
hosts = ["localhost:*", "127.0.0.1:*", "[::1]:*"]
```

An entry is `host`, `host:port` or `host:*`. The first two match the URL's **authority** —
host and optional explicit port — letter for letter. `"127.0.0.1:9090"` matches the authority
in `http://127.0.0.1:9090/…`, but does not match `localhost:9090` or a portless `127.0.0.1`.
`"127.0.0.1:*"` matches every port of `127.0.0.1`, with or without a port in the URL.
There is no wildcard for the host. The
daemon trims each entry. It refuses to start on an empty string, a duplicate, a non-string
or an unknown key under `[http]`, and on an entry that is none of the three forms (a scheme,
a path, a port that is empty, not a number or above 65535, or a host wildcard such as
`*.example.com`); that last refusal reads
`ConfigRejected: daemon config <path>: http.hosts[<i>] "<entry>" is not host, host:port or host:*`.
Plain `http://` is sent only to a loopback host; use `https://` for any other. For
`parameter_denied`, the actor's `_error` lists the request requirements, beginning
`request failed: parameter_denied; hint: at least one of these does not hold: [http] hosts in config.toml lists "<authority>" or "<host>:*"`,
where `<authority>` is the URL's host and optional port as written. The same code also covers
an unresolved secret header or a response containing a vault secret; the hint lists
requirements without identifying which one failed. The response check happens after sending.
The actor's own `method` config accepts exactly the lowercase `"get"` or `"post"` — `"GET"`
is not normalized; it is refused. The
actor also needs
`capabilities: { HttpFetch: { approval: "none" } }` (or `"required"`); without it the
declaration is refused with `config.capabilities = <missing>`, and without the grant inside it
with `config.capabilities.HttpFetch.approval = <missing>`. The
response `body` is Bytes; a `json` parse reads it directly, and `kv` or `regex` need
`string(event.body)` first.
Reference: `request`.

**`notify` requires string `title` and `body` fields.** The inlet `notification` accepts
additional fields but omits them from the delivered notification. The channel comes from the
actor's `channel` config, not from the message. The channel name is bound
to a sink by a `[[notify.channel]]` table in `config.toml`, and the actor needs
`capabilities: { UserNotify: { approval: "none" } }` (or `"required"`).

| Channel sink | What is delivered |
|---|---|
| `slack_webhook` | one JSON `text` field: `*<title>*`, a newline, then `<body>` |
| `system_program` | the absolute executable is called with title as the first argument and body as the second; standard input is left empty and no JSON body is written to it |

`alert`'s `transition` payload is `{from, to}` and does **not** have this shape, so a wire
`map` between them has to build the title and body explicitly — nothing is synthesized:
`.map("{'title': 'Alert ' + event.to, 'body': event.from + ' -> ' + event.to}")`.
Reference: `notify`.

**`alert` fires on a scheduled wakeup, not on a sample count.** There is no polling slot on
`alert`; the sample rate is whatever upstream (`timer`, `request`, …) is wired to it. Each
`event` arrival evaluates the predicate once and a Bool `true` is the violation. An
evaluation error or a non-Bool result is not a sample: the arrival still passes through on
`event`, the state does not move, and a dead letter with the actor-declared reason
`predicate_failed` records it. `firing_delay` and `recovery_delay` are Int
milliseconds greater than zero.

| State and input | What happens | `transition` emitted |
|---|---|---|
| `Ok`, not armed, first `true` | arm one wakeup `firing_delay` later | — |
| `Ok`, already armed, another `true` | keep the existing arming; the delay does not restart | — |
| `Ok`, armed, a `false` | drop the candidate and the arming; a late wakeup is suppressed by correlation | — |
| `Ok`, armed, the matching wakeup arrives | enter `Firing` | `Ok` → `Firing` |
| `Firing`, a `false` | enter `Cooldown` and arm `recovery_delay` | `Firing` → `Cooldown` |
| `Cooldown`, a `true` | clear the recovery arming, return to `Firing` at once | `Cooldown` → `Firing` |
| `Cooldown`, the matching wakeup arrives | return to `Ok` | `Cooldown` → `Ok` |

The `event` outlet passes every input payload through unchanged, violation or not. Elapsed
time or a count of `true` samples alone does not produce `Firing` — the recorded wakeup does.
Reference: `alert`.

**Port names.** Wire by these names; only the primary one may be left implicit.

| Actor | Inlets | Outlets |
|---|---|---|
| `timer` | `bang` (optional) | `tick`, payload `{sequence: UInt}` |
| `request` | `event` | `response`, plus the derived `_error` |
| `alert` | `event` | `event` (pass-through), `transition` (`{from, to}`) |
| `notify` | `notification` (`{title, body}`) | no normal outlet; only the derived `_error` |

`alert`'s inlet and outlet share the name `event` because the two directions are separate
name spaces; `transition` must be wired by name. References: `timer`, `request`,
`alert`, `notify`.

## 9. Logs, stopping, troubleshooting

`circular doctor --state "$STATE"` is the first thing to run; `circular bugreport --state
"$STATE"` writes one local file with the doctor rows, the versions, `daemon.health`, counts
by kind from the first page of up to 4,096 journal records, and a redacted log tail. The
count includes `complete: false` when more records remain. It uploads nothing — read it
before you attach it anywhere.

- Daemon output for a state directory attached by path, when the app or `circular daemon start`
  started the daemon: `~/Library/Logs/Circular/direct/<hash>.log` (hash of the path;
  `circular daemon logs --state "$STATE"` tails it and `circular daemon status --state "$STATE"`
  names the exact file); managed locations: `~/Library/Logs/Circular/<name>/stdout.log` and
  `stderr.log`. A hand-started `circular-daemon --state "$STATE"` logs to stderr only, so the
  file `status` names does not exist for it.
- Stop the daemon: `circular daemon stop --state "$STATE"` (it signals the process holding
  this state's claim file and waits for it to leave), or `pkill -TERM -f "circular-daemon
  --state $STATE"`. SIGTERM/SIGINT end
  this daemon instance only: it stops accepting, records the restart boundary and releases
  the endpoint and claim — it does **not** pause the pipeline. Restart preserves the
  recorded intent. Paused pipelines remain paused. Pipelines whose recorded intent is
  *Running* resume when the same `$STATE` starts again. A restart is an ordinary record in
  the pipeline's history, not a new start. Only an explicit **Pause** (§6, or
  the SDK's Pause) records the pipeline as paused.
- `owner-local state directory mode 0o755 must be exactly 0o700 with no special bits` →
  `chmod 700 "$STATE"`.
- `daemon config <path> must not set group/other permission bits: 0644` →
  `chmod 600 "$STATE/config.toml"`.
- Exit code 4 from a hand-started daemon means another instance already holds `$STATE`.
- Exit code 4 from `circular template deploy` means the commit was sent, its answer did not
  arrive, and the commit record does not show it yet. The stderr line names the CommitId to
  look for. Exit code 4 from `circular harness bind` or `unbind` means the command was sent,
  its answer did not arrive, and `agent.harnesses` does not show it saved yet;
  `circular harness list --state "$STATE"` shows whether it landed, and the same command can
  be called again: both are idempotent.
- `install: <REASON>: …` → the line says what to do; `scripts/INSTALL.md` lists every reason
  and exit code. The installer checks and selects an existing build; it stages a new build
  before placing it in its own directory.
