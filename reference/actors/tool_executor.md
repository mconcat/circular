# tool_executor

Takes one tool call, matches its name against a finite allowlist you configure, and runs
the single concrete external effect that allowlist entry describes. The outcome comes back
as one tool result carrying the same call id, which is what an `agent` needs to continue.

The executor runs the effect templates named in `tools`.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `call` **(primary)** | `{ id: Bytes, tool: String, arguments: Bytes }`: one tool call | Yes |
| Outlet | `result` **(primary)** | `{ call: Bytes, effect: String, ok: Bool, value }`: one tool result | — |
| Outlet | `_error` | The failure | Wire by name |

The number of tools in the allowlist does not change the port set.

### The call

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | Bytes, not empty | The call id. It comes back unchanged as the result's `call`. A String is accepted and read as its UTF-8 bytes. |
| `tool` | String | A key of `tools`. |
| `arguments` | Bytes | For a `spawn` tool, the standard input of the process. For a `file_write` tool, the contents written. A `file_read` tool does not read it. A String is accepted and read as its UTF-8 bytes; pass `b''` when there is nothing to send. |

All three fields must be present. A payload with another shape — `name` in place of
`tool`, or an object as `arguments` — is recorded as a dead letter. It produces no external
effect. A `tool` that is not a key of `tools` is also recorded as a dead letter.

An `agent`'s `tool_request` outlet already emits calls in this shape. To build a call on a
wire, use CEL bytes literals in a `map`:

```ts
breach.out.transition
  .map("{'id': b'rollback-1', 'tool': 'remediate', 'arguments': b''}")
  .into(tools.in.call);
```

### The result

| Field | Type | Meaning |
| --- | --- | --- |
| `call` | Bytes | The `id` of the call this result answers. |
| `effect` | String | `"file_read"`, `"file_write"` or `"spawn"`. |
| `ok` | Bool | Whether the effect succeeded. |
| `value` | — | On success: the Bytes read, the UInt length written, or `{ exit: Int, stdout: Bytes, stderr: Bytes }` for a spawn. On failure: one of the strings `"parameter_denied"`, `"diverged"`, `"endpoint_gone"`, `"approval_required"`, `"transport_terminal"` or `"interpreter_fault"`. |
| `detail` | UInt | Present only when `value` is `"parameter_denied"` or `"diverged"`. For `"parameter_denied"` it is the capability that refused the call: `4` `FsRead`, `5` `FsWrite`, `6` `ProcessSpawn` for this actor's effects. For `"diverged"` it is `1` (`MissingRecord`: replay found no record for the call) or `2` (`EffectMismatch`: the recorded effect differs). |

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `tools` | Object: tool name → effect template | **Yes — no default** | The tools this actor may run, each named with the one effect it performs; for example, read_log with effect file_read and a path. Each key is a tool name and each value is the template of exactly one concrete effect (below). |
| `capabilities` | `{ FsRead?, FsWrite?, ProcessSpawn? }` | **Yes when `tools` uses those effects** | One grant per effect the tools use. The grants are `FsRead: { approval, roots }` for a `file_read` tool, `FsWrite: { approval, roots }` for a `file_write` tool, `ProcessSpawn: { approval }` for a `spawn` tool. `approval` is `"none"` or `"required"` and `roots` is an array of absolute directories. A missing grant refuses the declaration, for example with `config.capabilities.ProcessSpawn.approval = <missing>`. With `tools: {}` no grant is needed. |

A tool may also carry its own `approval` key (`"none"` or `"required"`). A call waits for
approval when either the tool's `approval` or the grant's `approval` is `"required"`. This
actor has no top-level `approval` key.

The published schema types `tools` as an open object, which is an upper bound: the
vocabulary for attaching a per-entry template schema does not exist. The decoder accepts
these template forms. The change is checked with the same decoder the actor runs when it is
activated, so an entry outside these forms is refused before the change commits, with
`ConfigRejected` at `config.tools`:

| `effect` | Other keys |
| --- | --- |
| `"file_read"` | `path` |
| `"file_write"` | `path`, `mode` (`"create"` or `"replace"`) |
| `"spawn"` | `program`, `arguments` (array of strings) |

A duplicate tool name, an unknown effect tag, a partial template, or a template that does
not produce exactly one effect for a call is refused.

## State

Either idle, or pending with exactly one call and the tag of the concrete effect it
produced.

This actor is **single-flight**. While an effect is pending, wired calls wait at the
destination inlet. Calls from multiple producers use that inlet's capacity and delivery
policy. A call that reaches the actor hook while another is pending is rejected.

A concrete effect that fails is **normal tool result data** on `result`. Input, allowlist,
busy-call and mismatched-outcome rejections are recorded as dead letters. Explicit
diagnostics, such as a missing grant or an outcome received while idle, use `_error`.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | Duplicate tool name, forbidden effect constructor, partial template, or a configuration that cannot turn one call into exactly one effect. |
| `InputOutOfDomain` | The input payload is not a canonical tool call. No external effect is produced. |
| `DomainRejected` | The tool name is outside the allowlist, or a second call arrived while one was pending. The active call is kept. |
| `DomainRejected` | A success outcome whose payload shape does not match the effect tag that was submitted. The pending call is settled. |

## Example

```ts
import { toolExecutor } from "@circular/core";

export const tools = toolExecutor({
  tools: {
    read_log: { effect: "file_read", path: "/var/log/app.log" },
    remediate: { effect: "spawn", program: "/usr/local/bin/remediate", arguments: [], approval: "required" },
  },
  capabilities: {
    FsRead: { approval: "none", roots: ["/var/log"] },
    ProcessSpawn: { approval: "none" },
  },
});

worker.out.tool_request.into(tools.in.call);
tools.out.result.into(worker.in.tool_result);
```

Here `worker` is an `agent` declared elsewhere in the same program. The spawn program must
also be in the daemon's `[process] allowlist`.
