# `@circular/client`

Executable client surface built around an established `Session`. `establish` performs the `Hello`/`HelloAck`
exchange, starts one correlation-aware receive fold, and exposes declaration, query, subscription, interaction,
and replay partitions.

For owner-local authoring, request the session roles explicitly:

```js
const session = await establish(transport, {
  resourceCeilings,
  hello: { requestedRoles: [1n, 4n, [2n, []]] },
});
```

The sequence is Reader, Operator, and root Writer in canonical Value byte order. Omitting
`hello.requestedRoles` sends `[]`; it does not request authoring authority. A refused Hello throws
`SessionError` with `code === "ESTABLISHMENT_REJECTED"`. Its message displays the daemon's code,
message, and optional hint, and its standard `cause` retains the original rejection object without
adding absent fields. No session is returned after refusal.

After establishment, ordinary command rejection remains a `Rejected` result value. Query pages retain their anchors and explicit
terminal markers. Subscription handles retain the server-selected delivery discipline; only credit-based
handles can grant credit.

Declaration methods return a `CorrelatedRequest`: its session-local correlation is available before transport
completion, while its `completion` preserves the ordinary accepted-or-rejected result. This lets the authoring
host identify a pending epoch without inventing a second transaction envelope.

The `internal/execution` host-adapter subpath opens the mandatory-baseline epoch before module evaluation,
offers a synchronous `emit(command)` sink, and performs validate/commit or abort after evaluation. Its trace is
only the exact declaration request values already sent on the wire. Authored programs do not import this
subpath and do not call begin/submit/commit themselves. Validation or content rejection is followed by abort;
a commit rejection is already terminal and is never followed by `AbortEpoch`.

The replay subpath exposes the settled arrangement, readiness, session, start, rewind, end, and display-query
contracts. Forward seek, standalone pace mutation, operation cancellation, and rewind-cancellation methods are
not declared because their exact RPC mappings remain open.

`@circular/client` exposes owner-local connection facts and observation readers:

```js
import { OWNER_LOCAL_SOCKET_NAME, OWNER_LOCAL_RESOURCE_CEILINGS,
  runtimeApprovals, runtimeApprovalsPageFromValue, decideApproval } from '@circular/client';

const reply = await session.exchange('Query', 'Query', { name: runtimeApprovals.name, args: null });
if (reply.payload[0] === 1n) {
  const page = runtimeApprovalsPageFromValue(reply.payload[1]);
  // selectedIndex is chosen by the caller after displaying this page.
  const row = page.items[selectedIndex];
  if (row) await decideApproval(session, { item: row.item, decision: 'Approve' });
}
```

The constants are also exported by `@circular/client/owner-local`. They mirror
`circular_transport::OWNER_LOCAL_SOCKET_NAME` and `circular_core::Ceilings::PROVISIONAL`.
A caller passes the ceilings explicitly to `establish`; they are fixed wire limits.

`daemonHealthPageFromValue`, `timelinePageFromValue`, `recordsPageFromValue`,
`actorEventsPageFromValue`, and `runtimeApprovalsPageFromValue` read the successful
query page body. They preserve UInt carriers, cursors, terminal arms, cut vectors,
and opaque record bytes. Timeline pages remain caller-paged; the reader does not
collect pages or infer histogram bins. A missing actor in `daemon.health` has no
published health row: absence does not establish that the actor is alive or dead.
`daemonHealthPageFromValue` also reads each row against the anchor's `lifecycle` word: a row is
its actor's last recorded life, and a boot or restart that stood no pipeline records none. When
the word says no pipeline stands (`activation_failed`, `recovery_failed`, or `null` before any
stream opened), every row's `state` is `not_standing` and its recorded state is in `recorded`.
A stopped pipeline still stands, so its rows keep their recorded state.
`decideApproval` returns the existing accepted/rejected TransitionResult arms;
transport failures and malformed or mismatched receipts throw. `Deny` uses the same
function and existing daemon verb.

`actorEventParents(row, rows)` reads one `actor.events` row's recorded `causal_parents` as the
rows they name, over rows the caller already holds; it fetches nothing. Each recorded parent is
`found` (that row), `absent` with `not_in_rows`, or `undecidable` with `emission_arrived_twice`
(the sending actor recorded two arrivals of one emission) or `no_sending_end` (a row that did not
arrive over a wire names an emission) or `stamp_shared` (a given row's `at` is the stamp and
another given row also carries it, or a given row carries the parent of a `_lifecycle` row, which
is a record and never a row; two events were issued one stamp — today only an emission and its
sender's next arrival can be). A row without
`causal_parents` answers `unknown`.

`agentHarnessCandidatesPageFromValue` reads the `agent.harness-candidates` page: one
`{name, found}` row per harness adapter the daemon declares. `found` is the first of that
adapter's declared install locations a bind would accept when the daemon answered, or
`null`. The daemon measures its own home and filesystem and does not read `PATH`. The
answer binds nothing: a binding is one `SetAgentHarness` command (`setAgentHarness`), and
`agent.harnesses` lists the bindings.

`setAgentHarness(session, { name, program })` sends the `SetAgentHarness` command: the
daemon checks the name and the executable, rewrites that name's entry in its settings
document (`config.toml` `[agent] harnesses`), and hands the binding to the standing
pipeline, where an agent that declares the name takes the new executor between turns.
`program: null` erases the entry. Nothing is recorded in the journal. A refusal (6401,
6402, 6403) is returned as a result arm. `agentHarnessesPageFromValue` reads the
`agent.harnesses` page: one `{name, program, saved}` row per name, where `program` is what
the standing pipeline holds when the daemon answered and `saved` is what the settings
document says; either can be `null`.

`edgeDepths` is the `edge.depths` credit subscription (Null args, opened through
`openSubscription`). Opening it asks every standing actor once, and each frame carries one row as
an actor answers, so a fast actor's rows arrive while a slow one is still in its turn. An edge row
is `{edge, depth, queued, capacity}`: one edge into an actor that answered, measured by that
receiving actor at its own inlet. `edge` is the edge identity carrier `actor.events` rows use.
`depth` counts inputs received on the edge and not yet arrivals (held at the inlet, or waiting to be
recorded); it is the same measure an actor-health transition's `mailbox_depths` carries. `queued`
counts recorded arrivals of the edge not yet consumed. `capacity` is the wire's declared capacity,
or `null` when the wire declares none. An actor that ended before it answered gives one
`{actor, code}` row with the daemon's `EndedBeforeAnswering` code. The stream ends with reason
`Complete` once every asked actor answered or ended. An actor that never answers gives no row and
the stream stays open until the client closes it; that actor's health row says why.
`edgeDepthsItemFromValue` reads one frame payload. The answer is the value now and is not recorded;
a past cut shows depths only where an actor-health transition recorded them.

Revision mismatches currently share `Malformed=1` with other declaration failures.
No dedicated revision-conflict reason or cross-epoch undo/redo API is published by
this release; the producer work and the undo/redo format are still open.
