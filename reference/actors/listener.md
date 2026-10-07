# listener

Listens to an external origin and turns what appears there into events. The published origin
is a file tail: a glob selects the files, and polling produces line events.
A control pulse makes it replay the matched origins from the beginning.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `control` **(primary)** | Closed object `{ op: String }` | No — optional |
| Outlet | `line` **(primary)** | Open object: one line and where it came from | — |
| Outlet | `_error` | The failure | Wire by name |

The inlet is **optional** on purpose: a control pulse can come either from a wire inside the
graph or from an external injection, and a graph that only uses injection must still be able
to stand this actor up.

`line` is an open object because what a line carries depends on the origin arm. For the
file-tail arm it is the text together with the file and offset it came from. Closing the
shape would force a different outlet type per arm, which would make this one actor into
several.

The `control` vocabulary is owned by this actor and is closed:

| `op` | Meaning |
| --- | --- |
| `replay_from_start` | Re-read every currently matched origin from the beginning. |

Re-scanning uses the origin path, file identity and byte offset as the injection
idempotency key. A replayed line is an ordinary emission — it carries no marker saying it
was replayed.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `source` | Closed object `{ kind, value }` | **Yes — no default** | The origin to listen to: kind file_tail, whose value names the files to follow (glob) and the milliseconds between reads (poll). `kind` is a closed enumeration with one published arm, `"file_tail"`, whose `value` is `{ glob: String, poll: Int ms }`. |
| `capabilities` | `{ FsRead: { approval, roots } }` | **Yes — no default** | The grant for reading the origin files. `approval` is `"none"` or `"required"` and `roots` is an array of absolute directories; both must be written. Without the key the declaration is refused with `config.capabilities = <missing>`, and without the entry with `config.capabilities.FsRead.approval = <missing>`. |

With `capabilities.FsRead.approval` set to `"required"` the tail waits at its first open
until the read is approved. There is no top-level `approval` key; a declaration that writes one
is refused.

An **empty `glob` is refused**: a glob matching nothing is normal (the origin may not exist
yet), but an empty glob is an authoring mistake, and this is the only place the two can be
told apart. `poll` is integer milliseconds and **zero is outside the value space** — a
zero-interval poll is not a poll.

There is no mount name in this configuration. The name of the injection mount is owned by
the mount registration, so putting it here would give the same fact two sources.

Editing `source` replaces the incarnation, since absorbing it would mean carrying the old
origin's offsets into a different origin.

## State

The tail keeps offsets in memory; the actor has no checkpoint. A new tail starts at the
end of files present on its first scan. The origin file itself holds the content.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | `source` is not an arm of the closed sum; `glob` is empty; `poll` is not an integer, or is zero or negative. |
| Read failure | The origin could not be read. The failure is reported and the next poll tries again. |
| Unknown `op` | An `op` outside the closed enumeration is not applied. The actor emits the diagnostic string `control pulse op is not a supported variant` on `_error`. |

## Example

```ts
import { listener } from "@circular/core";

export const lines = listener({
  source: {
    kind: "file_tail",
    value: { glob: "/var/log/app/**/*.jsonl", poll: 500n },
  },
  capabilities: {
    FsRead: { approval: "none", roots: ["/var/log/app"] },
  },
});

export const text = lines.out.line;
```
