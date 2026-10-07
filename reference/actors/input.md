# input

Declares an inlet on the pipeline that contains it. `input` is not a source that produces
values — it is a boundary declaration. Inside its own scope it looks like one wirable
outlet; on the parent container that wraps that scope, the same declaration appears as one
inlet.

External injection can target an exported input boundary, including a `form` boundary.
An export mount’s request role resolves to that boundary port, preserving the payload.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Outlet (inside the scope) | derived boundary port | A stream of the base type `shape` names; Any when there is no `shape` | — |

The actor registers no authored fixed ports. One boundary port is derived from the
declaration: its wiring face inside the child scope is an outlet, and the matching face on
the parent container is an inlet. There is no `_error` outlet.

Port identity does not come from the label. Renaming preserves the port and the existing
wiring. Deleting the actor and creating a new one with the same label produces a new port,
and old edges and export references are not re-bound to it.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `label` | String | **Yes — no default** | The display name of this boundary. |
| `shape` | In the SDK, one base type name (`"float"`); on the wire, the canonical type expression of a stream of that base type | No — without it the boundary is Any | The type of the values this boundary injects. |

`shape` takes one `BaseShape` spelling from `@circular/protocol`: `null`, `bool`, `int`,
`float`, `string`, `bytes` or `uint`. The SDK lowers `shape: "float"` to the canonical type
expression `[1n, [2n, "float"]]`, the value the canvas writes too. Generated source
(`current.ts`) prints it as `shape: "float"`. In a child module, `projectInput({ topic, shape })`
takes the same key.

An input with `shape` is checked as that type on every connection, so a `string` input wired
to `ema` is refused. An input without `shape` is Any. Its own outgoing wire to a typed inlet is
accepted without that check. A wire further downstream is checked, so `input → ema` is accepted
and `input → tap → ema` is refused.

On the canvas, when you drag a connection from a port whose type is a stream of one base type
and create an input from it, the create form starts `shape` at that type. You can change it
before Create.

## State

`input` runs as a pass-through boundary actor with no checkpointed state.

## Rejections

| Code / name | When |
| --- | --- |
| `authoring.prepass.config-not-literal` | The SDK receives a `shape` that is not a published `BaseShape` spelling. |
| `ConfigRejected` | The configuration does not satisfy the schema, including a `shape` that is not the canonical type expression of a stream of one base type. |
| Unresolved reference (admission or injection) | An edge or export reference still points at a boundary actor that was deleted. Nothing is re-bound to a new actor just because the label matches. |

The boundary hook relays the arrival and has no error port. Backpressure is handled at
the receiving inlet.

## Example

```ts
import { input } from "@circular/core";

export const incoming = input({ label: "Transcript envelope" });
incoming.mount("transcripts");
```

```ts
import { input } from "@circular/core";

export const samples = input({ label: "Samples", shape: "float" });
export const smoothed = samples.ema({ half_life: 4n });
samples.mount("samples");
```
