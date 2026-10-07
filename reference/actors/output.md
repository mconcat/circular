# output

> **Alpha:** Use `projectOutput({ topic })` inside a child module opened by
> `pipelineActor({ source, in, out })`. The [two-file example](pipeline_actor.md#example)
> shows the parent and child together. A top-level `output` is refused at activation
> (`boundary activation rejected`); use a result mount to observe a top-level actor.

Declares an outlet on the pipeline that contains it. Inside its own scope it looks like one
wirable inlet; on the parent container that wraps that scope, the same declaration appears
as one outlet. Events that settle on the inside inlet are relayed to the matching outlet on
the parent.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet (inside the scope) | derived boundary port | Any value | — |

The actor registers no authored fixed ports. One boundary port is derived from the
declaration: its wiring face inside the child scope is an inlet, and the matching face on
the parent container is an outlet. There is no `_error` outlet.

Port identity does not come from the label. Renaming preserves the port. Deleting the actor
and creating a new one with the same label produces a new identity.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `label` | String | **Yes — no default** | The display name of this boundary. |

The published registration carries the `label` slot only. The configuration path that would
carry an explicit boundary type is not specified in the published semantics.

## State

`output` runs as a boundary actor with no retained state. Its hook relays arrivals
across the scope boundary.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | The configuration does not satisfy the schema. |

The boundary hook relays arrivals without inspecting their payloads.

## Example

In `scopes/worker.ts`, with the parent in the [module example](pipeline_actor.md#example):

```ts
import { projectInput } from "@circular/core";

export const request = projectInput({ topic: "request" });
export const doubled = request.map("event * 2").tap();
export const result = doubled.projectOutput({ topic: "result" });
```

The `topic` becomes the child boundary's `label` and matches the parent's `out: ["result"]`.
Each actor has its own exported binding. The function `template` form's generated children
do not appear on the canvas in this release.
