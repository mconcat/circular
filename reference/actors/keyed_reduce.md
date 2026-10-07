# keyed_reduce

Keeps a running total per key and publishes three views of that table: the whole
table, its sum, and how many keys are in it. A key retires through a separate inlet, and
when it does, its contribution disappears from all three views immediately.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Open object: carries the key and the number to add | Yes |
| Inlet | `remove` | Open object: carries the key to retire | Yes |
| Outlet | `map` **(primary)** | Open object: the whole table | — |
| Outlet | `total` | Float: the sum over live keys | Wire by name |
| Outlet | `count` | Int: how many keys are live | Wire by name |

There is no `_error` outlet.

`remove` is **required, not optional**. Watching a number fall when a key retires is half
the reason this actor exists; making retirement optional would make "a graph that never
retires anything" expressible, and such a graph's sum holds dead keys forever.

`count` is an integer while `map` and `total` are floating point. A cardinality is a system
value; an accumulation is your data.

**Which inlet an arrival came in on decides what it does.** A delta and a retirement carry
the same key on the same path, and telling them apart by payload would make a retirement
that looks like a delta expressible.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `at` | Array of path segments | **Yes — no default** | The exact payload path that selects the key. The **same path applies to both inlets** — a retirement carries the same key. |
| `value` | Array of path segments | **Yes — no default** | The exact payload path that selects the number to add. |

`at` uses the same value space as `route` and `replicator` use for the same idea.

Editing either path replaces the incarnation. If `at` changes, the live keys are no longer
keys of the same thing; if `value` changes, the accumulated numbers are no longer sums of
the same thing.

There is no configuration key selecting the fold: addition is the only one today, and a
one-armed enumeration would just be a ritual.

## State

A live table from key to accumulated number.

**The first delta for a key sits there as-is rather than being added to zero.** Adding to
an identity would fold `-0` into `0`, and those are different values here — a one-element
sum is that element.

**The sum is folded in canonical key order.** Floating-point addition is not associative, so
without a fixed order the same table could produce two different sums and `total` would stop
being a function of the state.

Checkpoint decoding refuses non-canonical bytes: keys out of ascending order, duplicate
keys, or a non-canonical NaN bit pattern. Numbers are carried as bits, not as decimal text,
so a `-0` or a low digit is not quietly lost.

The transitions:

| Situation | Effect on the table | What is emitted |
| --- | --- | --- |
| A valid delta for a key not in the table | It is inserted with the delta | `map`, `total`, and `count` together |
| A valid delta for a key already there | The delta is added | `map`, `total`, and `count` together, including an unchanged `count` |
| A `remove` whose key is in the table | The key is dropped | all three |
| A `remove` whose key is not there | Nothing | nothing |
| An arrival where the key or the number cannot be selected | Nothing | nothing |

Every valid delta emits `map`, `total`, and `count` together from the same table state,
even if a projection's value is unchanged. These emissions are ordered `map`, `total`,
`count`; removing an existing key emits the same three projections.

Retiring a key that is not there is not a failure. A key that retires and later returns
starts from its new delta — retirement removed the contribution, not a marker.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | `at` or `value` is missing or is not an exact path. |
| `DecodeFailed` | Stored state bytes are not this schema's shape. The actor starts with no state. |
| `StateInvariantViolated` | The shape is right but the value is out of domain: a non-canonical key sequence, or a non-canonical NaN. |
| `SchemaBeyondLadder` | The stored state is from a schema this build does not support. |

There is no error outlet. An arrival from which no key or no number can be selected simply
does not enter the aggregate — that is outside this actor's domain, not a failure.

A failed restore does not leave the state half-changed.

## Example

```ts
import { keyedReduce } from "@circular/core";

export const perSession = deltas.keyedReduce({
  at: ["sessionId"],
  value: ["total_tokens"],
});

export const fleetTotal = perSession.out.total;
export const liveSessions = perSession.out.count;
```
