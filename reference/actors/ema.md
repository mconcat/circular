# ema

Keeps an exponential moving average over a stream of numbers. Each accepted sample updates
the average and emits the new value together with how many samples have gone into it. The
decay distance between two samples is either one sample (the default) or the milliseconds
between their recorded arrivals.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `sample` **(primary)** | A finite floating-point number | Yes |
| Outlet | `ema` **(primary)** | Closed object `{ value: Float, samples: Int }` | — |

`samples` is the ordinal of the accepted sample, not a timestamp; the first emission carries
`1`. Configuration does not change the port set.

`ema` has no `_error` outlet in its published port set, so nothing can be wired to one. For
out-of-domain input the actor emits a diagnostic string on `_error`; that emission has no
wire and reaches nothing, and it is recorded as the actor's own emission (`actor.events` row
`actor_emission`, port `_error`).

Note that an `Int` output such as `counter`'s `count` does not connect directly to this
`Float` inlet; this actor adds no numeric conversion of its own.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `half_life` | Integer (`bigint`), greater than zero | **Yes — no default** | The half-life of the decay. With `time_basis: "samples"` the unit is a **count of samples**; with `"wallclock"` it is **milliseconds**. |
| `time_basis` | `"samples"` \| `"wallclock"` | No — default `"samples"` | Which distance the decay is measured in. |

Values outside the two closed tags, a non-string `time_basis`, and extra keys are refused.

## State

Either empty (no previous value yet), or ready: the last average, the number of accepted
samples, and — when the arrival carried one — the recorded arrival time of the previous
accepted sample.

The first accepted sample initialises the average to that sample with a count of 1.
Afterwards the update is `y = x + (y_prev - x) * 2^(-d/h)`, where `d` is the distance:

| Basis and situation | Distance `d` |
| --- | --- |
| `samples` | 1 |
| `wallclock`, both the current and previous arrival times are known | `max(0, current_ms - previous_ms)` |
| `wallclock`, the current arrival time is known but the previous one is not | 0 — no decay |
| `wallclock`, the arrival carries no recorded time | 1 — it behaves as `samples` |

A distance of zero keeps the previous average exactly. Equal or out-of-order arrival times
give a distance of zero; nothing is re-sorted, and the actor does not read a wall clock
itself. The count increases by one per accepted sample.

Restoring a state saved under `samples` into `wallclock` starts with no previous time, so
the first timed sample does not decay; the value and the count are kept.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | The configuration is not an object; `half_life` is missing or outside its domain; `time_basis` is not a string or not one of the two tags; an extra key is present. |
| `InputOutOfDomain` | An unknown inlet, a non-float payload, or a non-finite sample. The state, the previous time and the count are all kept. The diagnostic string emitted on `_error` has no wire; it is recorded as the actor's own emission. |
| `SchemaBeyondLadder` | The stored state is from an older schema. |
| `DecodeFailed` | The stored state has the wrong tag, length or encoding. |
| `StateInvariantViolated` | The stored value is non-finite, or the sample count is zero. |

A failed restore leaves no partial state.

## Example

```ts
import { ema } from "@circular/core";

export const smoothed = latencies.ema({
  half_life: 30000n,
  time_basis: "wallclock",
});
```
