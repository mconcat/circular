# notify

Submits user notifications on a configured channel, spaced by `minimum_interval`.
When a notification arrives inside that interval, what happens to it is your choice:
suppress it, keep only the latest, or queue them all.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `notification` **(primary)** | Object `{ title: String, body: String }` | Yes |
| Outlet | `_error` | The failure | Wire by name |

There is no ordinary outlet: `notify` is a terminal sink. Whether a successful delivery
acknowledgement should be emitted as an event is **not specified in the published
semantics**; today a matching success produces nothing.

Both `title` and `body` are required strings. A non-object payload, or a missing or
non-string `title` or `body`, fails the projection. Empty strings are not separately
refused. Nothing is synthesised for you — if an upstream payload has a different shape,
build `title` and `body` explicitly in the wire preprocessing.

How a channel renders the two fields depends on what the daemon binds that channel name to.
For a Slack webhook binding it is a single `text` field holding `*<title>*`, a newline, then
`<body>`. For a local program binding, `title` and `body` are the first two arguments, and
standard input is left empty.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `channel` | String | **Yes — no default** | The logical channel name. The daemon's configuration binds this name to an actual sink. |
| `minimum_interval` | Integer milliseconds (`bigint`) | **Yes — no default** | The shortest gap between two submissions. Zero means no cooling at all. |
| `during_interval` | `"suppress"` \| `"latest"` \| `"queue"` | **Yes — no default** | What to do with a notification that arrives during the interval. |
| `retry_delays` | Array of whole milliseconds (`bigint`), each at least 1 | No — default: the daemon's `[effects] retry_delays` | The waits between retries of a delivery, in milliseconds. A delivery is retried when it never reached its receiver or when Slack answered with `429` or `503` (see Rejections). The length of the array is the number of retries; `[]` never retries. |
| `capabilities` | `{ UserNotify: { approval } }` | **Yes — no default** | The grant for the notification effect. `capabilities.UserNotify.approval` is `"none"` or `"required"` and must be written. Without the key the declaration is refused with `config.capabilities = <missing>`, and without the entry with `config.capabilities.UserNotify.approval = <missing>`. |

A delivery waits for approval when `capabilities.UserNotify.approval` is `"required"`. There is no
top-level `approval` key; a declaration that writes one is refused.

There is no slot for projecting a title or a body out of the payload. A negative
`minimum_interval` is refused before the change commits; `0` means no cooling. An empty
channel name is also refused before the change commits.

## State

Either ready, or cooling. While cooling the actor holds the current arming and the deferred
work, whose shape follows the policy: `suppress` defers nothing, `latest` holds at most one
payload, `queue` holds a FIFO of payloads. Deferred payloads are kept whole.

The interval is measured from the moment of **submission**, not from a delivery
acknowledgement, and the boundary is realised by a wake-up whose arming matches — the actor
does not compare clock values. With `minimum_interval` of zero the actor stays ready and
arms nothing.

When the cooling wake-up arrives with something deferred, one notification goes out
(the latest, or the head of the queue), the interval is armed again, and the rest of the
queue is kept. With nothing deferred, the actor returns to ready.

Restart reconstructs deferred notifications and pending schedules by replaying this actor's
recorded arrival column. The replay starts after the actor's checkpoint cache when that
cache matches the recorded arrivals, and from the beginning otherwise. Configuration edits
use checkpoint hooks to transfer logical state that satisfies the new configuration. Pending
schedules also carry over during these edits.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | Channel, interval or interval policy does not satisfy the schema. |
| `EffectFailed` | Delivery failed: channel parameter refused, transport exhausted, interpreter fault. The diagnostic is emitted on `_error`. An unwired `_error` does not produce a fallback dead letter. |
| Retried delivery | A delivery that never reached its receiver — the Slack host name did not resolve, the connection was refused or timed out before it was established, or the system program could not be started — or that Slack answered with `429` or `503` is sent again by the effect executor after the waits in `retry_delays`. The actor sees only the last settlement; after the last wait it is `retry_exhausted` with the number of attempts. With `retry_delays: []` the first failure reaches the actor as it is (`transport_unreached` or `remote_deferred`). A delivery that may already have arrived is not sent again and fails at once with `transport_terminal`: a Slack `5xx` other than `503`, a connection that dropped or timed out after the request was sent, or a system program that ran and did not exit 0. A Slack `4xx` other than `429` is the channel refusing the request, `ParameterDenied`, and is not retried. Pause and Force Pause cancel a waiting retry. |
| `cooling` (suppression) | A notification arrived during the interval under the `suppress` policy. This is a normal suppression, not a failure. |
| Projection failure on arrival | An input that arrives while the actor is ready cannot be turned into a title and body. This `InputOutOfDomain` rejection is recorded as a dead letter regardless of `_error` wiring. Under the `latest` or `queue` policy, an input that arrives during the interval is kept without being projected; it is projected at the wake-up (below). Under `suppress` it is suppressed, not projected (the `cooling` row). |
| Approval required with no ticket | The mapping for this case is **not specified in the published semantics**. |

If a deferred payload fails to project when its wake-up arrives, the error text is emitted
on `_error` without a reason code, and the state is preserved. If `_error` is unwired, that
failure leaves no record: no dead letter is written. Whether that payload is discarded or
retried, and how the rest of the queue is rescheduled, is not settled — a later wake-up is
not armed automatically.

## Binding the channel

`channel` is a logical name. The daemon's `config.toml` binds it to a sink, one
`[[notify.channel]]` table per name, and the daemon reads that binding when it starts:

```toml
# A local program: called with the title as its first argument and the body as its second.
[[notify.channel]]
name = "ops"
sink = { system_program = "/absolute/path/to/program" }

# A Slack incoming webhook: the URL lives in the vault resource <state>/secrets/slack.webhook_url.
[[notify.channel]]
name = "slack"
sink = { slack_webhook = { secret = "slack.webhook_url" } }
```

A `slack_webhook` sink needs a `[secrets]` custody in the same file. The template
`incident-autopilot` walks through both bindings.

## Example

```ts
import { json, notify } from "@circular/core";

export const transition = json({ initial: { from: "Ok", to: "Firing" } });
export const escalate = notify({
  channel: "ops",
  minimum_interval: 60000n,
  during_interval: "latest",
  capabilities: { UserNotify: { approval: "none" } },
});

// The arriving value is named `event` in the map, whatever the inlet is called.
transition
  .map("{'title': 'Alert ' + event.to, 'body': event.from + ' -> ' + event.to}")
  .into(escalate);
```
