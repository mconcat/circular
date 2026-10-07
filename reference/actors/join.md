# join

Remembers the latest reference value per key, and when a trigger event arrives it emits that
event paired with the reference for the same key. Unlike a wire combinator, the reference
table belongs to this actor.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Any value: the trigger | Yes |
| Inlet | `state` | Any value: the reference to remember for this key | Yes |
| Inlet | `remove` | Any value: drops the reference for this key | Yes |
| Outlet | `event` **(primary)** | Closed object `{ event, state }` | — |

Three fixed inlets and one fixed outlet, with no dynamic expansion. There is **no separate
error outlet**: a failure is carried as the envelope tag of an ordinary emission on `event`,
and the body of a failed emission is the original input unchanged.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `at` | Array of path segments | **Yes — no default** | The exact payload path that selects the key. It applies to **all three inlets**. |

String elements select object keys, non-negative integer elements select array indices, and
an empty array selects the whole payload. A path written as a single string, a negative
index, and elements of any other kind are refused.

The **selected value must be a String**. It is not converted from a number, and there is no
non-empty constraint added on top.

There are no slots for an idle lifetime, a capacity, or a default for a missing reference.

## State

A table from key to the latest reference payload, empty when the actor is created.

- `state` replaces the whole previous payload for that key. "Latest" means the order this
  actor consumed them in — a timestamp field does not re-order anything.
- `event` only reads the table.
- `remove` drops exactly that key.

Checkpoint hooks transfer the table during configuration edits. The restore hook checks that
each payload's selected key matches its stored key before replacing the table. It rejects a
mismatched schema or a corrupt table. Restart reconstructs the table by replaying this
actor's recorded arrival column. The replay starts after the actor's checkpoint cache when
that cache matches the recorded arrivals, and from the beginning otherwise.

The transitions:

| Situation | Table | Emission |
| --- | --- | --- |
| `state` with a String key | That key's payload is replaced | none |
| `event` with a String key that has a reference | unchanged | `{ event, state }`, tagged success |
| `event` with a String key that has no reference | unchanged | the original input, tagged failure |
| `state` or `event` where no String key can be selected | unchanged | the original input, tagged failure |
| `remove` with a String key | That key is dropped; if it was absent, nothing changes | none |
| `remove` where no key can be selected | unchanged | none |

A bad key on `remove` deliberately does **not** behave like a missing reference on `event`:
it emits nothing rather than a failure, and no reference value is invented for an unknown
key.

Editing the configuration replaces this actor's incarnation and does not stop any other
actor.

## Rejections

| Code / name | When |
| --- | --- |
| `JoinConfigError` | `at` is missing or violates the exact-path form. The actor is not created; this is not deferred into a per-input failure. |
| `InputOutOfDomain` | An `event` whose key has no reference, or a `state` or `event` from which no String key can be selected. The failure-tagged emission carries the original payload. |
| `SchemaBeyondLadder` | The stored checkpoint is from another schema. |
| `DecodeFailed` | The checkpoint is not an object, a payload is corrupt, or a stored key does not match its payload's selected key. The existing table is not partially replaced. |

A missing reference does not produce a successful pairing or a substituted default.

A failure-tagged emission passes through the rest of the wire without the downstream
preprocessing steps being evaluated, and an ordinary inlet receives only successes. Use
`match` if you want to branch on the tag.

## Example

```ts
import { join } from "@circular/core";

export const enriched = join({ at: ["user_id"] });

profiles.into(enriched.in.state);
actions.into(enriched.in.event);
departures.into(enriched.in.remove);
```
