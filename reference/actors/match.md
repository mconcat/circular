# match

Splits a stream on the **envelope** tag rather than on anything inside the payload. A
successful arrival's payload leaves on `ok`; a failed one's reason leaves on `err`.

This is the counterpart to `route`, which reads a place inside the payload and branches on
its value. A success or failure tag is not a payload field, so it is not something `route`
can select on and not something you should decorate the payload with.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Any value; accepted regardless of the envelope tag | Yes |
| Outlet | `ok` **(primary)** | The payload of a successful arrival, with its type unchanged | — |
| Outlet | `err` | The reason of a failed arrival | Wire by name |

One fixed inlet and two fixed outlets; configuration does not add ports.

A reason carries a code and a detail:

| `code` | `detail` |
| --- | --- |
| `processing` | The underlying failure |
| `outcome_unclaimed` | `null` |
| `destination_gone` | `null` |
| `poisoned` | `null` |
| `actor_declared` | A string |

## Configuration

None. `match` has an empty configuration schema: the branch criterion is the envelope tag,
and there is nothing to configure. Its configuration value is `null`, and that is what the
published constructor sends. Any other value, including the empty object `{}`, is refused at
admission (see Rejections).

## State

Stateless. No previous input, tag, reason or time is kept, and there is nothing to
checkpoint.

## Rejections

`ConfigRejected` (code 1) at admission when the configuration is anything other than `null`:
the empty object `{}`, an object with keys, or any other value. The value is not normalised to
`null`, and the refusal points at no key because the schema has none.

At runtime, none. Receiving a failed arrival is **not** a processing failure for this actor —
it emits one reason on `err` and continues. For each accepted arrival, the hook selects
`ok` or `err` from the envelope tag and emits on that outlet.

A preprocessing failure that happened upstream is not re-evaluated here. On replay the tag
is recomputed from the recorded arrival and the same branch is taken.

## Example

```ts
import { json, match } from "@circular/core";

export const raw = json({ initial: null });

export const split = match(raw.out.value);
export const good = split.ok;
export const bad = split.err;
```
