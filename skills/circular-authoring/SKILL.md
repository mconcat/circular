---
name: circular-authoring
description: Build, modify, save and load Circular pipelines with the local TypeScript SDK.
---

# Circular SDK authoring

Read first-message.md and the current program file it names (current.ts, current/main.ts
with its scope modules, or main.ts) before editing. Work only inside the state directory
named in first-message.md. Treat node_modules and all linked @circular/* source directories
as read-only. Never read, copy, print, or move credentials; never change environment variables
to select behavior. Do not access another application’s state directory.
Harness authentication belongs to the user's CLI. Do not call a real AI provider for a test.

Use @circular/core to author, @circular/client to establish an owner-local session, and
@circular/authoring for code execution, and @circular/generator for SDK source reconstruction. The catalog has 26 published constructors,
including json (not constant); compatibility aliases are not new catalog entries.
Use the published configuration input types and the .in/.out port properties. Keep every created actor
in a top-level named export; do not hide multiple allocations inside one initializer.
Use a CEL string or a supported single-expression arrow function for map/filter. Check
local package declarations and actor admission diagnostics instead of inventing ports or
configuration.

## Presentation and `.view()`

Use `.view(kind, config)` for a card body heading, count label, column projections, caption,
recorded total, the closed body roles `title`, `status`, `value`, and the closed choices
`side`, `rows` and `spark`.
Type defaults and actor overrides use one vocabulary in `@circular/core`.
An omitted key inherits; a null key removes its default.
See [Presentation and `.view()`](../../reference/presentation.md) for the keys,
field paths, resolution rule and examples.

## Process tools run with a scrubbed environment

For tool_executor tools with effect: "spawn", program must be an absolute executable
path in the daemon's process allowlist. The child environment is scrubbed: HOME and
PATH are not inherited. Use absolute paths for the interpreter and every subprocess
inside a tool script too; do not rely on shell startup files, $HOME, or PATH lookup.
Test your own tool locally with `env -i /absolute/path/to/tool` and the intended input
before deployment. This environment rule applies to spawned process tools, not the
CLI harness that edits this session. See the incident-autopilot README for setup.

A call on the tool_executor `call` inlet is `{ id, tool, arguments }`. `id` is non-empty
Bytes (a String is taken as its UTF-8 bytes) and comes back as the result's `call`. `tool`
is a key of the actor's `tools` config. `arguments` is Bytes or a String: it is the standard
input of a `spawn` tool and the contents of a `file_write` tool, and `file_read` does not
read it. Any other shape, such as `name` for `tool` or an object for `arguments`, is recorded
as a dead letter. An agent's `tool_request` outlet already emits this shape.
To build a call on a wire, map to CEL bytes literals:
`alerts.out.transition.map("{'id': b'rollback-1', 'tool': 'flag_reset', 'arguments': b''}").into(tools.in.call)`.
The `result` outlet emits `{ call, effect, ok, value }`: on success `value` is the read
Bytes, the written length, or `{ exit, stdout, stderr }` for a spawn; on failure `ok` is
false and `value` is a failure code string. A `parameter_denied` or `diverged` failure also
carries `detail`, a number the tool_executor reference page lists. A tool_executor whose tools use `file_read`,
`file_write` or `spawn` needs the matching `capabilities` entry — `FsRead` and `FsWrite`
with `approval` and absolute `roots`, `ProcessSpawn` with `approval` — and the daemon
refuses the declaration without it.

## Use a bound agent harness name

Runtime agent harness bindings: unknown ("daemon bindings were not read"). Agent examples are unavailable; do not deploy an agent until bindings are reported.

The authoring CLI selected by circular chat --claude|--codex|--pi is separate from the
runtime agent harness. Chat reads agent.harnesses; it does not create that binding.
Use the reported name verbatim in agent({ harness: ... }). Do not translate a CLI name
or a peer adapter name into a harness name. When bindings are available, the agent examples below use a reported name.
Each reported row names the program the standing pipeline holds (`program`) and the program
saved in this state's config.toml (`saved`); either can be null. A row is not proof that a provider can run.

A harness binding is a setting of this state, not a declaration. It is not part of a program,
no epoch carries it, and the journal records nothing of it. If no binding is reported, save agent
code without deploying it and ask the owner to bind a harness. The owner binds one in the app
(the Harnesses dialog, opened from Harnesses on the Projects page or Local machine in the canvas
sidebar) or from a terminal:
`circular harness bind <name> --program <absolute path> --state /absolute/home/state`.
That command sends the daemon's one SetAgentHarness command, the same command the app's Bind
sends. The daemon checks the name and the program, writes that name's entry in config.toml
[agent] harnesses, and hands the binding to the standing pipeline.
`circular harness unbind <name> --state /absolute/home/state` removes the entry.
`circular harness list --state /absolute/home/state` prints the bindings and, for each harness
the daemon has an adapter for, the first install location where the daemon found a program
(`found`). Run bind or unbind only on the owner's instruction, with the exact owner-provided name
and absolute program path. Do not use the chat flag or the reference test executor as a
substitute binding. Start that state with
`circular daemon start --state /absolute/home/state`, or register its LaunchAgents with
`circular daemon install --state /absolute/home/state`.
Those commands manage the daemon only; neither installs nor logs into an
agent CLI, and neither creates an AgentHarness capability by itself.
`circular daemon uninstall --state /absolute/home/state` removes those LaunchAgents again.
Open a chat session on the same state and check its reported binding names before deploying.
An agent actor whose harness is not bound still stands. A harness call it makes waits until a
binding stands, and while the call waits the actor's `daemon.health` row is `waiting` with reason
`harness_unbound`. A binding sent while the agent stands reaches it between its turns; nothing
is restarted.
After binding, open a chat session again or query agent.harnesses with Null args to refresh
the names. Do not guess an allowed-name list or alter credentials. A rejected query
means bindings are unknown, not empty. After deployment, read arrivals to inspect recorded
input and `daemon.health` for actor health folded from recorded health transitions.

The name reference is valid only on a daemon started with --reference-agent. It is a
test harness, not an installation default, and agent.harnesses does not enumerate
that command-line fixture. Never substitute reference when the list is empty.

For owner-local authoring, explicitly pass
`hello: { requestedRoles: [1n, 4n, [2n, []]] }` when calling `establish`.
These are Reader, Operator, and root Writer in canonical Value byte order.
Omitting `hello.requestedRoles` sends an empty role request; it does not grant authoring access.
If the daemon refuses Hello, `ESTABLISHMENT_REJECTED` displays its code, message, and optional hint.
The error's `cause` preserves the original rejection object. Correct the request using that diagnostic.

Five independent port examples (authoring examples, not automatic provider calls):

1. An actor's primary output feeds the next actor's primary input.

```ts
import { input } from "@circular/core";
export let source = input({ label: "request" });
export let count = source.bang().counter();
source.mount("request", "request");
count.mount("counter-input", "result");
```

2. Select a named output explicitly.

```ts
import { input } from "@circular/core";
export let source = input({ label: "request" });
export let count = source.bang().counter();
export let observed = count.out.count.tap();
source.mount("request", "request");
observed.mount("count", "result");
```

3. A constructor can receive its upstream source as an inline input.

```ts
import { json, tap } from "@circular/core";
export let sample = json({ initial: "sample" });
export let observed = tap({ event: sample });
observed.mount("sample", "result");
```

4. A multi-input constructor takes a keyed input map.

The minimum agent config has `harness`, `queue_capacity` and `result`; `agent({})` is
rejected before deployment. `result` names the kind the `result` outlet carries — `"bytes"`
for the harness bytes, `"json"` for one parsed value that downstream reads without a parse
combinator. There is no default: an agent declaration without `result` is rejected.
Examples 4 and 5 use the bound harness name reported above, `queue_capacity: 8n` and
`result: "bytes"`. Keep all three fields when adapting them.

Agent example unavailable until a runtime harness binding is reported. Follow the binding steps above.

5. Connect an existing named output to an existing named input.

Agent example unavailable until a runtime harness binding is reported. Follow the binding steps above.

## Branch by fanning out, never with a conditional actor

There is no if actor and no switch actor, and you must not build one. To send different
arrivals down different paths, connect the same outlet to **every** branch unconditionally
and let each destination's inlet decide with its own filter. Filtering is wire
preprocessing on the receiving actor, so the branches are independent: adding, removing or
retuning one branch does not touch the others, and the same value reaching two branches is
normal rather than a conflict.

```ts
import { input } from "@circular/core";
export let events = input({ label: "events" });
export let failures = events.filter("event.level == 'error'").tap();
export let slow = events.filter("event.ms > 1000").tap();
events.mount("events", "request");
failures.mount("failures", "result");
slow.mount("slow", "result");
```

The prepass lowers supported synchronous single-expression arrow functions with one payload
parameter to CEL. It refuses block bodies, captured variables, calls and other unsupported
callback syntax. Literal CEL strings also work. Inside a map or filter string the arriving
value is named `event`, whatever the inlet is called; no other name is declared,
so `value.level` or `notification.title` is refused at admission.

`arrival.scan` with a branch’s registered mount name reads its recorded data arrivals
**before** preprocessing. It omits `_lifecycle` arrivals. A filtered-out value still appears
there. Put a tap after the filter when you want to see only what passed.

An edit is a program that holds only the change: import what already stands from
circular:current and declare only what is new or different. Running it is the change —
everything it does not mention stays as it is. Do not copy the current program file into
your program; that file is read-only. After an approved commit, `circular edit` regenerates
the session’s current projection from the daemon. If reconstruction fails, it removes that
projection and returns an error; the commit remains accepted. After a direct deploy, open
a fresh chat session to read a new copy.

Current state is the host-resolved virtual module circular:current. Use named exports,
for example `import { count } from "circular:current"; count.setFlags({ bypass: false,
mute: true, pause: false });` when count exists. The host also supplies the named `current`
namespace for key-based lookup. Do not create a disk package named circular:current or
assume an arbitrary actor name exists. Load a complete fresh snapshot before each execution.

An edge is named by its source port, target port and ordinal. A chain or `into` without an
ordinal names ordinal 0 of that pair, in a full program and in a circular:current snippet
alike. If that edge exists, the line replaces its declaration (preprocess steps, delay and
policy) in place, so deploying the same line again still leaves one edge. To change an
existing edge's preprocessing, redeclare it, for example
`import { src, sink } from "circular:current"; src.map("{'v': event.y}").into(sink);`.
A second, parallel wire between the same two ports needs an explicit unused ordinal:
`src.into(sink, { ordinal: 1 })`. Two lines of one program that name the same edge are
refused with CIRCULAR_EDGE_ORDINAL_CONFLICT. Leaving an edge out of a program does not remove
it; disconnecting its current edge handle does. Find the handle by the edge's endpoints:
`import { current, src, sink } from "circular:current"; current.edge(src, sink).disconnect();`.
The first argument is an outlet (`handle.out.<name>`, or a handle with a primary outlet), the
second an inlet (`handle.in.<name>`, or a handle with a primary inlet), and
`current.edge(src, sink.in.event, { ordinal: 1 })` picks a parallel wire; an omitted ordinal
is 0. The handles carry the port ids, so this also finds the edge from an input actor's outlet,
whose id is derived (`_bi1_…`). A missing edge is refused with authoring.current.lookup-missing
and the key that was searched. `current.edge(key)` still takes the edge's snapshot `edge.value`
as JSON text with object keys sorted. `replaceOptions()` on an edge handle changes delay and
policy only; passing `preprocess` to it is refused with CIRCULAR_EDGE_PREPROCESS_REPLACEMENT.

Deployment goes through installCodeExecutionHost. In this SDK that function is internal;
the public call `createCodeExecutionHost({ session })` installs it. Do not import the
internal function by an invented public export. It performs admission and owns the same
BeginEpoch → ValidateEpoch → CommitEpoch path. A rejection before commit aborts the epoch;
a commit whose outcome cannot be confirmed is reported as `unknown`.
An accepted commit records the declaration. Arrivals carry the revision each input entered
under; `daemon.health` reports recorded actor health.
After a deploy, call `waitForAdoption(session, result.commit.cursor)` before injecting or
reporting readiness. It reads the first recorded ActivationOutcome, RevisionAdoptionOutcome
or RecoveryOutcome whose revision is at or after that cursor, and returns that revision.
`adopted` carries `code: null`; `failed` preserves code 22, 24 or 30. A session or subscription
that ends first returns `closed` with its code and reason. Report that result and stop the
injection. The cursor is the numeric runtime revision; `authoringAfter` is a separate digest.
This wait observes asynchronous adoption and does not pause the pipeline.

The chat launcher only prepares files and reads the daemon; deployment is a later action
in the agent session, in response to the user's pipeline request.

For a new fluent SDK program, save this outer host as deploy.mjs in this session directory,
then run `node deploy.mjs <absolute-state-directory> pipeline.ts`. The host, not authored
pipeline code, reads files and owns the socket. This is a live deployment, not a test.
In a session that `circular edit` opened, deploy.mjs is already there and runs only after
the user approves: during the proposal turn, save the program and do not write or run a host.

```js
import fs from "node:fs";
import { randomBytes } from "node:crypto";
import { establish, waitForAdoption } from "@circular/client";
import { connectOwnerLocal } from "@circular/client/owner-local";
import { createCodeExecutionHost, semanticPrepass } from "@circular/authoring";
const session = await establish(await connectOwnerLocal({
  root: process.argv[2], socketName: "daemon.sock",
}), {
  resourceCeilings: { maximumBytes: 1048576, maximumDepth: 64,
    maximumContainerEntries: 4096, maximumStringBytes: 65536 },
  // Reader, Operator, root Writer in canonical Value byte order.
  hello: { requestedRoles: [1n, 4n, [2n, []]] },
  requestTimeoutMs: 5000,
});
try {
  const snapshot = await session.authoringSnapshot([], 256);
  // Project creation already recorded the environment, even before the first authoring commit.
  if (snapshot.status !== 'accepted') throw new Error(`authoring snapshot refused (${snapshot.reason ?? snapshot.status}): ${[...(snapshot.diagnostics ?? []), snapshot.diagnostic].filter(Boolean).map(d => `${d.code}: ${d.message}`).join("; ")}`);
  const program = { entry: "main.ts", modules: new Map([["main.ts", fs.readFileSync(process.argv[3])]]) };
  const prepared = semanticPrepass(program);
  if (prepared.status !== "complete") throw new Error(prepared.diagnostics.map(d => [d.message, ...(d.args ?? [])].join(': ')).join("; "));
  // The snapshot the epoch is fenced by: it binds circular:current handles when the program imports them,
  // and a presentation axis the program does not say keeps its value in the fold.
  const result = await createCodeExecutionHost({ session }).execute(program, {
    targetScope: [], commitId: randomBytes(16),
    expectedRevision: snapshot.value.anchor.authoringRevision,
    expectedEnvironment: snapshot.value.anchor.environment,
    currentSnapshot: snapshot.value,
  });
  if (result.status !== "committed") throw new Error(result.diagnostics.map(d => [d.message, ...(d.args ?? [])].join(': ')).join("; "));
  const adoption = await waitForAdoption(session, result.commit.cursor);
  if (adoption.status !== "adopted") throw new Error(`commit accepted; adoption ${adoption.status} (code ${adoption.code})`);
  // Inject into the revision's request mounts only after this recorded outcome.
} catch (error) {
  process.stderr.write(`${error.message}\n`);
  process.exitCode = 1;
} finally {
  await session.goodbye().catch(() => session.close());
}
```

## 1. Mount results and observe arrivals

A commit records a graph; it does not prove that an input was delivered. Declare request
mounts on input boundaries and result mounts on the receiving actors you want to inspect.
`handle.mount(name, role)` binds that role; there is no three-argument mount form.
`arrival.scan` reads an actor's recorded data arrivals **before wire preprocessing**,
including inputs rejected by filter. It omits activation, edit and stop arrivals on
`_lifecycle`. To see a counter's output, put a tap after its count outlet.
This independent pipeline is the target of the client examples below:

```ts
import { input } from "@circular/core";
export let source = input({ label: "values" });
export let counter = source.counter();
export let result = counter.out.count.tap();
source.mount("values", "request");
result.mount("counts", "result");
```

The following JavaScript runs in an outer client host, using an established `session`
with the same owner-local connection and resource ceilings as deploy.mjs above.
It is not pipeline.ts and must not go through the authoring prepass. Keep imports at the
module top and run the selected action inside the host try block, before session.goodbye.
Reuse the host's randomBytes import. Always close the session in finally.

```js
// observation-helpers: existing Query / QueryResult and accepted result arm.
function show(value) {
  return JSON.stringify(value, (_key, item) => typeof item === "bigint" ? `${item}n` : item);
}
// An accepted result with no argument is the tag itself (1n); one with an argument is
// [1n, argument]. InjectAck and SubscribeAck take the first shape, QueryResult the second.
function accepted(payload) {
  return payload === 1n || (Array.isArray(payload) && payload[0] === 1n);
}
function requireAccepted(answer, verb) {
  if (answer?.kind?.verb !== verb || !accepted(answer.payload)) {
    throw new Error(show(answer?.payload));
  }
  return answer.payload === 1n ? null : answer.payload[1];
}
async function queryPage(session, name, args, lens) {
  // Any registration may answer More; the daemon decides, not a list kept here.
  // Keep one held correlation, name and args until Complete and echo its cursor.
  // A replay lens is named on every page; without one the read is live.
  const stream = session.hold(name);
  const items = [];
  const issued = [];
  let anchor, cursor, first = true;
  try {
    for (;;) {
      // The first request carries no page, which every registration accepts. A continuation
      // asks for 256 rows; the daemon may return fewer to fit its ceilings.
      const request = cursor === undefined ? { name, args } : { name, args, page: { limit: 256n, cursor } };
      if (lens !== undefined) request.lens = BigInt(lens.correlation);
      await stream.send("Query", "Query", request);
      const answer = requireAccepted(await stream.next(5000), "QueryResult");
      if (!Array.isArray(answer.items)) throw new Error("query items required");
      if (first) { anchor = answer.anchor; first = false; }
      else if (show(anchor) !== show(answer.anchor)) throw new Error("query anchor changed");
      items.push(...answer.items);
      if (answer.terminal === 2n) return { anchor, items, terminal: 2n };
      const end = answer.terminal;
      // The cursor is the daemon's opaque record. Send it back unchanged; never read a
      // position out of it, compare it as a number, or build one of your own.
      if (!Array.isArray(end) || end.length !== 2 || end[0] !== 1n
          || end[1] === undefined || end[1] === null || answer.items.length === 0
          || issued.some((prior) => show(prior) === show(end[1]))) {
        throw new Error("complete query required: " + show(end));
      }
      cursor = end[1];
      issued.push(cursor);
    }
  } finally { stream.release(); }
}
```

Query `arrival.scan` with the mount name, not an invented actor address.
Several registrations — `timeline` and `actor.events` among them — answer a long result
one page at a time, so treat every query as pageable: More is `[1n, cursor]`, and the next
request repeats the same correlation, name and args with that cursor in `page.cursor`.
Stopping at the first page silently truncates the run. `query.catalog` reports each
registration's own `paging` if you need it; the loop above does not need to ask.
A rejection (including code 19, `query result encoding failed`)
is a failure even after earlier pages; never display those pages as a complete projection. Each item is
an object with `role` (which bound export role answered: request, progress, result or error), `kind`
(the recorded origin kind), `origin` (external-origin bytes, or null), `at` (this arrival record's
stamp), `causal_parents` (the recorded stamps that caused it) and `body`. One query answers every bound
role of that export, so the result whose `causal_parents` carries a request row's `at` is that
request's result. Do not pair a result to a request by position. An empty page can coexist
with recorded `_lifecycle` arrivals. It is not proof of a successful computation.
For live frames, subscribe to `actor.events` **before** sending the input. The existing
subscription client sends Subscribe, Credit and Unsubscribe on one held correlation.
This stream also carries health transitions; inspect its payload and Live/Replay origin
instead of treating every frame as a result. A receive timeout means no frame yet.

```js
// observe-example: run after deploying the values/counts pipeline.
import { openSubscription } from "@circular/client/subscription";
const live = await openSubscription(session, { target: "actor.events", args: null, initialCredit: 8 });
try {
  if (!accepted(live.ack)) throw new Error(show(live.ack));
  await injectValue(session, 11n); // helper in section 2; this is a real input
  const frame = await live.receive(3000);
  console.log(frame === null ? "No frame yet; inspect the arrival page" : show(frame));
  console.log(show((await queryPage(session, "arrival.scan", "counts")).items));
} finally {
  const closed = await live.close();
  if (!accepted(closed)) throw new Error(show(closed));
}
```

## 2. Inject a value through a request mount

Inject is EventInjection/Inject and answers InjectAck. `mount`, `payload`, and
`idempotency` are its existing fields. `mount` is the mount's address `{ local, scope }`:
its name and the scope it was declared in. A mount declared in the root program has
`scope: []`; one declared in a nested scope module carries that scope's segments in order,
each `[1n, name]`. A bare name string is refused. The key is Bytes, not a number array.
A new user action gets a new key; if deliberately retrying the same action, preserve
that action's original key and payload. Do not automatically retry a timeout.
The mount must already be declared: the values request above accepts this input.

```js
// inject-example: outer host helper; uses the randomBytes import from deploy.mjs.
async function injectValue(session, payload) {
  const request = { mount: { local: "values", scope: [] }, payload, idempotency: randomBytes(16) };
  return requireAccepted(await session.exchange("EventInjection", "Inject", request), "InjectAck");
}
// For injection without a subscription: await injectValue(session, 11n);
```

InjectAck acceptance means the input was accepted. Inspect counts with arrival.scan
or actor.events afterward; a deployment acknowledgement alone is not this evidence.
Agent examples still use the daemon-reported harness name inserted above. These
observation helpers do not select a provider or replace a missing binding with reference.

## 3. Pause, and replay recorded history

Pause is a **manual user interrupt**, not a deployment or edit step. Its default mode
is Pause: stop new intake and return without waiting for in-flight work. In-flight effects
can finish during Pause. Their outcomes are recorded but remain unconsumed until Resume.
Force Pause cancels current effects.
Pause applies to the pipeline in the connected state directory.
Pause on an already paused pipeline is refused; Force Pause on it is accepted and cancels the
turns that Pause left running. Report a refusal as it came back. `mode` is optional and omitting it means Pause; the owner-local session lowers the
two spellings `"Pause"` and `"ForcePause"` for you. Do not send any other mode value.

```js
const paused = await session.exchange("Lifecycle", "Pause", { mode: "Pause" });
if (paused?.kind?.verb !== "LifecycleResult" || !accepted(paused.payload)) throw new Error(show(paused?.payload));
```

Replay is not a second execution and it does not need a pause: the live pipeline keeps running.
`session.replay.start({ from, pace })` opens a **lens** — a reading clock at a recorded
coordinate — on a correlation key of its own. A read goes through the lens only when it names
it: `lens: BigInt(lens.correlation)` in a Query body (records, actor.events, display.rollup,
arrival.scan, daemon.health, dead.letters, instance.transitions) or `lens` in
`openSubscription` (the records and actor.events subscriptions). Those reads answer with
the same shapes up to the lens position. Reads that name no lens stay live, and opening, moving or
ending a lens does not touch them.
The coordinate is a timeline checkpoint handed back whole — its `stream`, per-actor
`cut` and the `revision_epoch` that owned it. There is no run number origin and no
milliseconds position; a checkpoint missing a place is refused, not completed.

The accepted start's `value` is the lens. `lens.rewind({ to, pace })` moves it backward or
forward (the same checkpoint with a new pace is a speed change; without `to` only the pace
changes) and `lens.end()` closes it. A pace is `"Free"` (straight to the recorded end),
`"Paused"`, `{ kind: "Step", upto: checkpoint }` or `{ kind: "Realtime", num, den }` — a
multiple of the recorded intervals, `2n/1n` being twice as fast. A subscription naming the
lens ends with ResetRequired when the lens moves backward (open it again, naming the lens) and
with TargetGone when the lens ends. Stop on any rejected result and show it.

```js
// replay-example: run only when the user asks to replay this pipeline's recorded history.
const timeline = await queryPage(session, "timeline", null);
if (timeline.items.length < 2) throw new Error("Need an earlier recorded timeline checkpoint");
const earlier = timeline.items[0]; // the daemon's checkpoint, handed back whole
function requireReplay(result) {
  if (result?.status !== "accepted") throw new Error(show(result));
}
// The live pipeline keeps running; only the reads that name the lens read the recorded clock.
const opened = await session.replay.start({ from: earlier, pace: "Paused" });
requireReplay(opened);
const lens = opened.value;
try {
  const before = await queryPage(session, "actor.events", null, lens); // history up to `earlier`
  // Play forward at twice the recorded speed; reads naming the lens follow its clock.
  requireReplay(await lens.rewind({ to: earlier, pace: { kind: "Realtime", num: 2n, den: 1n } }));
  console.log(show({ from: earlier, arrivals: before.items.length }));
} finally {
  requireReplay(await lens.end());
}
```

A paused pipeline resumes only by an explicit Lifecycle/Resume action whose
`expectedAuthoringRevision` is the 32-byte revision digest from a fresh complete authoring
snapshot — `snapshot.value.anchor.authoringRevision.revision`, not the
`{ kind: "At", revision }` object around it. The owner-local session lowers that field for
the daemon. Do not call Pause automatically around ordinary edits or around a replay.

```js
// resume-example: run only when the user asks to resume a paused pipeline.
const fresh = await session.authoringSnapshot([], 256);
if (fresh.status !== "accepted") throw new Error(show(fresh));
const resumed = await session.exchange("Lifecycle", "Resume",
  { expectedAuthoringRevision: fresh.value.anchor.authoringRevision.revision });
if (resumed?.kind?.verb !== "LifecycleResult" || !accepted(resumed.payload)) throw new Error(show(resumed?.payload));
```

Saving means saving the SDK program file inside this state directory. Loading means reading
that file and executing it through the installed host again. Files are not deployments.
Preserve the user's saved versions before a substantial change. Do not edit the current
program file; it is read-only.

The reconstructed program (current.ts, or current/main.ts with its scope modules) comes
from the SDK's program generator: exported actors, upstream chains, inlet maps and into
for explicit edge attributes. It is read-only: do not run it or copy it. Run your change
program through the installed code execution host above. Read a fresh snapshot for the
expected revision and environment.
Concrete scopes reconstruct to current/main.ts and current/scopes/<name>.ts, recursively.
Read every module in that bundle before editing; it is read-only. When you execute a program,
pass its complete entry/modules map to the host.
Unsupported commands still reject reconstruction with their exact diagnostic.
To add a nested pipeline, declare `pipelineActor({ source: "scopes/<name>.ts", in: [...], out: [...] })`
and write the child as its own module whose `projectInput` and `projectOutput` topics match
`in` and `out`.
An `input()` or `output()` written in the child module is a port of the container too. The parent's
`in` and `out` do not name it, and the reconstructed program prints it as `projectInput` or
`projectOutput` with its label in `in` or `out`.
Give every actor, including the child's boundary actors, its own exported
binding, and pass the complete entry/modules map to the host. The `pipeline_actor` reference
page has a two-file example. The function `template` form's children do not appear on the
canvas in this release.

If offline installation failed, first-message.md describes the workspace link fallback.
Do not run npm install through that shared node_modules link or modify the SDK to fix a
session import. Restore installed local dependencies in the owning workspace separately.
Do not download packages or obtain credentials as a workaround. If the daemon is absent,
main.ts is an empty skeleton; create/save code until the user
provides a running daemon. Report incomplete snapshots and unsupported declarations rather
than treating a failed read as an empty pipeline.
