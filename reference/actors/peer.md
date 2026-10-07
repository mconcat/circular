# peer

> **Experimental.** External-session messaging requires an enabled adapter. The test-only
> `memory` adapter is registered by default; see
> [Turning on an adapter](#turning-on-an-adapter-experimental) and
> [Known issues](../../README.md#known-issues-alpha).

A two-way boundary between the pipeline and external agent sessions. `send` submits text to
the chosen session; `message` carries messages returned through that harness's own mechanism.
For `claude`, these are messages sent to Circular's socket; for `codex`, these are agent
messages from turns this binding's `send` started. Inbound messages are recorded as outcomes
in this actor's arrival column before adapter acknowledgement. A submission receipt and an
inbound message are separate events; no Circular-specific reply tool is required.

Advertisement is optional: `claude` advertises Circular as a peer, while `codex` does not.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `send` **(primary)** | Open object `{ target: { adapter, realm, peer }, body, correlation?, provider_fields? }` | Yes |
| Inlet | `refresh` | Any value; the content is not read — it asks for a fresh peer snapshot of the current realm | Yes |
| Outlet | `message` **(primary)** | Open object: one inbound message, after it is recorded in this actor's arrival column; the adapter is acknowledged after that | — |
| Outlet | `peers` | Open object `{ realm, peers: [...], observed_at }` | Wire by name |
| Outlet | `delivery` | Open object: the submission receipt `{ message, provider_id, disposition, accepted_at }` and later delivery changes | Wire by name |
| Outlet | `binding` | Open object `{ id, actor, address, effective_name, lease, capabilities }` | Wire by name |
| Outlet | `_error` | The failure | Wire by name |

On `send`, `body` is text and `provider_fields` maps string keys to bytes. In `target`,
`adapter` is text but `realm` and `peer` are **bytes**. A `send` whose `realm` or `peer` is a
string is refused with `InputOutOfDomain: invalid peer send` on `_error`. There are three ways
to get a target:

- **Reply.** An inbound message's `reply_to` is a complete target. Send it back as-is.
- **Spell it from a Claude session's advertisement.** The `claude` adapter reads session
  files `<pid>.json` in `sessions_dir` (see below). A file's `messagingSocketPath` is
  the address: `peer` is `uds:` followed by that path. JSON has no bytes, so feed plain text
  and let a `map` on the wire convert it with CEL `bytes(...)`:

  ```ts
  import { input, peer } from "@circular/core";

  export let outbox = input({ label: "outbox" });
  outbox.mount("outbox", "request");
  export let planner = peer(
    { adapter: "claude", realm: "team", name: "planner",
      inbound_policy: { any_known_peer: true }, inbox_capacity: 64n },
    { send: outbox.map("{'target': {'adapter': 'claude', 'realm': bytes('team'), 'peer': bytes('uds:' + event.socket)}, 'body': event.body}") },
  );
  ```

  Then send `{"socket": "<messagingSocketPath>", "body": "hello"}` to `outbox`.
- **Discovery.** Send anything to `refresh`; the `peers` outlet lists the sessions it found, and
  each entry's `address` is a target.

A submission receipt reports accepted, queued or held. The `claude` and `codex` bridges
report `accepted`; neither emits later delivery changes.

An inbound message carries external-agent provenance. The peer path treats its text as
message data, not as an approval ticket or capability grant.

Attaching an observation mount **directly** to a `peer` port shows you the effect-settlement
summary, not the values the outlets carry. To see the values, put a `tap` between the outlet
and the mount.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `adapter` | String | **Yes — no default** | The peer adapter name. The daemon registers `memory` for tests and can enable `claude` and `codex` for external sessions — see [Turning on an adapter](#turning-on-an-adapter-experimental). `actor.create-inputs` publishes these names as this slot's closed value space; whether this daemon has enabled the named adapter is checked before the actor starts. |
| `realm` | String | **Yes — no default** | The realm this actor binds into. |
| `name` | String | **Yes — no default** | The requested binding name. The `claude` adapter also advertises it. |
| `inbound_policy` | Object | **Yes — no default** | Who may send to this actor: any_known_peer set to true for every sender on this adapter and realm, or exact with the list of allowed addresses. There is no implicit authorisation and no default. |
| `inbox_capacity` | Integer (`bigint`), at least 1 | **Yes — no default** | How many incoming messages can wait before this actor records them. |

`adapter`, `realm` and `name` refuse empty labels and labels with surrounding whitespace or
control characters.

`inbound_policy` takes exactly one of two mutually exclusive forms:

| Value | Meaning |
| --- | --- |
| `{ "any_known_peer": true }` | Every sender reaching the same adapter and realm is accepted, including senders not in the discovery snapshot. |
| `{ "exact": [{ "adapter": "...", "realm": "...", "peer": "..." }] }` | Only the listed addresses. The array may hold several. |
| `{ "exact": [] }` | No sender is accepted. |

`false`, an extra top-level key, an empty object, both forms at once, an extra field inside
an exact address, and a duplicate address after normalisation are all refused.

The effective binding name must equal `name`. If the provider hands back a different effective
name — a suffix to resolve a collision, say — the binding is refused rather than accepted
under another name.

## Turning on an adapter (experimental)

The daemon registers `claude` and `codex` when `<state>/config.toml` has a
`[[peer.adapter]]` entry for each enabled adapter. Without the matching entry, an edit
that adds a `peer` naming that adapter, or changes the config of such a `peer` while it
still names that adapter, is rejected before commit (`ValidateEpoch` and `CommitEpoch`) with `ConfigRejected` at
`config.adapter`, and nothing is committed. The daemon decides this by comparing the
pipeline after the edit with the committed one, so a `peer` declared directly, one
expanded from a template, and one moved to another scope with `MoveToScope` all count as
added. A `peer` that the edit leaves unchanged, retires, or changes only the flags of is
not checked. If the entry is removed and the daemon restarted while a committed pipeline
still has such a `peer`, `daemon.health` reports that actor as `failed` and the other
actors run; an edit that leaves the `peer` unchanged commits (including one that sends
the same declaration again), an edit that changes its config or moves it while it still
names the unregistered adapter is rejected the same way, and an edit that switches it to a
registered adapter commits. The daemon registers `memory` for tests
without an entry.
Unknown adapter names, unknown keys and duplicate adapter entries are config errors.
The daemon reads these entries at startup; restart it after changing them.

### Codex

```toml
[[peer.adapter]]
name = "codex"
```

| Key | Default | Meaning |
| --- | --- | --- |
| `name` | none — required | `codex`; this is the only key accepted in a Codex adapter entry. |

There are no Codex adapter settings for directories, a program path or a display name.
On connection, the bridge starts `codex app-server` as a child process of the daemon and
talks to it over standard input and output, using the `codex` executable on the daemon's
`PATH`. It does not start or use a separate app-server daemon.
It initializes the connection with `experimentalApi: true`.

Discovery pages through `thread/list` for stored thread metadata. It calls neither
`thread/loaded/list` nor `thread/read`. A target has `adapter: "codex"`, the
actor's chosen `realm` as UTF-8 bytes, and the raw thread ID as UTF-8 bytes in `peer`;
there is no `uds:` or `thread:` prefix. Discovery uses the thread name when nonempty,
otherwise its ID, and reports `idle` as idle, `active` as busy and other statuses as unknown.

For each `send`, the bridge calls `thread/resume`, then `turn/steer` with `expectedTurnId`
if a turn is in progress, or `turn/start` otherwise. A successful response produces an
`accepted` receipt whose `provider_id` is the returned turn ID as bytes; it does not wait
for the turn to finish. Steering a turn started elsewhere does not make its agent messages
eligible for this binding's `message` outlet.

A thread with no turns yet cannot be resumed by this child app-server. A thread that
another Codex client (the Codex app or the `codex` terminal UI) has open also refuses
`thread/resume`. These failures occur on `send` and appear as `peer_adapter_unavailable`
on `_error`.

The bridge pages through `thread/turns/list` in ascending order with `itemsView: "full"`
and selects `agentMessage` items from turns this binding started. It emits those messages
only once their turn's status is no longer `inProgress`. The `item/completed` and
`turn/completed` notifications trigger another scan; `item/completed` alone does not make
an in-progress turn eligible. Each selected agent message becomes one inbound message,
with its text as `body`, the UTF-8 bytes of its item ID as `id` and `provider_id`, and the
thread address as `from` and `reply_to`. Tool items and agent messages from other turns
are excluded.

Within a binding, the bridge assigns increasing cursors and retains pending messages
until acknowledgement; a receive request can return an unacknowledged message again when
its cursor is after the request's `after` cursor. Acknowledgement follows the recorded
arrival, removes the matching envelope at the head of the pending queue and advances the
adapter's local acknowledged position; it sends no item-ack RPC to Codex. A full pending
queue stops further scanning until acknowledgement frees room.

After a daemon restart, a previously bound peer requests a new binding, whose tracked
threads, started turns and pending messages begin empty. The new binding does not receive
outstanding replies to messages sent before the restart.

### Claude

```toml
[[peer.adapter]]
name = "claude"
sessions_dir = "/Users/you/peer-try/sessions"
socket_dir = "/Users/you/.peer-socks"
```

| Key | Default | Meaning |
| --- | --- | --- |
| `name` | none — required | `claude`. The remaining keys in this table apply only to this adapter. |
| `sessions_dir` | `.claude` then `sessions`, under the current user’s home | Where running sessions advertise themselves. Circular reads it to find peers and writes its own advertisement there, a file named `<daemon pid>.json`. |
| `socket_dir` | `cc-socks` under `/tmp` | Where Circular opens its own socket, `<daemon pid>.sock`. Other sessions send to it. |
| `display_name` | `circular` | The advertised name when an actor gives none. A peer actor’s required `name` supplies its advertised name. |
| `from_mode` | `prompting` | The `from-mode` attribute Circular puts on every message it sends. |

**Check the two directory defaults before using them.** They are Circular adapter
defaults; the daemon does not verify the external CLI’s configured directories. Set
`sessions_dir` and `socket_dir` explicitly when testing, as in the example above.

- Paths must be an absolute path, `~`, or a path starting with `~/`. `~` and `~/` stand
  for your home directory, and nothing else in a path is rewritten: `~alice/x` is treated
  as a relative path. Both directories must already exist; Circular does not create them.
  If a path is relative or a directory is missing, the daemon still starts, and the `peer`
  reports `peer_adapter_unavailable` on `_error`.
- Keep `socket_dir` short. A Unix socket path, `<pid>.sock` included, has to fit in about 100
  bytes.
- One `peer` actor per daemon can use `claude`: the advertisement belongs to the daemon
  process. The first `peer` to bind holds it. Another `peer` on `claude`, in the same pipeline
  or another, does not bind and reports `peer_unsupported_capability` on `_error`.
- With `claude`, the realm is a label you choose. Discovery lists readable, live session
  entries in `sessions_dir` under your realm, excluding Circular's own advertisement.
- A delivery receipt of `accepted` means the other session took the message off its socket.
  It does not mean an agent has read it.
- Releasing the Claude binding removes its advertisement and socket.
- Wire `_error` somewhere you can see it. A `peer` that cannot bind can still look healthy on
  the canvas; the reason is on `_error`.

After the adapter is enabled and the daemon is restarted, the next successful activation
requests a binding. Removing and redeploying the peer is not needed for this activation.

## State

Four states: unbound; binding; bound, holding the binding, the last successful peer
snapshot, and the cursor of the last accepted inbound message; and unbinding.

The binding is the adapter-issued identity, address, lease and capabilities as logical
values. The checkpoint hook encodes logical binding state. These values do not contain
provider handles or credentials. Restart reconstructs the actor's state by replaying its
recorded arrival column. The replay starts after the actor's checkpoint cache when that
cache matches the recorded arrivals, and from the beginning otherwise.

Unfinished peer effects are recorded as interrupted during restart. A peer that was bound
or awaiting a bind requests a fresh binding when it consumes that outcome.

Within a binding, the actor records the last accepted cursor and rejects receive outcomes
at or before it. Codex's cursor and pending-message handling are described above.
When the pending inbound queue is full, the `claude` adapter stops
accepting new messages until there is room, and the in-memory adapter refuses with an
inbox-full failure.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | A required key missing or violating its value space; an `adapter` this daemon has not registered on a `peer` the edit adds or changes (rejected before commit, see [Turning on an adapter](#turning-on-an-adapter-experimental)); `inbound_policy` not in exactly one of the two forms; an exact address with a bad field, wrong type, or a duplicate after normalisation. |
| `peer_adapter_unavailable` | The adapter bridge is not reachable. |
| `peer_wrong_adapter`, `peer_wrong_realm` | The operation names an adapter or realm other than this binding's. |
| `peer_not_found` | The exact target address does not exist. No substitute target is chosen. |
| `peer_binding_not_found`, `peer_binding_stale` | While bound, the peer emits the failure on `_error` and requests a fresh binding. |
| `peer_address_stale` | The adapter reports a stale address; the peer emits the failure on `_error`. |
| `peer_inbound_refused`, `peer_inbox_full` | The inbound policy refused the sender, or the adapter's pending inbound queue is full. |
| `peer_duplicate_message` | The adapter has already seen this outbound message ID. |
| `peer_binding_has_pending_events` | The binding still has undelivered events. |
| `peer_unsupported_capability` | The adapter does not provide a required capability. Nothing is synthesised in its place. |
| `peer_submission_unknown` | The provider could not say whether it accepted the submission. The same outbox entry is **not** resent automatically. |
| `peer_name_conflict` | The requested binding name collides, or the returned effective name differs from `name`. |
| `EndpointGone` | The provider bridge broke, or a terminal operation failed. |

An ambiguous peer — a display name matching more than one candidate — is a discovery
resolution result, not an effect failure; no target is chosen for you.

## Examples

### Codex thread

Enable `codex` with the entry above. Send any value to `refresh`, then decode a returned
peer's `address.peer` bytes as UTF-8 for the thread ID. Send
`{"thread": "<thread ID>", "body": "hello"}` to `outbox`; the wire converts the realm and
thread ID to bytes.

```ts
import { input, peer } from "@circular/core";

export const outbox = input({ label: "outbox" });
export const refresh = input({ label: "refresh" });
outbox.mount("outbox", "request");
refresh.mount("refresh", "request");

export const planner = peer(
  { adapter: "codex", realm: "team", name: "planner",
    inbound_policy: { any_known_peer: true }, inbox_capacity: 64n },
  {
    send: outbox.map("{'target': {'adapter': 'codex', 'realm': bytes('team'), 'peer': bytes(event.thread)}, 'body': event.body}"),
    refresh,
  },
);

export const peers = planner.out.peers.tap();
export const inbound = planner.out.message.tap();
export const receipts = planner.out.delivery.tap();
export const errors = planner.out._error.tap();
peers.mount("peers", "result");
inbound.mount("inbound", "result");
receipts.mount("receipts", "result");
errors.mount("errors", "result");
```

### Claude session

This example needs a `[[peer.adapter]]` entry named `claude` in `config.toml`; see
[Turning on an adapter](#turning-on-an-adapter-experimental).

```ts
import { peer } from "@circular/core";

export const planner = peer({
  adapter: "claude",
  realm: "team",
  name: "planner",
  inbound_policy: { any_known_peer: true },
  inbox_capacity: 64n,
});

export const inbound = planner.out.message;
export const receipts = planner.out.delivery;
```
