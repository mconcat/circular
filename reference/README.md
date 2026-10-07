# Circular actor reference

Circular pipelines are made of **actors**: long-running, independent units that each own
their own configuration and state, receive values on **inlets**, and emit values on
**outlets**. You wire them together with edges; every arrival is recorded before the actor
sees it, and the running state of an actor is the fold of the arrivals it has accepted.

This reference documents the **26 published actor constructors** — what each one is for,
which ports it has, which configuration keys it takes, what it remembers between arrivals,
and how it refuses bad input.

It is written for a person or an AI agent authoring a pipeline against the published
TypeScript surface (`@circular/core`). It does not describe engine internals.

## When this page and the daemon disagree

The running daemon publishes the authoritative catalog. Its **admission diagnostics** —
the rejections it returns when you send it a program — are the final word on port names,
configuration keys, value spaces, and required/optional status. If a page here disagrees
with a diagnostic from your daemon, the diagnostic is right and the page is stale.

The `actor.catalog` query publishes actor metadata and fixed ports; it may mark
configuration frames as incomplete and ports as configuration-dependent. Query
`actor.create-inputs` for the complete configuration schema for creating an actor. It
also explicitly identifies actors that need no configuration and schemas that are
unavailable. An incomplete catalog entry alone does not mean the actor cannot be created.

The SDK exposes these queries as `actorCatalog(session)` and `actorCreateInputs(session)`
from `@circular/client`. When connected to a daemon, `circular chat` includes both query
results in the session's `actor-catalog.md`. The SDK uses `actor.create-admission` to
admit an authored configuration and resolve its ports.

## The actors

[Presentation and `.view()`](presentation.md) documents body headings, field roles,
column projections, captions, recorded totals and type-level config slot text.

| Actor | What it does |
| --- | --- |
| [`route`](actors/route.md) | Sends each event to exactly one configured case outlet, or to `unmatched`. |
| [`pipeline_actor`](actors/pipeline_actor.md) | Opens a child scope and projects that child's declared boundary as its own ports. Use the module `source` form for children editable on the canvas. |
| [`debounce`](actors/debounce.md) | Holds the latest event and emits it once the input has been quiet for a window. |
| [`alert`](actors/alert.md) | Passes every event through and emits a transition when a predicate's held state changes. |
| [`tap`](actors/tap.md) | Passes payloads through unchanged and gives you a named observation point. |
| [`input`](actors/input.md) | Declares an inlet on the pipeline that contains it. |
| [`output`](actors/output.md) | Declares an outlet inside a nested pipeline. Use `projectOutput({ topic })` in the child module. |
| [`replicator`](actors/replicator.md) | Routes events into keyed child cells minted from a template. |
| [`agent`](actors/agent.md) | Runs one step of an external agent harness at a time. |
| [`counter`](actors/counter.md) | Counts accepted events and emits the running count. |
| [`ema`](actors/ema.md) | Computes an exponential moving average over numeric samples. |
| [`windowed_reduce`](actors/windowed_reduce.md) | Periodically folds the samples in a recent time window with an authored expression. |
| [`timer`](actors/timer.md) | Emits one sequenced tick per configured interval. |
| [`tool_executor`](actors/tool_executor.md) | Executes one allowlisted filesystem or process tool call at a time. |
| [`notify`](actors/notify.md) | Submits user notifications on a channel with a configured minimum interval. |
| [`peer`](actors/peer.md) | Exchanges durable asynchronous messages with external agent sessions. Experimental: external sessions need an enabled adapter; `memory` is built in for tests. |
| [`listener`](actors/listener.md) | Listens to an external origin and replays its history on a control pulse. |
| [`keyed_reduce`](actors/keyed_reduce.md) | Accumulates a value per key and projects the table, its sum, and its cardinality. |
| [`request`](actors/request.md) | Projects each event into one configured HTTP request. |
| [`file`](actors/file.md) | Reads or replaces the complete contents of one real file. |
| [`json`](actors/json.md) | Holds an authored value and emits it at start and on demand. |
| [`otlp`](actors/otlp.md) | Receives OTLP/HTTP JSON logs and metrics on a loopback address. |
| [`match`](actors/match.md) | Splits an envelope's success and failure tags onto two outlets. |
| [`assemble`](actors/assemble.md) | Groups records by key and emits one object when the window closes. |
| [`join`](actors/join.md) | Joins each event with the latest reference state for its key. |
| [`form`](actors/form.md) | Commits a typed draft, filled in by a person, into the graph. See the page for the `fields` configuration limitation. |

## How to read a page

Every page has the same six sections.

**Ports** — the inlets and outlets, with their payload shape and whether they must be
wired. One inlet and one outlet may be marked *primary*: those are the ones a chained call
connects to when you do not name a port, so `a.tap().counter()` wires primary to primary.
Everything else you wire by name through `handle.in.<port>` and `handle.out.<port>`. Some
actors also have an `_error` outlet for explicit diagnostics. Actor input and policy
rejections are recorded as dead letters.

**Configuration** — the keys the actor accepts. Circular deliberately has **no silent
defaults** in most places: where a key is required and has no default, you must write a
value, and omitting it is a rejection rather than a guess. Where a default does exist it is
listed explicitly. Time intervals are integer milliseconds written as `bigint` literals
(`5000n`). Payload paths are arrays of segments: string elements select object keys,
non-negative integer elements select array indices, and an empty array selects the whole
payload.

**State** — what the actor remembers across arrivals. "Stateless." means nothing carries
over from one arrival to the next.

**Rejections** — the named reasons the actor refuses something. `ConfigRejected` and the
other activation refusals happen at admission: the actor does not stand up at all. The
runtime reasons refuse one arrival or one outcome while the actor keeps running.

**Example** — a minimal authored snippet using the published constructor spelling. A name
the snippet uses but does not declare, such as `events` or `upstream`, stands for an
actor you have already declared, for example
`export let events = input({ label: "events" }); events.mount("events", "request");`.
Every actor is a top-level named export; an actor held in an unexported `const` is refused
with `authoring.prepass.unbound-actor`.

### Two spellings for the same actor

Most constructors can be called two ways:

```ts
import { counter } from "@circular/core";

// As a standalone constructor, optionally naming the sources for its inlets.
export const a = counter({ event: someSource });

// As a chain method on an upstream endpoint, which wires the primary ports for you.
export const b = someSource.counter();
```

`alert` takes its predicate as a separate first argument. `match` takes a source directly
rather than a configuration object. `tap` and `counter` take no configuration at all.
`input`, `output`, `form` and `pipeline_actor` are boundary and container declarations and
have no authored ports of their own.

### Capabilities and approval

The actors in the table below require a `capabilities` grant for each listed effect.
For `request`, `notify`, `file` and `listener`, `actor.create-inputs` publishes
`capabilities` as a required slot, and leaving it out refuses the declaration with
`config.capabilities = <missing>`. For `tool_executor` the slot is optional, because its
tools decide which grants it needs. Omitting a listed grant inside `capabilities` refuses the
declaration with a diagnostic that names the path, for example
`config.capabilities.HttpFetch.approval = <missing>`.
Other external actors, including `agent`, `peer` and `otlp`, use their own bindings and
configuration.

| Actor | Required `capabilities` |
| --- | --- |
| `request` | `{ HttpFetch: { approval } }` |
| `notify` | `{ UserNotify: { approval } }` |
| `file` | `{ FsRead: { approval, roots }, FsWrite: { approval, roots } }` |
| `listener` | `{ FsRead: { approval, roots } }` |
| `tool_executor` | `FsRead: { approval, roots }` if a tool uses `file_read`, `FsWrite: { approval, roots }` if one uses `file_write`, `ProcessSpawn: { approval }` if one uses `spawn`; nothing when `tools` is empty |

Every `approval` is exactly `"none"` or `"required"`, and every `roots` is an array of
absolute directories. An effect of `request`, `notify`, `file` or `listener` waits for approval
when its grant's `approval` is `"required"`; these actors have no top-level `approval` key, and a
declaration that writes one is refused. A `tool_executor` effect waits when either the tool's
own `approval` or the grant's is `"required"`. `agent`, which takes no grant, has an optional
top-level `approval` with the same two values; omitting it asks for no approval.

### Wire preprocessing

`map`, `filter`, `bang`, `parse` and `flatten` are not actors. They are steps on the
receiving inlet of a wire, and [combinators.md](combinators.md) describes each one.
