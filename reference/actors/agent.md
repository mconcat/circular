# agent

Sends a turn to an external agent harness and handles what that harness step returns: either
a final result or a tool request. The CLI harnesses in this release run their own tools
inside the CLI, and their steps return only a final result. When a harness step returns a
tool request, as the `reference` test harness does, the request leaves on `tool_request`, is
executed by a separate actor, and comes back on `tool_result`; only then does the next
harness step start.

One `agent` has at most one harness step or one tool call in flight at a time. Wired turns
wait at the destination inlet while it is busy.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `turn` **(primary)** | A text prompt: `String` or `Bytes` | Yes |
| Inlet | `tool_result` | Open object: the result of a tool call this agent requested | Yes |
| Outlet | `result` **(primary)** | The final output. Its type is chosen by the `result` configuration key: `bytes` gives `Bytes`, `json` gives a structured value | — |
| Outlet | `record` | Open object decoded from the recorded canonical bytes for non-terminal progress, tool-requested and tool-completed records | Wire by name |
| Outlet | `tool_request` | Open object: a tool call to be executed | Wire by name |
| Outlet | `_error` | The failure | Wire by name |

`turn` accepts text only. An object payload is read as an event and its `op` string is
looked up — but no `op` is registered, so every object payload is refused. Objects are not
stringified into prompts.

Within one harness outcome, the recorded progress emissions come first, then either the
tool request or the terminal result. A tool-completion record comes before the next harness
step.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `harness` | String, one of the harness adapter names this build declares | **Yes — no default** | Which external harness to invoke. `actor.create-inputs` publishes the declared harness adapter names as this slot's closed value space. The program it runs is the state's binding for that name (`circular harness bind`, read back with `agent.harnesses`). While the name has no binding the actor stands, and a harness call it makes waits until a binding stands (`daemon.health` `waiting`, reason `harness_unbound`). While the bound program does not exist or is not an executable file, the call is not started and waits the same way (reason `harness_unusable`, detail code `agent_harness.program_not_executable`). |
| `queue_capacity` | Integer (`bigint`), at least 1 | **Yes — no default** | The most turns that can wait inside the agent while it works on one. Wired turns wait at the destination inlet while the actor is busy. Inlet capacity and delivery policy govern those waiting inputs. |
| `result` | `"bytes"` \| `"json"` | **Yes — no default** | Selects the type of the result outlet. `json` removes the need for a parse step downstream. |
| `tools` | Any value | No — default `[]` | The tool policy. The actor factory reads an array of objects with a `name` string and builds an allow-set from them. An empty array is an empty allow-set. A non-object entry encountered by the parser prevents initial activation. A live edit with that entry is rejected before commit. An object without a string `name` stops policy parsing and leaves tool names unrestricted. |
| `approval` | `"none"` \| `"required"` | No — default `"none"` | Whether the harness step needs approval. Applies to this actor's harness invocation only, not to the tools a tool request reaches. |

The catalog publishes `tools` as `Any`, with default `[]`. The actor factory applies the
restrictions above during initial activation and live-edit validation. There is no `fallback`
key; a declaration that writes one is refused.

`tools: ["echo"]` — an array of bare strings — passes schema admission. The actor factory
requires objects, so this value prevents initial activation. A live edit with this value
is rejected before commit.

## State

Three states, each carrying an optional harness session id and an internal FIFO of queued
turns. Wired turns wait at the destination inlet while the actor is busy.

- **Ready** — nothing in flight.
- **Invoking** — one harness step in flight, holding its request.
- **AwaitingTool** — exactly one tool call in flight, with the exact call id, tool and
  arguments that the result must match.

The session id is an opaque logical name scoped to its harness. The checkpoint hook carries
logical session and turn state when the actor is ready or awaiting a tool result.
Configuration edits use this hook for state transfer. The hook returns no state during a
harness invocation. These values do not contain external clients, processes or sockets.
Restart reconstructs the state by replaying this actor's recorded arrival column. The replay
starts after the actor's checkpoint cache when that cache matches the recorded arrivals, and
from the beginning otherwise.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | Invalid harness name, queue capacity or tool declaration. |
| `InvalidTools` (actor factory) | The parser encounters a non-object tool entry or an invalid tool name. Initial activation fails. A live edit is rejected before commit. |
| `InputOutOfDomain` | A tool result arrives with no call in flight, or with a call id that does not match the active one; a harness step or result violates its canonical shape; a `turn` object payload whose `op` is absent, non-string, or not registered. |
| `DomainRejected` | The turn queue is full, or a value falls outside the explicit tool policy. |
| Harness invocation failure | Emits an error message on `_error` with the failure kind. |

Input and policy rejections are recorded as dead letters. Explicit diagnostics, including
harness invocation failures, are emitted on `_error`. An unwired `_error` emission does not
produce a fallback dead letter.

## Example

```ts
import { agent } from "@circular/core";

export const assistant = agent({
  harness: "claude",
  queue_capacity: 8n,
  result: "json",
});

export const progress = assistant.out.record;
export const calls = assistant.out.tool_request;
```
