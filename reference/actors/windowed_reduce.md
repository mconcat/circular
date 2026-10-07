# windowed_reduce

Keeps the recent numeric samples and, on a fixed period, folds the ones inside the current
window with an expression you author. Use it for rolling aggregates: a rate, a sum, a
maximum, an order statistic — the aggregation lives in the expression, not in a per-shape
variant of the actor.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `sample` **(primary)** | A floating-point number — the payload is the number itself | Yes |
| Outlet | `aggregate` **(primary)** | Whatever the `reduce` expression produces; it is not fixed to a number | — |

Admission requires `reduce`, whose presence generates the `aggregate` outlet. There is
no `_error` outlet.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `window_length` | Integer milliseconds (`bigint`) | **Yes — no default** | The width of the window. With `window_length: 0n`, the window contains no samples. |
| `emission_period` | Integer milliseconds (`bigint`), greater than zero | **Yes — no default** | How often the window is folded. Zero has no meaning here and is refused. |
| `reduce` | String expression | **Yes — no default** | The fold step, evaluated with two bindings: acc, the value folded so far, and sample, the incoming value. |
| `seed` | Any value | **Yes — no default** | The starting value of the fold. There is deliberately **no silent default of 0** — since the output is not fixed to a number, the seed takes the whole value space and you must write it. |

There is no configuration key for empty-window behaviour, because an empty window has
exactly one disposition: nothing is emitted.

## State

The accepted samples in consumption order, each with the recorded time of its own arrival,
and a ledger of the largest timer arming issued so far.

There is no phase field: the end of the window is the recorded time of the wake-up that
closes it. The actor tracks one active timer arming.

**Window membership is half-open, `[end - window_length, end)`.** A sample at the exact end
belongs to the next window, and a sample at the exact start belongs to this one, so a sample
on a boundary lands in exactly one window.

**Arming is driven by the samples.** A sample arriving at an unarmed actor arms the period.
After a fold, if no samples remain, nothing is rearmed — an actor with no samples would
produce nothing anyway. A late wake-up simply means its late time is that window's end;
missed periods are not caught up.

Restart reconstructs the samples and pending schedules by replaying this actor's recorded
arrival column. The replay starts after the actor's checkpoint cache when that cache matches
the recorded arrivals, and from the beginning otherwise. A new sample is not needed to
resume a pending schedule.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | Any of `window_length`, `reduce`, `emission_period` or `seed` missing or malformed; `emission_period` is zero; `reduce` does not parse, uses an undeclared name, or does not produce a ground result type; the kind of `seed` and the `Float` kind of `sample` split an operation in `reduce` (for example `seed: 0` with `acc + sample` — write `0.0`). The kind split is refused before the change commits, at `config.reduce`. |
| `InputOutOfDomain` | A `sample` payload outside the numeric domain, or an arrival with no recorded time. The observable consequence is closed — no sample is recorded, no data is emitted, no state changes — but the classification of the failure itself is **not specified in the published semantics**. |
| `stale_timer_fire` (suppression) | A wake-up from a superseded arming arrived. Nothing is emitted and nothing changes. |
| `reduce_failed` (dead letter) | The reduce expression produced no value while folding a window. That window emits no aggregate. The sample the expression failed on is recorded as a dead letter with this declared reason. This is a failure of one window, not of the actor: its health does not change and it keeps taking samples. |

An empty window is not a failure — emitting nothing is that window's correct result.

## Example

```ts
import { windowedReduce } from "@circular/core";

export const rollingSum = samples.windowedReduce({
  window_length: 60000n,
  emission_period: 5000n,
  reduce: "acc + sample",
  seed: 0.0,
});
```
