# Circular — how you author a pipeline

You are an AI agent working with Circular. This file is how you author, deploy, observe,
pause and replay a pipeline. It is written for you; the human's surface is the canvas,
where they watch the same program you are editing, approve things, and steer it.

`circular chat` drops a copy of this file into the session it opens for you and appends
the connection details for that machine — the state directory, the socket, whether a
daemon is running, what is deployed right now, and which agent harnesses that daemon
reports. Those details, and the generated authoring instructions beside them, are
authoritative where they are more specific than this file.

---

## 1. The execution model — read this before you write any code

Six sentences. Almost every mistake an agent makes with Circular is one of these six
being assumed away.

1. **An active actor runs as a task.** Check `daemon.health` for its recorded state. It
   holds its own declaration, its own state, and the senders for its own outgoing wires,
   and it does not wait for another actor's turn.
2. **A pipeline has no built-in completion.** It processes arrivals while running;
   pause and failure are distinct from completion.
3. **Branching is fan-out plus a filter on each branch.** There is no if actor and no
   switch actor. You connect the same outlet to every branch unconditionally and each
   destination filters at its own inlet.
4. **A wire's transform belongs to the destination, not to the wire.** `map`, `filter`,
   `bang`, `parse` and `flatten` are inlet preprocessing on the receiving actor. They are
   not actors, they do not appear as nodes, and they are not a place where a value can sit.
5. **Cycles are ordinary.** A back edge is a plain wire. Nothing asks whether a cycle is
   allowed, and feedback is a normal shape rather than an error to design around.
6. **The journal is the authority on what the pipeline is.** Every arrival is recorded
   before it reaches an actor, and the current state is what those records fold to. An SDK
   file is a sequence of change verbs, not a description of the pipeline — see §7.

Canvas gestures and SDK programs use the same declaration commands. Read a fresh
authoring snapshot before editing.

## 2. What you are connected to

Before you touch anything, know these. `circular chat` writes all of them into the
session it opens, in `first-message.md` and in the appended section of this file's copy.

- **The state directory**, as an absolute path. Never infer it from the working directory
  and never guess between two of them. The socket, configuration, secrets and journals
  belong to one state directory; the daemon’s operating log is stored separately.
- **Whether a daemon is running there**, and whether the pipeline is empty (*genesis*, no
  epoch committed yet) or already carries a deployed program.
- **What is deployed right now**, reconstructed for you as SDK code: `current.ts`, or
  `current/main.ts` plus one module per scope when there are nested scopes. An absent
  daemon socket gives you an empty skeleton named `main.ts` — that is not a recovered graph,
  and you must not treat it as "the pipeline is empty".
- **Which agent harnesses the daemon reports** (§4).
- **Versions.** `circular version --state "$STATE"` prints the CLI's and the installed
  daemon executable's; `circular --version` prints the CLI's alone.

Read the deployed code before you change it. "Build something new" and "edit what is
standing" are different jobs: the first writes a whole program, the second writes only the
change, importing what stands from `circular:current`.

## 3. Getting a daemon

A state directory must be an absolute path below the user's home, owned by them, with mode
exactly 0700 — no group or other bits. `<state>/config.toml` must have no group/other bits
either (0600), and the path must be short enough that `<state>/daemon.sock` fits in 103
bytes.

When a daemon start finds neither `journal.sqlite3` nor `config.toml` in the state, it writes
a mode-0600 `config.toml` with `[http] hosts = ["localhost:*", "127.0.0.1:*", "[::1]:*"]`
and logs that write. These entries allow every port of those loopback hosts. An existing
document is used without adding the table. A state with a journal does not receive this
initial document; there, a missing `config.toml` is read as an empty document. Without
`[http]`, the HTTP executor has an empty host list and rejects hosts with `parameter_denied`;
the `request` actor's `_error` includes a hint. Preserve the HTTP table when editing other
settings if the user wants those hosts allowed.

Five integer operating values have defaults when omitted: `process.deadline_secs = 1800`,
`notify.http_timeout_secs = 20`, and
`runtime_arrivals.arrivals_max_mib = 256`, `arrivals_max_records = 500000`, `total_max_mib = 2048`.
The `arrivals_max_mib` and `arrivals_max_records` ceilings apply to the body bytes and
record count recorded in that state's runtime journal, including arrivals, emissions,
observations and effects; those counts do not reset to zero on restart. `total_max_mib`
applies to the combined sizes of the SQLite database, WAL and SHM files.
A document that sets one of them replaces that key only. `daemon.health` names each
default in force in `anchor.config_defaults`, an array of `{key, value}` in document order
(`key` is `<table>.<key>`, `value` a UInt; empty when the document sets all five), so read
it before you tell the user which values are in force. The daemon also logs each one as
`circular-daemon: config default used: <key>=<value>`. A wrong value (`0`, a negative
number, a non-integer) is refused with `ConfigRejected: daemon config <path>: <key> …` and
the daemon releases its claim and exits. Do not write a `config.toml` just to restate the
defaults; write a key only when the user wants a different value.

`journal.retain` and `journal.retain_derived` are rejected regardless of their values.
For `journal.retain`, startup fails with
`ConfigRejected: daemon config <path>: unknown config key journal.retain. Remove journal.retain from config.toml.`
For `journal.retain_derived`, startup fails with
`ConfigRejected: daemon config <path>: unknown config key journal.retain_derived`.

```sh
circular daemon start  --state "$STATE"  # detached, logging outside the state directory
circular daemon status --state "$STATE" --json
circular daemon logs   --state "$STATE"  # the daemon's operating log, not the arrival journal
circular daemon stop   --state "$STATE"
circular-daemon --state "$STATE"         # engine alone in the foreground, logs to stderr
```

`daemon status` separates *running* (a process holds this state's claim file) from
*answering* (`daemon.health` came back). Quitting the UI does not stop the daemon, and
stopping the daemon is **not** pausing the pipeline — see §8.
`daemon.health.items` contains each actor's last recorded health transition: `actor`, `state`,
`reason` (a closed code or null), `since_ms` (UInt), and `record` (the positive UInt arrival-journal
record ordinal, not a commit number or clock). `running`, `waiting`, and `backpressure` report
recorded life; `stopped` and `failed` report recorded death. An actor without a row is unobserved.
`failed` and `backpressure` require a reason. `waiting` carries `harness_unbound` while a call waits
for a harness binding, `harness_unusable` while a call waits because the bound program does not exist
or is not an executable file, and otherwise null. `running` and `stopped` carry null. No elapsed-time threshold
changes health: inactivity is shown by the arrival rate, and the same journal prefix gives the same
health answer. These rows report recorded facts, not a heartbeat or proof that the process is alive now.
`daemon.health` exposes `anchor.journal`, the arrival journal's own storage fact. Its
`ceiling` is null, or the recorded crossing `{code, record, since_ms}` where `code` is a
`journal.<ceilings>_exceeded` code naming every `runtime_arrivals` ceiling crossed. Its
`usage` is null when no run was issued, and otherwise six UInts: `bytes`, `records` and
`file_bytes` measured now, and the configured `arrivals_max_bytes`, `arrivals_max_records` and
`total_max_bytes`. Crossing a ceiling refuses no input and stops nothing; it is an alarm
for the user, who decides what to free.

When something is wrong, `circular doctor --state "$STATE"` gives one row per fact: a
code and one measured line, with a remedy where provided. It diagnoses only — it installs
nothing, starts nothing and writes no settings file, and it marks what it could not measure
`unknown` rather than passing it. `circular bugreport --state "$STATE"` writes those rows
plus versions, `daemon.health`, counts by kind from the first page of up to 4,096 journal
records (with `complete: false` when more remain), and a redacted operating-log tail into
one local file, and uploads nothing.

## 4. Bind an agent harness

An `agent` actor whose harness is not bound still stands. A harness call it makes waits until
a binding stands, and while the call waits the actor's `daemon.health` row is `waiting` with
reason `harness_unbound`. An unbound agent with no call waiting has the same health row as an
idle one, so read `agent.harnesses` to see whether its harness is bound. When the bound program
does not exist or is not an executable file, the call is not started: it waits the same way, with
reason `harness_unusable` and detail code `agent_harness.program_not_executable`, until a binding
is sent again (`SetAgentHarness` accepts only an executable program) or the daemon restarts.

Binding is a separate act from installing or logging into an agent CLI, and Circular does
not do it for the user. A binding is a setting of the state, saved in `<state>/config.toml`
`[agent] harnesses`. It is not a declaration: no epoch carries it and the journal records
nothing of it. In the UI it is the Harnesses dialog, which **Harnesses** on the Projects page
and **Local machine** in the canvas sidebar both open. The dialog has one row per harness and no
name field. Under a harness with no program, the user puts the absolute path of the CLI they are
logged into in **Program** and presses that row's **Bind** button (**Bind claude**, for
example). From the command line, `circular harness list --state "$STATE"` reads the
bindings and, for each harness, the first install location where the daemon found its CLI
(`found`); `circular harness bind <name> --program <absolute path> --state "$STATE"` binds one;
`circular harness unbind <name> --state "$STATE"` removes one. The UI, the CLI and
`setAgentHarness` in `@circular/client` send the same daemon command, `SetAgentHarness`; it is
not a second authoring surface. A binding sent while agents stand reaches them between their
turns, and nothing is restarted. You supply the name and absolute program path the user gives you.

Then read the names back. The daemon answers `agent.harnesses` with one row per name: the
program the standing pipeline holds (`program`) and the program saved in `config.toml`
(`saved`); either can be null. Use a reported name **verbatim**. Do not translate a CLI name
into a harness name. A rejected query means the bindings are *unknown*, not empty — never
fill an empty list with the `reference` executor, which exists only for deterministic tests
and is not a provider.

## 5. Authoring

Every actor you create is a top-level named export. Do not hide a second allocation inside
one initializer. Read `actor.catalog` for actor metadata and fixed ports; its configuration
frames may be marked incomplete. Read `actor.create-inputs` for the complete configuration
schema for creating an actor, including explicit no-configuration and unavailable cases.
The SDK exposes these as `actorCatalog(session)` and `actorCreateInputs(session)` from
`@circular/client`; a connected `circular chat` session includes both in `actor-catalog.md`.
Read the local package declarations and the admission diagnostics instead of inventing
configuration keys or port names.

**First example — one screen.** An input boundary, an actor, and somewhere to look.

```ts
import { input } from "@circular/core";
export let source = input({ label: "values" });
export let counter = source.counter();
export let result = counter.out.count.tap();
source.mount("values", "request");
result.mount("counts", "result");
```

`source.counter()` wires the primary outlet to the primary inlet. `counter.out.count`
names an outlet explicitly. `mount` is what gives you a place to inject into and a place
to read from: a `request` mount is an input boundary, a `result` mount is an observation
point. `actor.events` can also report actor activity without a mount name.

**Second example — branching.** This is the one to learn early, because the habit it
replaces is the expensive one.

```ts
import { input } from "@circular/core";
export let events = input({ label: "events" });
export let failures = events.filter("event.level == 'error'").tap();
export let slow = events.filter("event.ms > 1000").tap();
events.mount("events", "request");
failures.mount("failures", "result");
slow.mount("slow", "result");
```

Both branches receive **every** event; each one decides at its own inlet. There is no
conditional actor in the middle, nothing routes, and an event that matches both predicates
goes down both paths — that is correct, not a conflict. Because the filter belongs to the
destination, you can retune one branch without touching the other.

Map and filter accept CEL strings or supported single-expression arrow functions, which
the prepass lowers to CEL. Unsupported callback syntax is refused before deployment.
Inside a `map` or `filter` string the arriving value is named `event`, whatever the inlet
is called; no other name is declared, so `value.level` is refused at admission.
`reference/combinators.md` describes `map`, `filter`, `bang`, `parse` and `flatten`.

## 6. Deploying

Deployment runs your program through the installed code execution host, which owns
`BeginEpoch → ValidateEpoch → CommitEpoch`. A rejection before commit aborts the epoch;
an unanswered commit has an unknown outcome that must be checked.
An outer host script — not the pipeline program — opens the session, takes a fresh
authoring snapshot for the expected revision and environment, and executes. The generated
instructions in your session carry that script verbatim; use it rather than assembling
your own session.

When an edit is rejected, read the diagnostic and report it. Do not retry it differently
until you know why it was refused.

## 7. Observing

A commit records declarations. Arrivals show the revision under which an input entered;
use `daemon.health` to inspect recorded actor health.

Ask a **finite query first** — read a bounded page and stop — before you open any
subscription. An open subscription that nobody closes is the easiest way for you to hang.
`arrival.scan` takes a registered mount name and reads recorded data arrivals before inlet
preprocessing. It omits activation, edit and stop arrivals on `_lifecycle`. Each returned
arrival carries the revision it entered under and the recorded stamps that caused it.
Use those stamps to link a result back to the request that produced it. Do not pair a
result to a request by position.

An empty page can coexist with recorded `_lifecycle` arrivals. It is not proof that a
computation succeeded, and it is not proof that it failed. Silence from an actor is not
failure either — an upstream that has nothing to send is idle, which is a normal state.
Health, idleness and death must be told apart by what the records say, not by waiting.

Saving means writing a program file inside the state directory. Loading means reading it
back and executing it through the host again. **A file is not a deployment**, and a file
you generated is not the authority on what is standing. Preserve the user's saved versions
and the original before a substantial change.

## 8. Pausing and replaying

**Pausing is a manual user interrupt.** Neither a deploy nor an edit pauses the
pipeline — editing does not require pausing, because untouched actors keep running
while an edited one is replaced. Do not call it around ordinary changes.

Its default is *Pause*: new intake stops, and the response does not wait for in-flight
work. In-flight effects can finish during Pause. Their outcomes are recorded but remain
unconsumed until Resume. *Force Pause* cancels current effects. Resuming is an explicit
action with the expected authoring revision from a fresh snapshot. From the SDK it is one
call on an established session:

```js
const snapshot = await session.authoringSnapshot([], 256);
if (snapshot.status !== "accepted") throw new Error("complete snapshot required");
await session.exchange("Lifecycle", "Resume", {
  // The 32-byte revision digest, not the { kind: "At", revision } object around it.
  expectedAuthoringRevision: snapshot.value.anchor.authoringRevision.revision,
});
```

The answer is a `LifecycleResult`: `[1n, 1n]` is Resumed, `[1n, 2n]` is Paused, and
`[2n, { code, message }]` is a refusal to report as it came back.

Killing the daemon process is **not** Pause. Restart preserves the recorded intent.
Paused pipelines remain paused. Pipelines whose recorded intent is *Running* resume when
the same state directory starts again.

Replay reads journal records. A rewind target is a checkpoint the daemon handed you
whole — its per-actor cut together with the revision epoch that owned it — not a
millisecond and not a position you compute; a cut without its revision epoch is refused.
Observational replay is not a second live injection.

Replay consumes recorded effect outcomes without repeating completed effects. It does
not undo effects that reached the outside world.

## 9. Limits you work under

- **Never read, copy, print or move credentials.** Harness authentication belongs to the
  user's own CLI.
- **Never set an environment variable to select behavior.** Options come from arguments,
  from `config.toml`, or from an actor's own configuration. Nothing here is configured by
  the environment.
- **Write only inside the state directory** the user named. Treat `node_modules` and every
  linked `@circular/*` directory as read-only. Do not access another application’s state
  directory.
- **Never edit a journal.** The record of what happened is not an editable file, and
  nothing you can write there would be true.
- **Do not call a real provider to test something.**
- **Process tools run with a scrubbed environment.** A `tool_executor` tool with
  `effect: "spawn"` needs an absolute program path in the daemon's allowlist, and the child
  inherits neither `HOME` nor `PATH`. Use absolute paths inside the tool script too, and
  test it with `env -i /absolute/path/to/tool` before deploying.
- **Report a rejection; do not route around it.** An incomplete snapshot, an unsupported
  declaration, a refused query — say what came back. A failed read is not an empty
  pipeline, and no answer is neither success nor failure.

## 10. What Circular is not

Circular uses the following execution model.

1. Actors run continuously; a trigger does not start a run that later ends.
2. Branching is unconditional fan-out with a filter at each destination, not a conditional
   node.
3. Transformation on a wire belongs to the destination's inlet, not to a node in between.
4. Cycles and feedback are ordinary structure, not a violation to be checked for.
5. The canvas shows the running program itself; editing it edits what is running, and no
   global stop is involved.
6. The journal defines the current state; an SDK file is a sequence of change verbs, not a
   source of truth you can diff against the pipeline.
7. Replay is deterministic over recorded events; live execution is not, and is not claimed
   to be.
8. Nesting is a scope and a navigation boundary, not a second runtime inside the first.

## 11. Where else to look

The paths below are relative to the directory this file was published in. In a session
that `circular chat` opened, the last section of this file lists the absolute path of each
one on this machine, or says that this installation does not carry it.

| | |
|---|---|
| the session `circular chat` opened | the connection details and the generated authoring instructions, filled in for this machine |
| `QUICKSTART.md` | the human's path from download to a running, observable pipeline |
| `reference/actors/` | one page per actor: ports, configuration, state, rejections, example |
| `reference/combinators.md` | `map`, `filter`, `bang`, `parse` and `flatten`: what each does to a value and what happens when it fails |
| `README.md` | what Circular is, and how to install it |
| `sdk/typescript/templates/<name>/README.md` | a complete pipeline you can deploy and then read; `circular template list` names the ones this installation ships and `circular template deploy <name> --state "$STATE"` runs one |
| `DATA.md`, `VERSIONING.md` | what leaves the machine, what stability means |
