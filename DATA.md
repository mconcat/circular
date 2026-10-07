# What Circular stores, and what leaves your machine

"Runs locally" does not mean "nothing leaves the machine". It means the daemon, the app
and the CLI send no telemetry, while the pipelines **you** author can reach the network
because you told them to. Those are different things and this page keeps them apart.

## 1. Where your data lives

The **state directory** holds daemon state and pipeline data. One daemon holds one state
directory; operating logs and bug reports may be stored elsewhere.

| Inside `<state>` | What it is |
|---|---|
| `daemon.sock` | the local socket the UI and CLI connect to |
| `config.toml` | the daemon's configuration, including capability bindings and the agent harness bindings (`[agent] harnesses`) that `circular harness bind` and the app's **Bind** buttons write |
| `secrets/` | secret material for the `file_vault_v0` backend selected in `config.toml` |
| journals and recorded history | the recorded arrivals the pipeline's state is folded from, and what replay reads |
| `workspace/` | the working directory of the programs the daemon starts. Each agent harness turn runs in its node's own folder here, and that folder, with the login and session paths the harness declares, is the only place the harness may write. A `tool_executor` `spawn` program starts in `workspace/` but is not confined to it. A `file` actor and a `tool_executor` file tool get no access from it: they read and write only the absolute paths under the `roots` their declarations grant. |
| `chat/<timestamp>-<agent-cli>/` | one directory per `circular chat`, holding that session's program files and generated instructions |
| `chat/defaults.json` | the agent CLI `circular chat` last used for this state, so it does not ask again |
| `cache/actors/<stream>/<hash>.ckpt` | one file per actor: a cache of that actor's state at a turn boundary, used only to shorten restart. The format is internal. Nothing but restart reads it; no query or SDK call returns it. When a file is missing, damaged or does not match the actor's recorded arrivals, the daemon replays that actor from its recorded arrivals instead and writes a `checkpoint_cache_miss` line with a reason to its log. After the `cache/` directory is deleted, the next restart replays each actor from its recorded arrivals. |

The state directory must be below your home directory, owned by you, with mode 0700. The
daemon refuses to open one that grants access to anyone else, and tells you the fix.

Daemon logs are outside the state directory: `~/Library/Logs/Circular/`.
`circular daemon logs --state <dir>` tails the one for that state. A daemon you started by
hand in the foreground logs to stderr instead.

`circular bugreport --state <dir>` writes one local file holding the doctor rows, the
platform, the versions and installation paths, the state directory's path with a file count
and byte total per directory inside it, the daemon's process id, `daemon.health`, counts by
kind from the first page of up to 4,096 journal records, and a redacted tail of that
operating log. The count includes `complete: false` when more records remain. It carries no
journal contents and no `config.toml`, and it uploads nothing: you decide what to do with
the file. Read it before you attach it anywhere — the redaction is a pass over known secret
shapes, not a proof.

These diagnostic commands upload nothing. Local diagnostics also read state metadata
and operating logs.

## 2. Installation and local connections

Circular requires no account or sign-in. The daemon uses a local control socket; configured
webhook and OTLP receivers open loopback TCP listeners. The CLI talks to the control socket.

Release installation contacts GitHub for the daemon asset, checksum and source. The
installer downloads or builds assets, installs dependencies and Electron as needed, and
checks the daemon and CLI versions.

## 3. What a pipeline you author can send

These are the outbound paths, and each one exists only because a pipeline declared it.

| Actor | Where it can reach | What bounds it |
|---|---|---|
| `request` | HTTP hosts allowed by `config.toml` | `[http] hosts` entries `host` and `host:port` match the URL's authority exactly; `host:*` matches that host with or without an explicit port. The HTTP executor refuses a host outside the list with `parameter_denied` before sending. Plain `http://` is restricted to loopback hosts. Methods are lowercase `"get"` and `"post"`. |
| `notify` | an HTTP endpoint, or a program on your machine | `notify.http_timeout_secs`, which defaults to 20 seconds; the daemon logs the default when it uses it. A notifier program receives the title as its first argument and the body as its second, with empty stdin. |
| `agent` | whatever the bound agent CLI talks to | the CLI is yours, already installed and already logged in. Circular runs it; the network conversation is between that CLI and its provider, under that provider's terms, and Circular holds no credential for it. |
| `peer` (experimental) | other agent sessions through a configured external adapter, or test peers through the built-in `memory` adapter | The Claude adapter uses `sessions_dir` for advertisements and `socket_dir` for sockets. External adapters need their own bindings. |
| `tool_executor` with `effect: "spawn"` | a program you allowlisted | `process.allowlist` in `config.toml` — absolute paths only. The child process inherits neither `HOME` nor `PATH`. |

When a daemon start finds neither `journal.sqlite3` nor `config.toml` in the state, it writes
an initial document with `[http] hosts = ["localhost:*", "127.0.0.1:*", "[::1]:*"]` and logs
that write. This allows `request` actors to contact every port of those loopback hosts.
An existing document is used without adding these hosts; a state with a journal does not
receive this initial document. A document without `[http]`, or with `hosts = []`, gives the
`request` HTTP executor an empty list, so it denies hosts with `parameter_denied` and the
actor's `_error` includes a hint. These host settings apply to `request`, not to all of the
outbound paths in the table.

## 4. What can reach in

| Path | What bounds it |
|---|---|
| the webhook mount | you set `[webhook] bind` to a loopback address such as `127.0.0.1:32180`; the daemon refuses an address that is not loopback. You also set a bearer token drawn from your secret vault. Bodies over the ceiling are answered `413`. |
| the `otlp` source | a localhost-only OTLP/HTTP JSON receiver, on a port an authored pipeline names. |
| `peer` (experimental) | the `peer` declaration’s `inbound_policy` decides which senders it accepts through its adapter. |
| `listener` | reads files a declaration names, under roots that declaration lists. With `capabilities.FsRead.approval: "required"` the first read waits for you to approve it. |

The daemon opens the webhook socket from its configuration at startup, before any
deployment declares a mount. A valid, authenticated request receives `503` before a
deployment, or `404` if a deployment is present but the requested mount does not exist.

## 5. Harness settings Circular may modify

For a `claude` agent turn, the daemon sets `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` in
that turn's environment. `circular chat` writes its files inside the session directory it
creates under `<state>/chat/`, not into your agent CLI's settings.

A harness binding (`circular harness bind`/`unbind`, or a **Bind** button in the app) changes
one entry of `<state>/config.toml` `[agent] harnesses`: the daemon rewrites or removes that
name's entry and leaves the file's other lines and comments as they were. It does not touch
the agent CLI's own settings, and the journal records nothing of it.

## 6. Secrets

This release loads only the explicitly selected `file_vault_v0` backend from
`<state>/secrets/`. The daemon refuses to start when a vault exists and the configuration
has not made an explicit custody choice; it does not choose a backend by itself.

The file-backed vault stores secrets as plain files. That is what it is for and what it
says it is; choose it knowing that.

## 7. Questions this page does not answer

- **Replay and effects already sent.** Replay consumes recorded effect outcomes without
  repeating completed effects; it does not undo an HTTP request or notification.
- **Journal storage.** The journal keeps every record the pipeline wrote.
  The journal grows with the pipeline's input; nothing is deleted automatically.
  There is no command that deletes records.
- **Journal ceilings are alarms, not quotas.** The three `runtime_arrivals` values are
  ceilings you choose. When the journal crosses one, no input is refused and the pipeline
  keeps running; `daemon.health` reports the crossing as `anchor.journal.ceiling` (a
  `journal.<ceiling>_exceeded` code) and the sizes now as `anchor.journal.usage`, and the app
  shows the same code and sizes in its status bar. Freeing the space is your decision.
