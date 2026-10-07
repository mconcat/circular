# Astronomy Shop on-call — a demo over the OpenTelemetry demo store

**This is a demo.** It shows one complete on-call loop running as a Circular pipeline against
a real, noisy system: the [OpenTelemetry Astronomy Shop](https://github.com/open-telemetry/opentelemetry-demo).
Your own pipelines start from an empty canvas; this one exists so you can watch every part
of a loop move in a few minutes.

```text
timer ─▶ request (Prometheus) ─▶ parse/flatten/map ─▶ alert
                                                       │ fires
                        ┌──────────────────────────────┴──────────────────────┐
                        ▼                                                     ▼
                 agent (claude) ─▶ notify "oncall"          flag_off, waiting for your approval
                                                                              │
                                        ┌─────────────────────────────────────┘
                                        ▼
               tool_executor (page, flag_off) ─▶ debounce 90 s ─▶ request (same question)
                        ▲                                                     │
                        └────────── still failing: page again ◀── verdict ────┤
                                                                              ▼
                                                                  resolved: notify "oncall"
```

- Every 15 seconds the pipeline asks the demo's Prometheus how many orders failed per
  minute (checkout requests that ended in an error).
- When failed orders persist, the `alert` fires. Two things happen at once: a `claude` agent
  writes a short triage note for the on-call channel, and a rollback — turn the demo's fault
  flags off — is proposed and waits for your approval.
- After an action, the pipeline waits 90 seconds and asks Prometheus the same question again.
  Recovered: it says so on the on-call channel. Still failing: it pages again, and that page
  starts the same wait and re-check. That is a loop in the graph, not a retry setting.

`program.ts` is the whole pipeline as SDK code. `deploy.mjs` fills in three values (this
directory, the Prometheus address and the demo's address) and commits it.

## What you need

- Circular installed and working through step 4 of `QUICKSTART.md`: the `circular` shell
  function and `$BIN` for your lane, and the `claude` CLI logged in.
- Docker, with several GB of memory to spare. The demo is heavy.
- `make` and `/usr/bin/python3`. Both come with the Xcode command line tools
  (`xcode-select --install`); the flag scripts use only Python's standard library.

Set the template directory for your lane. `pwd -P` writes the path with symbolic links
resolved. The process allowlist compares paths as written, and `deploy.mjs` declares its
programs by the resolved path, so the allowlist below has to use the same spelling:

```sh
# Installed with the one-line installer (scripts/install.sh):
TPL="$(cd "$PREFIX/src/sdk/typescript/templates/otel-astronomy" && pwd -P)"
# Source build, from the repository root:
TPL="$(cd sdk/typescript/templates/otel-astronomy && pwd -P)"
```

## 1. Start the OpenTelemetry demo

```sh
git clone https://github.com/open-telemetry/opentelemetry-demo.git
cd opentelemetry-demo
make start
```

Start it with `make start`, the demo's own start command. It brings up the full demo with
its observability services, Prometheus among them. A plain `docker compose up` starts only
the core services, without Prometheus, and the alert would not fire.

The first start pulls several GB of images. When it is up, Prometheus answers at
<http://localhost:9090>, and the store's front door (the store, and its feature flag editor
at `/feature`) is at port 8080. The demo's load generator places orders on its own, so
there is traffic to watch without doing anything.

Every later step reaches the front door through `DEMO`. Set it in the terminal where you run
the steps below:

```sh
DEMO=http://127.0.0.1:8080
```

If port 8080 is taken on your machine, pick a free port, for example 18080. Before
`make start`, add these three lines to the demo's `.env.override`. `ENVOY_PORT` moves the
front door. `FRONTEND_PROXY_ADDR` points the demo's collector at it, and `LOCUST_HOST` points
the demo's load generator at it; without `LOCUST_HOST` nothing places orders.

```sh
ENVOY_PORT=18080
FRONTEND_PROXY_ADDR=frontend-proxy:18080
LOCUST_HOST=http://frontend-proxy:18080
```

Then set `DEMO=http://127.0.0.1:18080`. Prometheus stays at port 9090.

## 2. A fresh state directory

Deploy the demo into a state of its own. A template deployed into a pipeline that already
has actors can replace actors of the same name, so `deploy.mjs` refuses a state that has any.

With neither a journal nor `config.toml`, the daemon writes a first document allowing
`localhost:*`, `127.0.0.1:*` and `[::1]:*`. The block below writes the document before that
start and limits `request` actors to the demo's Prometheus authority, `127.0.0.1:9090`.
It replaces any existing `config.toml`; merge its entries by hand if you have settings to
preserve.

```sh
STATE="$HOME/.circular/astronomy-demo"
mkdir -p -m 700 "$STATE"
cat > "$STATE/config.toml" <<TOML
# The HTTP executor denies a host outside this list with parameter_denied before sending.
# Omitting [http] gives it an empty list; the request actor's _error includes a hint.
[http]
hosts = ["127.0.0.1:9090"]

# The two action programs the pipeline may run.
[process]
max_concurrent = 2
allowlist = ["$TPL/bin/page", "$TPL/bin/flag-off"]

# The on-call channel. The demo's stand-in pager receives the title and body.
[[notify.channel]]
name = "oncall"
sink = { system_program = "$TPL/bin/page" }
TOML
chmod 600 "$STATE/config.toml"
```

## 3. Start the daemon and bind `claude`

In the same terminal:

```sh
"$BIN/circular-daemon" --state "$STATE" &   # wait for the "circular-daemon: serving" log line
circular harness list --state "$STATE"      # "found": where the daemon found each agent CLI
circular harness bind claude --program /absolute/path/to/claude --state "$STATE"
```

## 4. Deploy

```sh
circular template deploy otel-astronomy --state "$STATE" --demo "$DEMO"
```

It prints what it added: nine actors and the ten wires between them. Open the app, choose
`$STATE` with **File → Open Project…** (or **Open project** on Projects), and the canvas shows
the pipeline standing: `poll`, `orders`, `failing`, `triage`, `oncall`, `act`, `settle`,
`recheck` and `verdict`. It is running already; nothing else starts it.

## 5. Break the shop and watch

Make every payment fail:

```sh
"$TPL/bin/flag-on" "$DEMO" paymentFailure 100%
```

Prometheus receives the demo's numbers once a minute, so allow one to two minutes. The
whole loop takes five to eight minutes, most of it waiting for Prometheus. Then:

1. **The alert fires.** `failing` moves to *Firing*.
2. **The triage turn.** `triage` runs one `claude` turn, usually in a few seconds, and its
   note arrives at `oncall`.
3. **The approval.** **Requests** in the top bar counts 1: the proposed `flag_off` call
   waits there with the alert that caused it. Approve it.
4. **The action.** `act` runs `flag-off` at once. The flag editor at
   `$DEMO/feature` shows `paymentFailure` back at `off`.
5. **The verification loop.** 90 seconds after the action, `recheck` asks Prometheus again
   and `verdict` holds the answer. The numbers lag by up to two minutes, so the first
   re-check can still see failures. Then `act` pages again, and that page starts the same
   wait and re-check.
6. **The notify.** When `verdict` finds no failed orders, `oncall` sends
   *Astronomy Shop: orders recovered* to the stand-in pager, and its card reads
   `notification_delivered`. Two minutes later `failing` is back at *Ok*.

## 6. Put it back

Turn the fault off by hand at any time; it is safe to run twice:

```sh
"$TPL/bin/flag-off" "$DEMO"
```

Stop the daemon with `circular daemon stop --state "$STATE"`, and the demo with
`make stop` in its directory.

## What the scripts touch

- `bin/flag-on` and `bin/flag-off` change the `defaultVariant` of the named flags in the
  demo's flag file (`src/flagd/demo.flagd.json` in the demo checkout), and nothing else.
  They go through the demo's own flag editor API, `<demo>/feature/api/read-file` and
  `<demo>/feature/api/write-to-file`, the same one the page at `/feature` uses, and flagd
  picks the change up by itself. `flag-off` with no flag named turns off every flag that
  is not `off`.
- `bin/page` prints the page and exits. It stands in for your paging service.
