import { PORT_BASE_SHAPES, encodePortFlow } from "../../protocol/src/port-type.js";

const plain = value => value !== null && typeof value === "object"
  && [Object.prototype, null].includes(Object.getPrototypeOf(value));
const base = name => ({ kind: "Base", base: name });
const stream = item => {
  try { return encodePortFlow({ kind: "Stream", item }); }
  catch (error) { throw Object.assign(new TypeError(error.message), { code: "authoring.prepass.config-not-literal" }); }
};

function lowerFormConfig(config) {
  if (!plain(config) || (!Array.isArray(config.fields) && !plain(config.fields))) {
    throw Object.assign(new TypeError("form.fields requires a name-to-BaseShape map or canonical array"),
      { code: "authoring.prepass.config-not-literal" });
  }
  if (Array.isArray(config.fields)) return config;
  const fields = Reflect.ownKeys(config.fields).map(name => {
    const field = Object.getOwnPropertyDescriptor(config.fields, name);
    if (typeof name !== "string" || !field.enumerable || !("value" in field)
      || !PORT_BASE_SHAPES.includes(field.value)) {
      throw Object.assign(new TypeError("form.fields entries require string names and a published BaseShape"),
        { code: "authoring.prepass.config-not-literal" });
    }
    return { name, shape: base(field.value) };
  });
  return { ...config, fields: stream({ kind: "Object", fields, open: false }) };
}

function lowerInputConfig(config) {
  if (!plain(config) || !Object.hasOwn(config, "shape")) return config;
  if (!PORT_BASE_SHAPES.includes(config.shape)) {
    throw Object.assign(new TypeError("input.shape requires a published BaseShape"),
      { code: "authoring.prepass.config-not-literal" });
  }
  return { ...config, shape: stream(base(config.shape)) };
}

const boundary = (name, keys, lower) => config => {
  if (!plain(config) || typeof config.topic !== "string" || !config.topic.length
    || Object.keys(config).some(key => !keys.includes(key))) {
    throw Object.assign(new TypeError(`${name} takes ${keys.join(" and ")}, with topic a non-empty string`),
      { code: "authoring.prepass.boundary-field-not-carried" });
  }
  const { topic, ...declared } = config;
  return lower({ label: topic, ...declared });
};

const LOWERINGS = Object.freeze({ form: lowerFormConfig, input: lowerInputConfig,
  project_input: boundary("projectInput", ["topic", "shape"], lowerInputConfig),
  project_output: boundary("projectOutput", ["topic"], config => config) });
export function lowerConfig(spelling, config) {
  return Object.hasOwn(LOWERINGS, spelling) ? LOWERINGS[spelling](config) : config;
}
