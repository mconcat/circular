# Contributing

Issues and pull requests are both welcome; open an issue first when the change touches
the execution model described in [AGENTS.md](AGENTS.md).

This page is for changing Circular itself. If you are building a pipeline **on** Circular,
you want [AGENTS.md](AGENTS.md) and [QUICKSTART.md](QUICKSTART.md) instead; nothing here
is needed for that.

## Build it

```sh
cargo build -p engine --bin circular-daemon
(cd sdk/typescript && npm install)
```

Rust stable 1.95 or newer (the workspace is edition 2024), Node.js 22.12 or newer with npm 10, the
Xcode command-line tools, and Python 3.9 or newer. The binaries land in `target/debug`,
and the CLI runs as `node sdk/typescript/circular.mjs`. There is no `cargo install` path.

`scripts/install.sh --source .` installs your checkout's committed `HEAD` the way the one-line
installer installs a release (`scripts/INSTALL.md`). `python3 scripts/build-artifact.py` builds
the prebuilt daemon asset a release publishes; the macOS `python3` is enough for it.

## Run the tests

```sh
cargo test                                  # engine and protocol
(cd sdk/typescript && npm test)             # SDK, CLI, templates
(cd sdk/typescript && npm run check:types)  # SDK type check
(cd ui/app && npm ci && npm test)           # desktop app: node --test test/*.test.mjs
```

The desktop app's screenshot comparison, `node scripts/shot-diff.mjs` in `ui/app`, needs
Google Chrome. `ui/app/README.md` describes it.

Tests that need a real daemon, a socket, or a logged-in desktop session do not run in a
background or sandboxed shell.

## What a change has to hold

These are not style preferences. A change that breaks one of them is wrong even when every
test is green, because the tests are downstream of these.

- **An actor holds only its own things** — its own declaration, its own state, the senders
  for its own outgoing wires. It does not open a shared structure to decide where a message
  goes. If you find yourself reaching for a table that every actor consults, that is the
  bug.
- **Nothing in the centre pushes the graph forward.** No loop walks the actors calling
  them. Actors are tasks; they run.
- **There is one source of truth, and it is the journal.** Do not introduce a second place
  that knows the same fact — a serialized graph document, a cached index, a copy the UI
  holds and edits.
- **There is one authoring surface.** The canvas and the SDK emit the same change verbs. A
  command that only the UI can send breaks the round trip between a human editing and an
  agent editing.
- **No vendor specifics in the engine.** There is no primitive named after a particular
  agent CLI. Concrete things live in configuration and in the
  pipelines composed from it; the primitives stay generic.
- **Nothing is silent.** A discard, a refusal, a downgrade, a normalization — each is a
  value and each is observable. A failure that looks like a success is the worst outcome
  available.
- **No environment variables select product behavior.** Options come from arguments,
  configuration, or an actor's own configuration.

## Changing something published

The wire protocol, the recorded formats, the configuration schema, the SDK's public names,
and the command names and flags are all public surface. Changing one is not a
refactor — it changes what an existing user's pipeline means.

Circular is an alpha. Today a wrong shape is corrected in place, and the old spelling is
refused. For example, `journal.retain` and `journal.retain_derived` are rejected regardless
of their values. For `journal.retain`, startup fails with
`ConfigRejected: daemon config <path>: unknown config key journal.retain. Remove journal.retain from config.toml.`
For `journal.retain_derived`, startup fails with
`ConfigRejected: daemon config <path>: unknown config key journal.retain_derived`.

That includes the formats the daemon stores: a state
directory made with one alpha version may not open in the next (see
[VERSIONING.md](VERSIONING.md)).

## In a pull request

Say what you changed, and say what you did to see it work — the user-visible action, not
only the test name. A change to the engine or the SDK that nobody exercised end to end is
a change nobody has seen work.
