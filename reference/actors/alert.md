# alert

Samples a predicate once per arriving event and holds that result until the next event.
Every event passes through unchanged on the `event` outlet. Separately, when the held
result has persisted long enough to move the alert between its three states, one transition
is emitted on the `transition` outlet.

This is a sample-and-hold, not a poll: `alert` has no polling interval of its own. How
often the predicate is sampled is decided by whatever is wired upstream.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Any value; the type passes through unchanged | Yes |
| Outlet | `event` **(primary)** | The input payload, unchanged, whether or not the predicate held | — |
| Outlet | `transition` | Closed object `{ from: String, to: String }` | Wire by name |

The transition payload carries only the two state tags. The time of the transition is not a
payload field; it is carried by the recorded arrival. There is no `_error` outlet.

When one arrival produces both a pass-through and a transition, the pass-through emission
comes first.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `predicate` | String expression (or a source arrow) over the `event` payload | **Yes — no default** | Evaluated once per arrival. A boolean true is read as a violation. |
| `firing_delay` | Integer milliseconds (`bigint`), greater than zero | **Yes — no default** | How long a violation must persist before the alert moves to firing. |
| `recovery_delay` | Integer milliseconds (`bigint`), greater than zero | **Yes — no default** | How long a recovery must persist before the alert returns to ok. |

The polarity is fixed: `true` means violation. There is no configuration key that inverts
it. An evaluation error or a non-Bool result is not a sample: the arrival still passes
through on `event`, the state does not move, and a dead letter with the actor-declared
reason `predicate_failed` records it.

## State

Three states, starting at `Ok`:

- **`Ok`** — no violation is firing. It may or may not have a firing delay armed.
- **`Firing`** — the violation persisted through the firing delay.
- **`Cooldown`** — a recovery was seen while firing, and the recovery delay is armed.

The actor also holds whether a delay is currently armed and which arming it is waiting for.
The predicate result is held between arrivals: several timer wake-ups between two events do
not re-evaluate it.

The concrete behaviour:

| Situation | What happens | `transition` |
| --- | --- | --- |
| First `true` sample while ok and unarmed | Arms a wake-up after `firing_delay` | none |
| Further `true` samples while armed | Keeps the existing arming; the delay does not restart | none |
| A `false` sample while armed | Clears the candidate and the arming; the late wake-up is then suppressed | none |
| The armed wake-up arrives | Moves to firing | `Ok` → `Firing` |
| A `false` sample while firing | Moves to cooldown and arms a wake-up after `recovery_delay` | `Firing` → `Cooldown` |
| A `true` sample while in cooldown | Clears the recovery arming and returns to firing immediately | `Cooldown` → `Firing` |
| The recovery wake-up arrives | Returns to ok | `Cooldown` → `Ok` |

Counting samples is not part of the rule: after the first `true`, the alert fires when the
armed wake-up arrives and no `false` sample came first. Stale wake-ups from a superseded
arming are suppressed and change nothing. A fresh incarnation starts at `Ok` with nothing
armed. Checkpoint hooks carry the phase and arming correlation during configuration edits.
Pending schedules also carry over during these edits. Restart reconstructs the state and
pending schedules by replaying this actor's recorded arrival column. The replay starts after
the actor's checkpoint cache when that cache matches the recorded arrivals, and from the
beginning otherwise.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | `predicate`, `firing_delay` or `recovery_delay` is missing or malformed; the predicate does not parse; the predicate does not produce a boolean; the shape wired into `event` makes an operation in the predicate compare two different kinds (for example `event > 1.0` fed by a `counter`'s integer `count`). The kind split is refused before a change that adds or changes the `alert` commits, at `config.predicate`. |
| `predicate_failed` (dead letter) | The predicate yielded no value for this event. The event still passes through on `event` and the state and arming do not move. The event is recorded as a dead letter with this declared reason. This is a failure of one event, not of the actor: its health does not change and it keeps taking events. |

## Example

```ts
import { alert } from "@circular/core";

export const latency = alert("event.latency_ms > 1000.0", {
  firing_delay: 30000n,
  recovery_delay: 60000n,
});

export const changes = latency.out.transition;
```

The predicate compares numbers of the same kind only. `1000.0` matches a floating-point
`latency_ms`; for an integer field write `event.latency_ms > 1000`. A sample whose number
kinds differ is not a violation: it is recorded as a `predicate_failed` dead letter, and the
alert does not arm.

A threshold on a Prometheus HTTP API answer reads the sample text as a number:

```ts
export const errorRate = alert(
  "event.data.result.exists(r, r.value[1] != 'NaN' && double(r.value[1]) > 0.05)",
  { firing_delay: 30000n, recovery_delay: 60000n },
);
```

The `!= 'NaN'` test is there because a `0/0` ratio arrives as the text `"NaN"`, and an
ordering comparison with `NaN` is an evaluation error. Without the test, such a sample is a
`predicate_failed` dead letter. See "Functions inside an expression" in
[`combinators.md`](../combinators.md).
