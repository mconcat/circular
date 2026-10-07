# assemble

Groups arriving records by a key and emits one object per group when that group's window
closes. A group closes either because nothing has arrived for it recently, or because it hit
its maximum age — and the emitted object says which.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Open object: one record carrying the key | Yes |
| Outlet | `event` **(primary)** | Closed object `{ key: String, events: Array<object>, stuck: Bool }`, one per closed window | — |
| Outlet | `_error` | The failure | Wire by name |

There are no dynamic ports: a key does not get its own outlet.

`events` holds the records in the order they were consumed. `stuck` is `true` when the
window was closed by exceeding `max_window` rather than by going idle. It is a field of a
normal result, not a failure marker.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `at` | Array of path segments | **Yes — no default** | The exact payload path that selects the key. The selected value must be a non-empty string. |
| `inactivity_timeout` | Integer milliseconds (`bigint`), greater than zero | **Yes — no default** | How long a group may go without a record before it closes. |
| `max_window` | Integer milliseconds (`bigint`), greater than zero | **Yes — no default** | The maximum age of a group, measured from when it opened. |
| `capacity` | Integer (`bigint`), at least 1 | **Yes — no default** | How many groups may be open at once. You must write this; there is no default. |

`inactivity_timeout` must be less than or equal to `max_window`.

`capacity` bounds **simultaneously open keys**, which is a different thing from the number
of records inside one window. The bound on records in a window, and the memory bound, are
not specified.

## State

Per open key: the records consumed into that group, when the group started, the recorded
time of the most recent arrival, and which timer arming it is waiting for.

A new record for an open key is appended and refreshes only that group's **inactivity**
deadline — the maximum-age deadline stays anchored to when the group opened.

Arrivals are replayed in their recorded order rather than re-sorted, and a timestamp in the
payload is not replaced with an execution-time clock.

The transitions:

| Situation | What happens |
| --- | --- |
| The first valid record for a key | The group opens and holds the record. Nothing is emitted yet. |
| A later record for the same key | Appended; the inactivity deadline is refreshed; the maximum age is not extended. |
| The inactivity wake-up for the current group | The group closes and emits one object, with `stuck` false. |
| The wake-up for a group past its maximum age | The group closes and emits one object, with `stuck` true. |
| A stale wake-up for a group already closed | Nothing; a closed window is not emitted twice. |
| A new key arrives while `capacity` is reached | The new arrival is settled as a dead letter with the reason `capacity`. No group opens and no existing group is evicted. |
| Configuration change or retirement | Open groups emit nothing and leave no extra record. Their inputs were already recorded as arrivals. |
| Restart | Daemon restart rebuilds each actor by replaying its recorded arrival column. The replay starts after the actor's checkpoint cache when that cache matches the recorded arrivals, and from the beginning otherwise. |

A group whose age exactly equals `max_window` has not exceeded it: that is an ordinary
close, and only a genuine overrun sets `stuck` to true.

The first implementation does not checkpoint. What happens to a late record for a key whose
group already closed — whether it opens a new group, and how a reused or duplicated key is
judged — is **not specified in the published semantics**.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | A required key missing or malformed; the interval relation violated; `capacity` not a positive integer. A configuration error is not deferred into a per-input failure. |
| `InputOutOfDomain` | The input is not an object, the key is absent, or the key value is outside its value space. No default key is invented. |
| Dead letter reason `capacity` | A new key arrived while the open-key count was at `capacity`. |

Exceeding the maximum window is **not** a failure: it is a normal result carrying
`stuck: true`.

Behaviour beyond these — other saturation cases and a restored state that does not match —
is not settled; neither silent discard nor unbounded retention is promised.

## Example

```ts
import { assemble } from "@circular/core";

export const traces = spans.assemble({
  at: ["trace_id"],
  inactivity_timeout: 3000n,
  max_window: 30000n,
  capacity: 1024n,
});
```
