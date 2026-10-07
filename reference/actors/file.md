# file

A two-way actor bound to one real file on disk. A write replaces the whole file; a pulse on
`read` reads the whole thing back. The file itself holds the content — the actor does not
copy it into its state or a checkpoint, so after a restart a `read` simply reads the real
file again.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `write` **(primary)** | `Bytes` or `String`, submitted as a whole-file replacement | Yes |
| Inlet | `read` | Any value; the content is not read — it is a pulse | No — optional |
| Outlet | `content` **(primary)** | `Bytes`: what was read, undecoded | — |
| Outlet | `written` | Int: how many bytes were written | Wire by name |
| Outlet | `_error` | The failure | Wire by name |

Configuration does not expand the port set.

There is no `encoding` key: `Bytes` are written as they are, a `String` is written as its
UTF-8 bytes, and anything else is refused. `content` is bytes with no text decoding applied.
There is no `mode`, no size cap and no flush period.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `path` | String | **Yes — no default** | The file this actor is bound to, as an absolute path. |
| `capabilities` | `{ FsRead: { approval, roots }, FsWrite: { approval, roots } }` | **Yes — no default** | The grants for reading and for writing. Both entries must be written, each with `approval` (`"none"` or `"required"`) and `roots` (an array of absolute directories). Without the key the declaration is refused with `config.capabilities = <missing>`, and without an entry with, for example, `config.capabilities.FsRead.approval = <missing>`. |

`path` must be absolute. Activation normalises `..` segments and refuses a path that
escapes the filesystem root; the grant’s `roots` are checked when a read or write runs. A
relative path, or one that escapes the root, passes the create-input schema but is refused
before the change commits, with `ConfigRejected` at `config.path`. The allowed range is checked by the
grant on every execution: a path outside `roots` is accepted too, and each read or write of
it leaves on `_error` as
`file failed: parameter_denied`. A read or write waits for approval when the matching grant's
`approval` is `"required"`. There is no top-level `approval` key; a declaration that writes one is
refused.

Editing `path` means a different file, so it replaces the incarnation.

A replacing write opens the file with create-and-truncate: **a file that does not exist is
created by the write itself**, and an existing file is replaced with no old tail left behind.
There is no separate create step, existence check or retry. Parent directories are not
created for you, and the replacement is not promised to be atomic.

## State

Either idle, or pending one read or one write. Waiting wired inputs are held at the
destination inlet.

This actor is **single-flight**. While one effect is pending, wired inputs wait at the
inlet. The inlet releases them after the actor consumes the previous effect's outcome.
A read wired after a write therefore runs after that write settles. It does not block other
processes from changing the file.

Waiting wired inputs are subject to inlet capacity and delivery policy. The actor has no
internal input queue. Restart reconstructs its execution state by replaying its recorded
arrival column. The replay starts after the actor's checkpoint cache when that cache matches
the recorded arrivals, and from the beginning otherwise. The file's contents remain on disk.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | `path` is missing or does not satisfy the schema. |
| `InputOutOfDomain` | A `write` payload that is neither `Bytes` nor `String`. It is recorded as a dead letter regardless of `_error` wiring. The current pending effect is untouched. |
| Unknown inlet | An arrival on an inlet this actor does not have. The current pending effect is untouched. |
| Missing grant | The capability grant needed to read or write is not present. |
| Terminal failure | The file to read is missing, the write failed, or the path is outside the allowed range. The failure kind is preserved. |
| Outcome kind mismatch | A settlement whose kind does not match the effect that was submitted. |
| Written length out of range | The number of bytes written does not fit the integer outlet. It is refused rather than truncated. |

Invalid write payloads are recorded as dead letters. Explicit file failure diagnostics
use `_error`. Consuming a failed effect outcome also lets the inlet release waiting inputs.

Watching a file for changes, appending, a set of encoders, and checkpointing file contents
are all outside this actor's scope.

## Example

```ts
import { json, file } from "@circular/core";

export const report = json({ initial: "last report" });
export const scratch = file({
  path: "/path/to/reports/last-report.txt",
  capabilities: {
    FsRead: { approval: "none", roots: ["/path/to/reports"] },
    FsWrite: { approval: "none", roots: ["/path/to/reports"] },
  },
});

report.into(scratch.in.write);
export const written = scratch.out.written;
export const contents = scratch.out.content;
```

Replace `/path/to/reports` with an absolute directory you own that already exists; the
actor does not create parent directories. Configuration values are literals, so write the
absolute directory out in each place; a `const` shared between them is refused by the
prepass.
