# counter

Counts the events it accepts and emits the running count after each one. It counts arrivals
without inspecting their payloads.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Any value; the content is not read | Yes |
| Outlet | `count` **(primary)** | Integer: the number of events accepted so far | — |

There is no `_error` outlet.

## Configuration

None. `counter` takes no configuration. Its configuration value is `null`, and that is what
the published constructor sends. Any other value, including the empty object `{}`, is
refused at admission (see Rejections).

## State

One number: the running total of events this actor has accepted. A new actor starts at 0.
Restart reconstructs the total by replaying this actor's recorded arrival column. The replay
starts after the actor's checkpoint cache when that cache matches the recorded arrivals, and
from the beginning otherwise. Each accepted event moves the count to the next value. The
checkpoint hook reads the actor's stored count.

## Rejections

`ConfigRejected` (code 1) at admission when the configuration is anything other than `null`:
the empty object `{}`, an object with keys, or any other value. The value is not normalised to
`null`, and the refusal points at no key because the schema has none.

At runtime there is no domain rejection: the actor counts arrivals without reading the payload.

## Example

```ts
import { counter } from "@circular/core";

export const errors = events.filter("(event.type == \"error\")").counter();
export const total = errors.out.count;
```
