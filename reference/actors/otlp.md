# otlp

A source boundary between the pipeline and a loopback-only OTLP/HTTP JSON receiver. Point
an OpenTelemetry exporter at it and the logs and metrics it accepts come out on two
separate outlets.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Outlet | `logs` | Open object: one scrubbed fragment, preserving the resource and scope structure | Wire by name |
| Outlet | `metrics` | Open object: one scrubbed fragment, preserving the resource and scope structure | Wire by name |

There are **no inlets**, there is no `_error` outlet, and there are no dynamic ports.
**Neither outlet is primary**, so both must be connected by name.

Raw request bodies are not kept in the recorded arrivals. No envelope keys and no status
outlet are added.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `listen` | String | **Yes — no default** | The loopback address and port to receive telemetry on, in numeric form such as 127.0.0.1:4318. Either an IPv4 loopback address or `[::1]`, with a port from 1 to 65535. |

A hostname such as `localhost`, a wildcard or non-loopback address, and a port of 0 or out
of range are all refused. IPv4 loopback is not narrowed to `127.0.0.1` alone. `4318` is a
common example port, not a default. No other configuration key is accepted — in particular
the mount name, a bearer token, an ingress URL and scan roots do not live here.

The create-input schema only checks that a required string is present. The address and port
are checked before the change commits, with the same parser the receiver runs when it is
activated. A committed address can still fail to bind.

### Body limits

These are current constants, not configurable slots:

| Path | Body limit | Over the limit |
| --- | --- | --- |
| The daemon's general webhook mount | 1 MiB (1,048,576 bytes) | 413 |
| This actor's dedicated OTLP receiver | 16 MiB (16,777,216 bytes) | 413 |
| A fragment after an accepted batch is split | 900 KiB (921,600 bytes) | 413, if one item cannot be split to fit |

Being under the limit is not sufficient: the receiver also checks the client’s loopback
address and the request shape. A recommended batch size in items is **not specified**,
because the byte size depends on the signal content.

## State

The OTLP receiver runs as this actor's worker. The worker holds the socket, undelivered
scrubbed batches and refusal counters. Worker failures are recorded in this actor's health.

Recovery after a restart is the replay of recorded arrivals plus the hand-over of the
undelivered durable queue. The custody log in the actor’s private directory appends
records during normal operation. Once settled records reach the queue’s capacity,
compaction replaces the log with one folded snapshot.

## Rejections

| Code / name | When |
| --- | --- |
| `ConfigRejected` | `listen` missing, not a string, not a numeric loopback address with a non-zero port, or an extra configuration key is present. The actor does not start. |
| Bind failure | The loopback bind failed. It is diagnosed and counted; the actor does not announce that it can receive. |

The receiver handles HTTP-level refusals without emitting them on the data outlets:

| Response | Cause |
| --- | --- |
| 403 | A non-loopback client. |
| 404 / 405 | Unknown path / a method other than POST. |
| 400 | Malformed HTTP, malformed JSON, or an unsupported signal shape. |
| 408 | The request timed out. |
| 411 | No `Content-Length`. |
| 413 | Body over the limit, or one item that cannot be split to fit. |
| 415 | A `Content-Type` other than `application/json`, or compression / transfer encoding. |
| 431 | Headers over the limit. |
| 503 | The queue is full. The receiver checks capacity for all splits before enqueueing the batch. The answer carries `Retry-After`: `1` for the first three refusals, `5` for the next three, then `15` while the queue stays full; after the queue has drained, the next refusal says `1` again. A sender that keeps the longer of its own backoff and this value stops knocking every second at a receiver that is as good as blocked. |

A normal batch returns 200 after durable acceptance. A scrub failure drops the affected
signal items and records those drops; the receiver can accept the remaining items in the
batch. A valid but empty batch also returns 200, is counted and emits nothing.

An absent repeated field is not a malformed signal — the JSON mapping omits empty repeated
fields, so `{}` and a zero-item batch are valid and return 200. The unsupported-shape 400
is for genuinely wrong JSON types, another signal's root key, or extra keys.

## Example

```ts
import { otlp } from "@circular/core";

export const telemetry = otlp({ listen: "127.0.0.1:4318" });

export const logs = telemetry.out.logs;
export const metrics = telemetry.out.metrics;
```
