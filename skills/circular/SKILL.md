---
name: circular
description: Build or change a Circular actor pipeline on the user’s machine through a Circular authoring session.
---

# Circular

Use this when the user asks you to build, change, observe or explain a Circular pipeline.
A pipeline is a set of actor tasks hosted by the Circular daemon on the user’s machine.
You change it with a short TypeScript program; the user watches and steers the same pipeline
on the canvas.

## 1. Know the state directory

Every Circular command that names a state takes `--state <absolute directory>`, and nothing
guesses it. Ask the user which state directory to use if you were not told. Check it before
anything else:

```sh
circular daemon status --state "$STATE" --json
```

If no daemon is running and the user wants one, `circular daemon start --state "$STATE"`.
`circular doctor --state "$STATE"` explains what is wrong and changes nothing.

## 2. Open an authoring session

```sh
circular chat --state "$STATE" --claude --no-open   # or --codex / --pi: the CLI you are
```

It prints the path of a `launch.command` file. Its directory, under `$STATE/chat/`, is the
session. Work there, and read in this order before you change anything:

1. `first-message.md` — what stood when the session opened, the daemon, and the files to read
   next.
2. `AGENTS.md` — how a pipeline is authored, deployed, observed and paused.
3. The instruction file `first-message.md` names — the SDK spellings for this daemon. It
   also carries the deploy script for this session; use it rather than writing your own.
4. `current.ts` (or `current/main.ts` with its scope modules) — the pipeline that stood when
   the session opened, generated as SDK code from the daemon. It is read-only. Programs you
   deploy read `circular:current` from the daemon again when they run. A missing daemon socket
   produces an empty `main.ts` skeleton, not a recovered graph. A connected genesis state
   produces an empty `current.ts` from the daemon's accepted snapshot.

## 3. Write only the change

To change what stands, write a program that says only the change, and import what stands
from `circular:current`:

```ts
import { counter } from "circular:current";
export const seen = counter.out.count.tap();
```

A program that says nothing about an actor leaves that actor as it is. For a new pipeline,
write the whole program. SDK constructors and methods are camelCase (`toolExecutor`,
`keyedReduce`). `actor.catalog` publishes actor metadata and may mark configuration frames
incomplete. Use `actor.create-inputs` for the complete configuration schema for creating
an actor, including explicit no-configuration and unavailable cases. The SDK exposes
these queries as `actorCatalog(session)` and `actorCreateInputs(session)` from
`@circular/client`; a connected `circular chat` session includes both in `actor-catalog.md`.

When a deploy is refused, report the diagnostic that came back and fix what it names. Do not
retry it in another form until you know why it was refused.

## 4. The model, in five lines

1. An active actor runs as a task; a pipeline has no built-in completion.
2. Branching is fan-out with a `filter` at each destination; there is no if-actor.
3. `map`, `filter`, `bang`, `parse` and `flatten` belong to the receiving inlet; they are not
   actors. `map` and `filter` take CEL strings or supported single-expression arrow functions.
   In a CEL string, the arriving value is named `event`.
4. Cycles are ordinary wires.
5. The daemon's journal is what the pipeline is. A file you wrote is not a deployment.

A commit records declarations; arrivals show recorded input, and `daemon.health` reports
recorded actor health. Read a bounded page with `arrival.scan`, naming a registered mount,
before you open any subscription.

## 5. Limits

- Do not read, copy or print credentials, and do not set environment variables to select
  behavior.
- Write only inside the state directory. `node_modules` and linked `@circular/*` packages
  are read-only.
- Do not deploy, pause or edit the pipeline unless the user asked for it.
- An `agent` actor runs the program the user bound to its harness name
  (`circular harness list --state "$STATE"` reads the bindings); use a reported name as it is.
  Bind or unbind (`circular harness bind|unbind`) only when the user asks, with the name and
  absolute program path the user gives.
