/**
 * The only source-specific boundary in the Agent Session Monitor template.
 *
 * Today it accepts scrubbed OTLP/HTTP JSON posted to the daemon's OTLP ingress. Replacing that source with a
 * Loki/Grafana pull means replacing this file's `sourceAdapter()` topology and normalization
 * transforms while preserving the normalized `{signal, recognized, evidence}` output contract.
 * Nothing in graph.mjs reads an OTLP field or an agent/vendor field directly.
 */

import { deriveBoundaryPortId } from "@circular/protocol";

export const SIGNALS = Object.freeze(["logs", "metrics"]);

export const INGRESS_MOUNTS = Object.freeze({
  logs: "agent-otel-logs",
  metrics: "agent-otel-metrics",
});

/**
 * Attribute and text spellings accepted at the source boundary.
 *
 * The HTTP aliases and error/success fields are the only compatibility map. They cover the
 * measured payloads without making downstream classification know Claude or Codex.
 */
export const SOURCE_ADAPTER_RULES = Object.freeze({
  eventAttributeKeys: Object.freeze(["event.name"]),
  successAttributeKeys: Object.freeze(["success"]),
  httpStatusAttributeKeys: Object.freeze([
    "http.response.status_code",
    "http.status_code",
    "status_code",
  ]),
  errorAttributeKeys: Object.freeze(["error"]),
  errorAttributePrefixes: Object.freeze(["error."]),
  errorTextMarkers: Object.freeze(["error", "fail", "exception", "fatal"]),
  warningTextMarkers: Object.freeze(["warn", "retry", "throttle", "degraded"]),
  normalTextMarkers: Object.freeze(["request", "completed", "starts", "success"]),
  errorSeverityText: Object.freeze(["ERROR", "FATAL"]),
  warningSeverityText: Object.freeze(["WARN", "WARNING"]),
  normalSeverityText: Object.freeze(["TRACE", "DEBUG", "INFO", "NOTICE"]),
});

export const EVIDENCE_FIELDS = Object.freeze([
  "error",
  "warning",
  "normal",
]);

const celString = (value) => JSON.stringify(value);
const anyEqual = (expression, values) => values.length === 1
  ? `${expression} == ${celString(values[0])}`
  : `${expression} in [${values.map(celString).join(", ")}]`;
const anyContains = (expression, markers) => markers.map((marker) => `${expression}.contains(${celString(marker)})`).join(" || ");
const anyStartsWith = (expression, prefixes) => prefixes.map((prefix) => `${expression}.startsWith(${celString(prefix)})`).join(" || ");

const measuredLogsShape = [
  '"resourceLogs" in event && size(event.resourceLogs) == 1',
  '&& "scopeLogs" in event.resourceLogs[0] && size(event.resourceLogs[0].scopeLogs) == 1',
  '&& "logRecords" in event.resourceLogs[0].scopeLogs[0]',
].join(" ");
export const LOGS_RECOGNIZED_PREDICATE = `((${measuredLogsShape}) && size(event.resourceLogs[0].scopeLogs[0].logRecords) > 0)`;

function anyLogRecord(recordPredicate) {
  return `event.resourceLogs[0].scopeLogs[0].logRecords.exists(record, ${recordPredicate})`;
}

const hasKey = (keys) => `"key" in attribute && (${anyEqual("attribute.key", keys)})`;
const hasValueField = (field) => `"value" in attribute && !(attribute.value in [null]) && ${celString(field)} in attribute.value`;

function attributeBoolPredicate(keys, expected) {
  return `${hasKey(keys)} && ${hasValueField("boolValue")} && attribute.value.boolValue == ${expected}`;
}

function attributeStatusPredicate(minimum, maximum = null) {
  const upper = maximum === null ? "" : ` && attribute.value.intValue <= ${maximum}`;
  return `(${hasKey(SOURCE_ADAPTER_RULES.httpStatusAttributeKeys)}) && ${hasValueField("intValue")}`
    + ` && attribute.value.intValue >= ${minimum}${upper}`;
}

function attributeTextPredicate(markers) {
  return `(${hasKey(SOURCE_ADAPTER_RULES.eventAttributeKeys)}) && ${hasValueField("stringValue")}`
    + ` && (${anyContains("attribute.value.stringValue", markers)})`;
}

function severityNumberPredicate(minimum, maximum = null) {
  const upper = maximum === null ? "" : ` && record.severityNumber <= ${maximum}`;
  return `"severityNumber" in record && record.severityNumber >= ${minimum}${upper}`;
}

function severityTextPredicate(values) {
  return `"severityText" in record && (${anyEqual("record.severityText", values)})`;
}

function errorAttributePredicate() {
  const exact = anyEqual("attribute.key", SOURCE_ADAPTER_RULES.errorAttributeKeys);
  const prefixed = anyStartsWith("attribute.key", SOURCE_ADAPTER_RULES.errorAttributePrefixes);
  return `"key" in attribute && (${exact} || ${prefixed})`;
}

const recordSeverityEvidence = (minimum, maximum, texts) => anyLogRecord(
  `(${severityNumberPredicate(minimum, maximum)} || ${severityTextPredicate(texts)})`,
);

const recordAttributeEvidence = (predicates) => anyLogRecord(
  `("attributes" in record && record.attributes.exists(attribute, ${predicates.join(" || ")}))`,
);

const recordTextEvidence = (markers) => anyLogRecord(
  `(("attributes" in record && record.attributes.exists(attribute, ${attributeTextPredicate(markers)}))`
    + ` || ("body" in record && !(record.body in [null]) && "stringValue" in record.body && (${anyContains("record.body.stringValue", markers)})))`,
);

export const LOG_EVIDENCE_PREDICATES = Object.freeze({
  errorSeverity: recordSeverityEvidence(17, null, SOURCE_ADAPTER_RULES.errorSeverityText),
  errorAttribute: recordAttributeEvidence([errorAttributePredicate()]),
  errorOutcome: recordAttributeEvidence([
      attributeBoolPredicate(SOURCE_ADAPTER_RULES.successAttributeKeys, "false"),
      attributeStatusPredicate(400),
  ]),
  errorText: recordTextEvidence(SOURCE_ADAPTER_RULES.errorTextMarkers),
  warningSeverity: recordSeverityEvidence(13, 16, SOURCE_ADAPTER_RULES.warningSeverityText),
  warningStatus: recordAttributeEvidence([attributeStatusPredicate(300, 399)]),
  warningText: recordTextEvidence(SOURCE_ADAPTER_RULES.warningTextMarkers),
  normalSeverity: recordSeverityEvidence(1, 12, SOURCE_ADAPTER_RULES.normalSeverityText),
  normalOutcomeStatus: recordAttributeEvidence([
    attributeBoolPredicate(SOURCE_ADAPTER_RULES.successAttributeKeys, "true"),
    attributeStatusPredicate(200, 299),
  ]),
  normalOutcomeText: recordTextEvidence(SOURCE_ADAPTER_RULES.normalTextMarkers),
});

function logsDecisionTransform(classification, recognized = true) {
  const evidenceLiteral = EVIDENCE_FIELDS
    .map((name) => `${celString(name)}: ${name === classification}`)
    .join(", ");
  return `{'signal': 'logs', 'recognized': ${recognized}, 'evidence': {${evidenceLiteral}}}`;
}

/**
 * The measured metrics carry no trustworthy failure dimension. Expose that the batch was recognized,
 * but do not manufacture a normal result from the mere presence of a metric.
 */
export function metricsNormalizationTransform() {
  const evidenceLiteral = EVIDENCE_FIELDS.map((name) => `${celString(name)}: false`).join(", ");
  return `{'signal': 'metrics', 'recognized': (("resourceMetrics" in event) && size(event.resourceMetrics) > 0), 'evidence': {${evidenceLiteral}}}`;
}

/**
 * Source topology plus its normalization boundary. `graph.mjs` composes this with the generic
 * classifier and owns no source spelling.
 */
export function sourceAdapter({ actorKey }) {
  const entries = Object.fromEntries(SIGNALS.map((signal) => {
    const input = `input_${signal}`;
    const port = deriveBoundaryPortId("inlet", actorKey(input), 0n);
    return [signal, { input, parse: `parse_${signal}`, port }];
  }));
  const commonActors = SIGNALS.flatMap((signal) => [
    { id: entries[signal].input, actorType: "input", config: { label: INGRESS_MOUNTS[signal] } },
    { id: entries[signal].parse, actorType: "parse", config: { decoder: "json", field: "body", arguments: {} } },
  ]);
  const evidence = (names) => names.map(name => `(${LOG_EVIDENCE_PREDICATES[name]})`).join(" || ");
  const decisions = [
    ["error", ["errorSeverity", "errorAttribute", "errorOutcome", "errorText"]],
    ["warning", ["warningSeverity", "warningStatus", "warningText"]],
    ["normal", ["normalSeverity", "normalOutcomeStatus", "normalOutcomeText"]],
  ];
  const recognized = decisions.reduceRight((otherwise, [classification, names]) =>
    `((${evidence(names)}) ? ${logsDecisionTransform(classification)} : ${otherwise})`,
    logsDecisionTransform("unclassified"));
  const actors = [
    ...commonActors,
    { id: "normalize_logs", actorType: "map", config: {
      transform: `(${LOGS_RECOGNIZED_PREDICATE} ? ${recognized} : ${logsDecisionTransform("unclassified", false)})`,
    } },
    { id: "normalize_metrics", actorType: "map", config: { transform: metricsNormalizationTransform() } },
  ];
  const edges = SIGNALS.flatMap(signal => [
    { from: entries[signal].input, fromPort: entries[signal].port, to: entries[signal].parse, toPort: "event" },
    { from: entries[signal].parse, fromPort: "event", to: `normalize_${signal}`, toPort: "event" },
    { from: `normalize_${signal}`, fromPort: "event", to: `classify_${signal}`, toPort: "event" },
  ]);
  const mounts = SIGNALS.map((signal) => ({
    name: INGRESS_MOUNTS[signal],
    role: "request",
    actor: entries[signal].input,
    port: entries[signal].port,
  }));
  return Object.freeze({ entries: Object.freeze(entries), actors, edges, mounts });
}
