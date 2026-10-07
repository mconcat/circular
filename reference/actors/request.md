# request

Projects each incoming event into one HTTP request that the configuration describes. The
request is per event and holds no batching state. URL interpolation, redirect following and
streaming are not this actor's job — a redirect is a fact in the response status.

A **transient** failure is retried by the effect executor, not by this actor. There are two
kinds. Either the request never reached the server — the host name did not resolve, or the
connection was refused or timed out before it was established — or the server answered `429`
or `503`, saying it did not process the request. The same request is sent again after the
waits in `retry_delays` (by default 1 s three times, then 5 s three times, then 15 s three
times); a longer `Retry-After` from the server is followed instead. The actor sees only the
last settlement: the response that finally came back, or `retry_exhausted` with the number of
attempts. Everything else is the first settlement, at once. Any other status — `2xx`, `3xx`,
`4xx` other than `429`, and `5xx` other than `503` such as `500`, `502` and `504` — is a fact
on `response`. A connection that drops or times out after the request was sent fails with
`transport_terminal`. Those two may already have changed something on the server, so sending
the request again could do it twice. While a retry is waiting the request is still pending;
Pause and Force Pause cancel a waiting retry, and a daemon that stops keeps it pending for
its next start.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Any value. With `method: "post"` it must be `Bytes` or `String` and becomes the body; with `"get"` it is not read at all | Yes |
| Outlet | `response` **(primary)** | Closed object `{ status: UInt, body: Bytes, truncated: Bool, retry_after_seconds: UInt \| Null }` | — |
| Outlet | `_error` | The failure | Wire by name |

Configuration does not expand the port set: method, URL and headers are configuration, not
ports.

`status` includes 4xx — **the status is a fact, and judging it is downstream's job**. A
`429` or `503` reaches `response` only when retries are off (`retry_delays: []`); otherwise it
is retried as above. Other `5xx` statuses such as `500`, `502` and `504` reach `response` at
once. With retries off, a request that never reached the server fails with
`transport_unreached`. `retry_after_seconds` carries delta-seconds only; an HTTP-date is not reinterpreted
and comes through as null. `truncated` says the response body hit the snapshot bound.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `method` | `"get"` \| `"post"` | **Yes — no default** | The HTTP method. Lowercase only: exactly `"get"` or `"post"`. `"GET"` and `"POST"` are **not** normalised; the change that declares them is refused before it commits. |
| `url` | String | **Yes — no default** | The request URL. |
| `headers` | Array | No — default `[]` | The request headers. Each entry is either `{ name, value }` for a literal header or `{ name, secret }` naming a secret held outside the graph. `name` is required, and exactly one of `value` or `secret`. Names are normalised to lowercase and carriage returns and line feeds are refused. |
| `retry_delays` | Array of whole milliseconds (`bigint`), each at least 1 | No — default: the daemon's `[effects] retry_delays` | The waits between retries of a transient failure, in milliseconds. The length of the array is the number of retries; `[]` never retries. |
| `capabilities` | `{ HttpFetch: { approval } }` | **Yes — no default** | The grant for the HTTP effect. `capabilities.HttpFetch.approval` is `"none"` or `"required"` and must be written. Without the key the declaration is refused with `config.capabilities = <missing>`, and without the entry with `config.capabilities.HttpFetch.approval = <missing>`. |

A request waits for approval when `capabilities.HttpFetch.approval` is `"required"`. There is no
top-level `approval` key; a declaration that writes one is refused.

A secret header stores its reference name in configuration; the effect resolves its
value before sending the request.

The daemon checks the request URL against `[http] hosts` in the state's `config.toml`.
Entries take three forms: `host`, `host:port` or `host:*`. On the first start of a new state,
when neither its journal nor `config.toml` exists, the daemon writes this array with mode
0600 and reports it in the startup log:

```toml
[http]
hosts = ["localhost:*", "127.0.0.1:*", "[::1]:*"]
```

An existing config is kept as written; the daemon does not merge these hosts into it.
Once the state has a journal, deleting `config.toml` leaves it absent on later starts.

After trimming whitespace around an entry, matching is case-sensitive against the URL's
authority: the host and any explicitly written port. `"api.example.com"` matches
`https://api.example.com/path` but not `https://api.example.com:443/path`.
`"127.0.0.1:9090"` matches that exact host and port, not `localhost:9090` or a URL with
no written port. `"host:*"` matches that exact host with any port or no written port;
host wildcards such as `"*.example.com"` and `"*"` are rejected. IPv6 addresses use
brackets, as in `"[::1]:*"`.

The daemon rejects entries containing a scheme or path, empty strings, duplicates,
non-strings, invalid ports (empty, nonnumeric or above 65535), and unknown configuration
keys at startup. An absent `[http]` table or `hosts = []` allows no hosts; a present
`[http]` table without `hosts` is rejected. Plain `http://` is sent only to loopback hosts;
other hosts require `https://`.

A request outside the list, or a plain `http://` request to another host, fails with
`parameter_denied` before anything is sent. The same code also covers a secret header the
vault cannot supply, and a response whose body contains a value from the secret vault — that
one is refused after the request was sent, before the response is recorded. The failure does
not say which of these happened, so the `_error` lists what the request needs, for example
`request failed: parameter_denied; hint: at least one of these does not hold: [http] hosts in config.toml lists "api.example.com" or "api.example.com:*"; the response body contains no value from the secret vault (checked after the request was sent)`.
Before the last clause, a plain `http://` URL adds
`; a plain http:// URL names a loopback host (use https:// for any other host)`, and a
request with a secret header adds
`; each secret header's name is in the secret vault and its value is UTF-8 text with no ASCII control character other than tab`.

## State

One number: how many requests have been submitted and not yet settled. Concurrent requests
are allowed and their completion order is not promised. Nothing carries over between
requests, and this count is not checkpointed — recovering an in-flight request belongs to
the settlement path, not to the actor.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` at `config.method` | `method` is anything other than `"get"` or `"post"`. The change is refused before it commits, and nothing starts. The diagnostic reads `config input config.method is outside its declared space; allowed: "get", "post"`. |
| `ConfigRejected` at `config.url` or `config.headers` | `url` is not a valid URL with a supported scheme; a header entry is not exactly one of `{ name, value }` and `{ name, secret }`, or carries another key; a header name that is not a lowercase HTTP token, a value with a line break, or an empty secret name. The change is refused before it commits. |
| Domain rejection | A `post` whose payload is neither `Bytes` nor `String`. A canonical body encoding for structured values is **not specified in the published semantics**, so the value is refused rather than stringified into a body. |
| Domain rejection | An arrival on an unknown inlet. |
| Transport failure | The request settled as a failure. The failure kind is carried through unchanged — a transport failure is not folded into a domain rejection. |
| `retry_exhausted` | A transient failure outlasted `retry_delays`. The recorded outcome carries `attempts`, the number of times the request reached for the server, first attempt included. |
| `ParameterDenied` | The URL's host is outside the daemon allowlist; a plain `http://` URL names a host that is not loopback; a secret header cannot be resolved from the vault; or the response body contains a value from the secret vault (refused after the request was sent). All four pass activation and are caught at settlement; an unavailable secret header is reported as `ParameterDenied`. |

Invalid request inputs are recorded as dead letters. Effect failures and other explicit
diagnostics are emitted on `_error`.

Durable submission is refused: there is no idempotency identifier axis, so this actor
does not accept durable inputs.

## Example

```ts
import { timer } from "@circular/core";

export const everyMinute = timer({ every: 60000n });
export const probe = everyMinute.out.tick.request({
  method: "get", // lowercase only: "get" or "post"
  url: "http://127.0.0.1:9090/api/v1/query?query=up",
  capabilities: { HttpFetch: { approval: "none" } },
});

export const responses = probe.out.response;
```

`body` is Bytes. A `json` parse reads it directly when it holds UTF-8 text:
`probe.out.response.parse({ decoder: "json", field: "body" })`. `kv` and `regex` read only a
String field, so map the body through `string(event.body)` first.
[The combinators page](../combinators.md) describes both steps.
