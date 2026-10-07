import { exactPayloadPathIssue } from "../../protocol/src/payload-path.js";

const record = value => value !== null && typeof value === "object"
  && [Object.prototype, null].includes(Object.getPrototypeOf(value));

function invalid(path, expectation) {
  throw Object.assign(new TypeError(`view.config.${path}: ${expectation}`),
    { code: "CIRCULAR_VIEW_CONFIG_INVALID" });
}

function fieldPath(path, value) {
  const issue = exactPayloadPathIssue(value);
  if (issue) invalid(path, issue);
}

/**
 * The keys whose value is one closed String choice, each with its value space. The validator reads
 * this table; `view-config.d.ts` declares its literal type, and the `ViewVocabulary` member types
 * derive from that declaration. Which view reads a key is a matter for that view.
 */
export const VIEW_CHOICES = Object.freeze({
  side: Object.freeze(["emitted", "arrivals", "both"]),
  rows: Object.freeze(["latest", "outlets"]),
  spark: Object.freeze(["none", "samples"]),
});

/** The single validator for both vocabulary supply points; other view data stays opaque. */
export function validateViewConfig(config) {
  if (!record(config)) return;
  for (const key of ["heading", "count_label", "caption"]) {
    if (Object.hasOwn(config, key) && config[key] !== null && typeof config[key] !== "string") {
      invalid(key, "expected String or null");
    }
  }
  for (const [key, choices] of Object.entries(VIEW_CHOICES)) {
    if (Object.hasOwn(config, key) && config[key] !== null && !choices.includes(config[key])) {
      invalid(key, `expected one of ${choices.map(choice => JSON.stringify(choice)).join(", ")} or null`);
    }
  }
  if (Object.hasOwn(config, "columns") && config.columns !== null) {
    if (!Array.isArray(config.columns)) invalid("columns", "expected an array of { path, label } projections");
    const paths = [];
    for (const [index, column] of config.columns.entries()) {
      const at = `columns[${index}]`;
      if (!record(column)) invalid(at, "expected { path, label }");
      for (const key of Object.keys(column)) {
        if (key !== "path" && key !== "label") invalid(`${at}.${key}`, "unknown column key");
      }
      fieldPath(`${at}.path`, column.path);
      if (typeof column.label !== "string") invalid(`${at}.label`, "expected String");
      if (paths.some(path => path.length === column.path.length
        && path.every((segment, i) => segment === column.path[i]))) {
        invalid(`${at}.path`, "duplicate column path");
      }
      paths.push(column.path);
    }
  }
  if (Object.hasOwn(config, "total") && config.total !== null) {
    const total = config.total;
    if (!record(total)) invalid("total", "expected { outlet, path } or null");
    for (const key of Object.keys(total)) {
      if (key !== "outlet" && key !== "path") invalid(`total.${key}`, "unknown total key");
    }
    if (typeof total.outlet !== "string" || total.outlet.length === 0) invalid("total.outlet", "expected a non-empty outlet id");
    fieldPath("total.path", total.path);
  }
  if (Object.hasOwn(config, "fields") && config.fields !== null) {
    if (!record(config.fields)) invalid("fields", "expected a role-to-path record or null");
    for (const [role, path] of Object.entries(config.fields)) {
      if (!["title", "status", "value"].includes(role)) invalid(`fields.${role}`, "unknown field role");
      fieldPath(`fields.${role}`, path);
    }
  }
}

/** Null removes a top-level key; nested values replace whole, in declaration order of precedence. */
export function resolveViewConfig(actorConfig, typeDefault) {
  validateViewConfig(typeDefault);
  validateViewConfig(actorConfig);
  const resolved = { ...(record(typeDefault) ? typeDefault : {}), ...(record(actorConfig) ? actorConfig : {}) };
  for (const key of Object.keys(resolved)) if (resolved[key] === null) delete resolved[key];
  return resolved;
}
