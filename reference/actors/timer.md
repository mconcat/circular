# timer

Emits one sequenced tick every configured interval. A pulse on `bang` does not emit
immediately — it restarts the wait with a fresh generation, so a stream of bangs keeps
pushing the next tick further out.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `bang` **(primary)** | Any value; the content is not read | No — optional |
| Outlet | `tick` **(primary)** | Closed object `{ sequence: UInt }` | — |
| Outlet | `_error` | A diagnostic string | Wire by name |

The inlet is optional because a timer that is never banged still runs: the daemon supplies
the first pulse that arms it. Configuration does not add ports; there is no phase slot and
no payload-mapping slot.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `every` | Integer milliseconds (`bigint`), greater than zero | **Yes — no default** | The interval between ticks. |

Extra keys are refused.

## State

Two counters: a generation, incremented by each `bang`, and a sequence, incremented by each
tick. Both start at zero. Checkpoint hooks carry the counters during configuration edits.

A wake-up whose generation matches the current one emits one tick and rearms the same
generation for another full `every`. A wake-up from an older generation is a normal stale
fire: it is suppressed, and nothing is emitted or rearmed.

**A late wake-up fires once.** It does not produce one tick per missed interval, and the
next arming is the full interval from the moment it fired — so a delayed pipeline shifts
the period rather than catching up. The timer hook uses schedule effects and recorded
wake-ups to advance its sequence.

Restart reconstructs the counters and pending schedules by replaying this actor's recorded
arrival column. The replay starts after the actor's checkpoint cache when that cache matches
the recorded arrivals, and from the beginning otherwise. A new `bang` is not needed to
resume a pending schedule.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | The configuration is not an object; `every` is missing, non-integer, zero or negative; an extra key is present. |
| `timer received an unknown inlet` | An arrival on a port this actor does not have. State is unchanged. |
| `timer correlation payload is not UInt` | An internal wake-up whose correlation payload has the wrong type. State is unchanged. |
| `timer received an unexpected outcome: <kind>` | A successful outcome other than the arming acknowledgement. State is unchanged. |
| `timer scheduling failed: <kind>` | The arming itself failed. State is unchanged. |

The four runtime diagnostics leave on `_error`. A stale generation's wake-up is a normal
suppression and is not promoted to a failure.

## Example

```ts
import { timer } from "@circular/core";

export const everySecond = timer({ every: 1000n });
export const ticks = everySecond.out.tick;
```
