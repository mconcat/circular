# route

Sends each incoming event to exactly one outlet, chosen by comparing a value taken from the
payload against a finite set of cases you declare. Anything that does not match a declared
case goes to `unmatched`, which is an ordinary output and not a failure lane.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | The event, unchanged; passes through with its type preserved | Yes |
| Outlet | `route_<case>` | The same payload, unchanged | One per key in `cases` |
| Outlet | `unmatched` | The same payload, unchanged | No — but it is the only place an unmatched event goes |

The case outlets are generated from your configuration: a `cases` key `k` produces an
outlet named `route_<k>`. Case names must survive being spelled into a port name. There is
no primary outlet — `unmatched` is not an implicit default, so every outlet you want is
wired by name. There is no `_error` outlet.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `at` | Array of path segments | **Yes — no default** | The exact payload path whose value is compared. An empty array selects the whole payload. |
| `cases` | Object: case name → match value | **Yes — no default** | Each case pairs a name with a value; an event whose value matches leaves on that case's outlet. A case named `k` makes the outlet `route_k`. May be empty, in which case everything goes to `unmatched`. |

Match values are compared by **structural equality**, with no coercion: the number `1`, the
string `"1"` and the boolean `true` are three different keys, and so are `1` and `1.0`.
Array element order participates in equality. Two cases may not declare structurally equal
match values.

If the path selects nothing — a missing key, an array index out of range, or a mismatched
shape partway down — that is not an error; the event goes to `unmatched`.

## State

Stateless. The case table and the path come from configuration, and no previous input,
previous key, or access order is remembered. Outlet selection depends on the configured
path, cases and payload.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | `at` is not a valid path expression; `cases` is not an object map; a case name or the derived port name is outside the canonical port-name form; two cases declare structurally equal match values. |

No runtime rejection. Every accepted event produces exactly one emission.

## Example

```ts
import { route } from "@circular/core";

// The case outlets are named by admission, so supply their names as the
// output type parameter to reach them through `.out`.
export const byKind = route<never, Record<"route_usage" | "route_error", unknown>>({
  at: ["type"],
  cases: { usage: "usage", error: "error" },
});

export const errors = byKind.out.route_error;
export const rest = byKind.out.unmatched;
```
