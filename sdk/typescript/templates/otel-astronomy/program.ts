/**
 * On-call for the OpenTelemetry Astronomy Shop: a demo pipeline.
 *
 * deploy.mjs fills in three values before it runs this program:
 *   $PROMETHEUS  the demo's Prometheus, for example http://127.0.0.1:9090
 *   $DEMO        the demo's web address, for its feature flag editor
 *   $TEMPLATE    this template's directory, where the action programs live
 *
 * The question both requests ask Prometheus, before URL encoding:
 *   sum(irate(traces_span_metrics_calls_total{service_name="checkout",
 *     span_name="oteldemo.CheckoutService/PlaceOrder",status_code="STATUS_CODE_ERROR"}[2m])) * 60
 *   or vector(0)
 * that is, how many orders failed per minute, from the last two samples.
 */
import { agent, alert, debounce, notify, request, timer, toolExecutor } from "@circular/core";

/** Watch: every 15 seconds, ask Prometheus how many orders are failing. */
export let poll = timer({ every: 15000n });
export let orders = request({
  method: "get",
  url: "$PROMETHEUS/api/v1/query?query=sum%28irate%28traces_span_metrics_calls_total%7Bservice_name%3D%22checkout%22%2Cspan_name%3D%22oteldemo.CheckoutService%2FPlaceOrder%22%2Cstatus_code%3D%22STATUS_CODE_ERROR%22%7D%5B2m%5D%29%29%20%2A%2060%20or%20vector%280%29",
  capabilities: { HttpFetch: { approval: "none" } },
});
poll.out.tick.into(orders.in.event);

/**
 * Alert: fires once failed orders have been seen for 10 seconds, and returns to ok after
 * two quiet minutes.
 */
export let failing = alert("event.failed_per_minute > 0.5", { firing_delay: 10000n, recovery_delay: 120000n });
orders.out.response
  .parse({ decoder: "json", field: "body" })
  .flatten({ at: ["data", "result"] })
  .map("{'failed_per_minute': double(event.data.result.value[1])}")
  .into(failing.in.event);

/** Triage: when the alert fires, an agent writes the first note for the on-call engineer. */
export let triage = agent({ harness: "claude", queue_capacity: 4n, result: "bytes" });
failing.out.transition
  .filter("event.from == 'Ok' && event.to == 'Firing'")
  .map("'The Astronomy Shop, the OpenTelemetry demo store, is failing orders: checkout requests end in errors. The demo injects its faults through feature flags. In at most three short lines, tell the on-call engineer what to check first. Do not use tools.'")
  .into(triage.in.turn);

/** The on-call channel: the triage note now, the verification outcome later. */
export let oncall = notify({
  channel: "oncall",
  minimum_interval: 0n,
  during_interval: "queue",
  capabilities: { UserNotify: { approval: "none" } },
});
triage.out.result
  .map("{'title': 'Astronomy Shop: orders failing', 'body': string(event)}")
  .into(oncall.in.notification);

/**
 * Actions: page the on-call engineer, or turn the demo's fault flags off. Turning flags off
 * waits for your approval.
 */
export let act = toolExecutor({
  capabilities: { ProcessSpawn: { approval: "none" } },
  tools: {
    page: { effect: "spawn", program: "$TEMPLATE/bin/page", arguments: [] },
    flag_off: { effect: "spawn", program: "$TEMPLATE/bin/flag-off", arguments: ["$DEMO", "-"], approval: "required" },
  },
});
failing.out.transition
  .filter("event.from == 'Ok' && event.to == 'Firing'")
  .map("{'id': b'rollback', 'tool': 'flag_off', 'arguments': b''}")
  .into(act.in.call);

/**
 * Verification: once an action has been quiet for 90 seconds, ask Prometheus the same
 * question again.
 */
export let settle = debounce({ quiet_window: 90000n });
act.out.result.into(settle.in.event);
export let recheck = request({
  method: "get",
  url: "$PROMETHEUS/api/v1/query?query=sum%28irate%28traces_span_metrics_calls_total%7Bservice_name%3D%22checkout%22%2Cspan_name%3D%22oteldemo.CheckoutService%2FPlaceOrder%22%2Cstatus_code%3D%22STATUS_CODE_ERROR%22%7D%5B2m%5D%29%29%20%2A%2060%20or%20vector%280%29",
  capabilities: { HttpFetch: { approval: "none" } },
});
settle.out.event.into(recheck.in.event);
export let verdict = recheck.out.response
  .parse({ decoder: "json", field: "body" })
  .flatten({ at: ["data", "result"] })
  .map("{'failed_per_minute': double(event.data.result.value[1]), 'resolved': double(event.data.result.value[1]) <= 0.5}")
  .tap();

/** Resolved: say so on the on-call channel. */
verdict.out.event
  .filter("event.resolved")
  .map("{'title': 'Astronomy Shop: orders recovered', 'body': 'The re-check found ' + string(int(event.failed_per_minute)) + ' failed orders per minute.'}")
  .into(oncall.in.notification);

/**
 * Not resolved: escalate with a page. The page's result settles like any action result, so
 * the same re-check runs again. This is the loop's back edge.
 */
verdict.out.event
  .filter("!event.resolved")
  .map("{'id': b'escalate', 'tool': 'page', 'arguments': bytes('Orders still failing after the action: ' + string(int(event.failed_per_minute)) + ' per minute')}")
  .into(act.in.call);
