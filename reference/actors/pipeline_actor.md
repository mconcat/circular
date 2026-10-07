# pipeline_actor

> **Alpha:** Use `pipelineActor({ source, in, out })` with a child module for nested
> pipelines whose children appear on the canvas and can be edited there. The function
> `template` form's generated children do not appear on the canvas in this release.

Opens a child scope and stands as one actor in the parent graph. Its ports are not declared
directly: they are folded from the child's own `input` and `output` declarations, so a
child that declares two inputs and one output becomes a container with two inlets and one
outlet. Nesting is namespacing, not a second runtime — the top level and a nested pipeline
follow the same interface rules.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | derived, one per child `input` | Same type and arity as the child `input` declares | Derived from the child |
| Outlet | derived, one per child `output` | Same type and arity as the child `output` declares | Derived from the child |

The container registers no fixed ports of its own, and configuration does not expand ports.
If a direction ends up with exactly one port, that port is the primary one; with two or
more, that direction has no primary and every edge names its port. There is no `_error`
outlet.

Port identity comes from the child boundary declaration, not from its label. Renaming a
child `input` or `output` keeps the parent wiring intact. Deleting one and creating a new
one with the same label produces a new port, and old edges are not re-bound to it.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `source` | String | One of `source` or `template` | Bundle path to a child module, such as `scopes/worker.ts`. |
| `template` | Named function reference | One of `source` or `template` | The stored child program this container mints at activation. Its generated children are absent from the authoring snapshot and canvas. |
| `in` | Array of topic names | **Yes — no default** | Entry topics, matched exactly against the child's `input` labels. |
| `out` | Array of topic names | **Yes — no default** | Exit topics, matched exactly against the child's `output` labels. |

Supply either `source` or `template`, together with `in` and `out`. The authoring client
lowers the module form into a container with null configuration, an `UpsertScope`, and
the child's declarations. The function form stores a template and binds the container to
it. A template name string is not a function reference in SDK source.

`in` and `out` declare the topic correspondence only. They are not a second source for the
boundary types — the types come from the child `input` and `output` declarations.

## State

Stateless. The template value, the derived child prototype, and the running child instances
are not fields of this actor.

## Rejections

| Code / name | When |
| --- | --- |
| Authoring diagnostic / `ConfigRejected` | The configuration does not satisfy the schema, or `in` / `out` do not match the child boundary topics. |
| Unresolved reference (admission) | An edge or export still points at a child boundary actor that was deleted and recreated. The new actor is not bound automatically just because its label matches. |

No runtime rejection: boundary relay does not produce actor processing failures.

## Example

Save both files in the session directory that `circular edit` created, with
`scopes/worker.ts` beside `main.ts`, and approve the entry file:
`circular edit --state "$STATE" --session <session directory> --program main.ts --approve`.
`source` is a path from the directory that holds `main.ts`. Each actor, including child boundary actors
and the intermediate `tap`, has its own `export const` or `export let` binding.

`main.ts`:

```ts
import { input, pipelineActor } from "@circular/core";

export const source = input({ label: "values" });
export const worker = pipelineActor({
  source: "scopes/worker.ts",
  in: ["request"],
  out: ["result"],
});
source.into(worker.in.request);
export const seen = worker.out.result.tap();
source.mount("values", "request");
seen.mount("results", "result");
```

`scopes/worker.ts`:

```ts
import { projectInput } from "@circular/core";

export const request = projectInput({ topic: "request" });
export const doubled = request.map("event * 2").tap();
export const result = doubled.projectOutput({ topic: "result" });
```

The `map` is preprocessing on `doubled`'s inlet. A value of `7` injected at `values`
is relayed through the child and reaches the parent observation mount `results` as `14`.
The child input and output topics match the parent's `in` and `out` entries.
An `input()` or `output()` written in a child module is a port of the container too. The parent's
`in` and `out` do not name it, and the reconstructed program prints it as `projectInput` or
`projectOutput` with its label in `in` or `out`.

Keep actor allocations separate: `request.tap().projectOutput(...)` hides the `tap`
without its own binding and is refused with `authoring.prepass.unbound-actor`.
`circular chat` reconstructs this module form as `current/main.ts` and `current/scopes/worker.ts`.
