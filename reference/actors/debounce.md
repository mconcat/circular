# debounce

Holds the most recent event and emits it only after the inlet has been quiet for a
configured window. Each new arrival replaces what was pending and restarts the window, so a
burst of events produces one emission carrying the last payload of the burst.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Any value; the type passes through unchanged | Yes |
| Outlet | `event` **(primary)** | The pending payload, unchanged | — |

Inlet and outlet share the name `event`; directions have separate name spaces. The inlet
and outlet carry the same type, so `debounce` does not change what flows through it. There is
no `_error` outlet.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `quiet_window` | Integer milliseconds (`bigint`) | **Yes — no default** | How long the inlet must stay quiet before the pending payload is emitted. Zero is inside the value space. |

## State

Either empty, or one pending payload together with the moment it is due. A new arrival
replaces the pending payload, its cause, and its due moment all at once; at most one
event is pending.

The quiet window is measured from recorded arrivals, not from a wall clock. The actor does
not read the current time.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | `quiet_window` is missing, or is not an integer millisecond interval. |

No runtime rejection. Replacing a pending value with a newer one is a normal transition,
not a suppression or a failure.

## Example

```ts
import { debounce } from "@circular/core";

export const settled = debounce({ quiet_window: 500n });
```
