# replicator

Routes events into keyed child cells. Each distinct key mints one cell from the referenced
template; later events with the same key go to the cell that already exists. A cell retires
after its idle window, and a capacity limit bounds how many cells can be alive at once.

It is a keyed `pipeline_actor`: a container that mints its child many times instead of once.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Any value | Yes |
| Outlet | derived, one per template `output` | Same type and arity as the template `output` declares | Derived from the template |

The inlet is fixed; the outlets are folded from the referenced template's `output`
declarations, exactly as they are for a `pipeline_actor`. Configuration does not expand
ports, and the set of live cells does not either — cells do not each get their own port.
There is no `_error` outlet.

The referenced template must declare **exactly one** boundary inlet, because `event` is the
only inlet and there is no port with which to choose a destination inside the cell. A
template with zero or more than one `input` is refused at admission.

When a cell emits on one of its `output` boundaries, the value leaves through the matching
container outlet. The cell key travels in the emission envelope, not in the payload — the
payload is unchanged, and downstream actors that need the key read the envelope. There is no
global ordering between cells; within one cell, order is the arrival order on that cell's
`output` inlet.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `template` | Template function reference | **Yes — no default** | The cell program minted per key. Pass the function, not a string name. |
| `in` | Array of topic names | **Yes — no default** | Entry topics, matched against the template's `input` labels. |
| `out` | Array of topic names | **Yes — no default** | Exit topics, matched against the template's `output` labels. |
| `at` | Array of path segments | **Yes — no default** | The exact payload path whose value becomes the cell key. An empty array selects the whole payload. |
| `ttl` | Integer milliseconds (`bigint`), greater than zero | **Yes — no default** | A cell that sees nothing for this long retires. There is no "never expires" value. |
| `capacity` | Integer (`bigint`), at least 1 | **Yes — no default** | The maximum number of live cells. |

`ttl = 0` and `capacity = 0` are outside the value space. There is deliberately no default
idle lifetime: that is a property of your pipeline, not something the catalog invents.

Editing `ttl` or `capacity` is absorbed by the running actor — a new `ttl` applies from the
next rearm, a new `capacity` from the next minting decision. Editing `at` replaces the
incarnation, because the live keys would no longer be keys of the same thing.

## State

Per-key idle-timer bookkeeping. Activity from recorded arrivals determines whether a timer
retires the cell or waits for the rest of its idle window. At capacity, the oldest minted
cell retires first, with ties broken by canonical key order.

The set of live cells is **not** actor state — it is reconstructed from the recorded mint
and retire history. The cell registry tracks one live cell per key and generation. When a
key retires and later returns, the new cell has the same address and a later generation;
the old accumulation does not come back.

Editing the template body does not patch existing cells. New cells start from the new
prototype; existing cells keep the body they were minted with until retirement.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | `at`, `ttl` or `capacity` missing or outside its value space, including `ttl = 0` and `capacity = 0`. |
| Boundary disagreement (admission) | The referenced template does not declare exactly one boundary inlet. |
| `InputOutOfDomain` | The key expression yields `Undefined`, `Null`, an array or an object. |
| `InputOutOfDomain` | The key is a number that is `NaN`, infinite, non-integer, or outside the safe integer range. |

There is no error outlet, so these runtime rejections go to the failure lane. Idle expiry
and capacity retirement are normal lifecycle transitions, not failures.

## Example

```ts
import { replicator, projectInput, projectOutput } from "@circular/core";

export function worker() {
  const incoming = projectInput({ topic: "request" });
  const outgoing = projectOutput({ topic: "result" });
  incoming.into(outgoing);
}

export const workers = replicator({
  template: worker,
  in: ["request"],
  out: ["result"],
  at: ["sessionId"],
  ttl: 60000n,
  capacity: 16n,
});
```
