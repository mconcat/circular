# json

A value box. It begins with an authored value and emits its current value on the startup
pulse and on later pulses. Writing a new value and triggering an emission are deliberately
two different inlets.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `bang` **(primary)** | Any value; the content is not read — it is a pulse | No — optional |
| Inlet | `set` | Any value: becomes the new current value | Yes |
| Outlet | `value` **(primary)** | The current value | — |

There is no `_error` outlet, and configuration does not add or remove ports.

- **`set`** replaces the current value. It emits nothing.
- **`bang`** ignores its own payload, emits the current value once, and marks the actor started.
- **Start** supplies a `bang`, which emits the current value.

Because the two jobs live on different inlets, there is no question of whether an update
emits before or after it lands: `set` makes the new value the current one, and the next
`bang` reads it.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `initial` | Any value | **Yes — no default** | The authored starting value. |

## State

The current value — the authored initial value, or the payload of the most recent `set` —
together with whether the start emission has already happened.

A `bang` emits the current value, including a value set before that bang, and marks the
actor started. It does not change the current value.

A fresh incarnation starts with `initial`. Restart reconstructs the current value and start
marker by replaying this actor's recorded arrival column. The replay starts after the
actor's checkpoint cache when that cache matches the recorded arrivals, and from the
beginning otherwise. The checkpoint hook encodes these logical values. A configuration edit
sets the current value to the new `initial`.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | `initial` is missing. Its value space is any value, so nothing is refused on shape. |

No runtime rejection. This actor does not coerce or evaluate payloads, so there is no
domain refusal on `set` or `bang`.

## Example

```ts
import { json } from "@circular/core";

export const settings = json({ initial: { threshold: 0.9, enabled: true } });

export const current = settings.out.value;
updates.into(settings.in.set);
```
