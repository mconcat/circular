import { surfaceValue } from './surface-value.js';
import { getExecutionContext } from '@circular/core/internal';

const SURFACE_MARK = Symbol.for("@circular/exports/mark");
const SURFACE_SELECTION = Symbol.for("@circular/exports/selection");
const SURFACE_STATE = Symbol.for("@circular/exports/state");
const STATIC_REFERENCE = Symbol.for("@circular/exports/static-reference");
const ROLE_REFERENCE = Symbol.for("@circular/exports/role-reference");
const EXPORT_DEFINITION = Symbol.for("@circular/exports/definition");
const EXPORT_INSTANCE = Symbol.for("@circular/exports/instance");

const EMPTY_OBJECT = Object.freeze({});
const ROLE_NAMES = Object.freeze(["request", "progress", "result", "error"]);
const ROLE_NAME_SET = new Set(ROLE_NAMES);
const OBSERVED_ROLE_SET = new Set(["progress", "result", "error"]);
const PARAMETER_TYPES = new Set(["number", "string", "boolean"]);

function hasOwn(value, key) {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function isObject(value) {
  return value !== null && typeof value === "object";
}

function isPlainObject(value) {
  if (!isObject(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function assertPlainObject(value, label) {
  if (!isPlainObject(value)) throw new TypeError(`${label} must be a plain object`);
  return value;
}

function assertClosedKeys(value, allowed, label) {
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      throw new TypeError(`${label} contains unsupported field ${JSON.stringify(key)}`);
    }
  }
}

function assertFiniteNumber(value, label) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    throw new TypeError(`${label} must be a finite number`);
  }
  return value;
}

function assertNonNegativeInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError(`${label} must be a non-negative safe integer`);
  }
  return value;
}

function assertPositiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new RangeError(`${label} must be a positive safe integer`);
  }
  return value;
}

function isStaticReference(value) {
  return isObject(value) && value[STATIC_REFERENCE] === true;
}

function assertStatic(value, predicate, label) {
  if (isStaticReference(value)) return value;
  if (!predicate(value)) throw new TypeError(`${label} has an invalid static value`);
  return value;
}

function assertStaticString(value, label) {
  return assertStatic(value, (candidate) => typeof candidate === "string", label);
}

function assertStaticPositiveInteger(value, label) {
  return assertStatic(value, (candidate) => Number.isSafeInteger(candidate) && candidate > 0, label);
}

function assertStaticPositiveNumber(value, label) {
  return assertStatic(
    value,
    (candidate) => typeof candidate === "number" && Number.isFinite(candidate) && candidate > 0,
    label,
  );
}

function immutableCopy(value, seen = new Map()) {
  if (value === null || typeof value === "string" || typeof value === "boolean") return value;
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new TypeError("structural values must contain finite numbers");
    return value;
  }
  if (isStaticReference(value) || isRoleReference(value) || isViewMark(value)) return value;
  if (typeof value !== "object") {
    throw new TypeError("structural values may not contain functions, symbols, bigint, or undefined");
  }
  if (seen.has(value)) throw new TypeError("structural values may not contain cycles");
  seen.set(value, true);
  if (Array.isArray(value)) {
    const result = value.map((item) => immutableCopy(item, seen));
    seen.delete(value);
    return Object.freeze(result);
  }
  assertPlainObject(value, "structural value");
  const result = {};
  for (const [key, item] of Object.entries(value)) {
    if (item === undefined) continue;
    result[key] = immutableCopy(item, seen);
  }
  seen.delete(value);
  return Object.freeze(result);
}

function isViewMark(value) {
  return isObject(value) && value[SURFACE_MARK] === true && typeof value.mark === "string";
}

function assertViewMark(value, label = "view mark") {
  if (!isViewMark(value)) throw new TypeError(`${label} must be a Circular view mark`);
  return value;
}

function createStaticReference(name) {
  const reference = { kind: "parameter", name };
  Object.defineProperties(reference, {
    [STATIC_REFERENCE]: { value: true },
    toJSON: {
      value() {
        return Object.freeze({ $circular: "parameter", name });
      },
    },
  });
  return Object.freeze(reference);
}

function isRoleReference(value) {
  return isObject(value) && value[ROLE_REFERENCE] === true && ROLE_NAME_SET.has(value.role);
}

function createRoleReference(role) {
  const reference = { role };
  Object.defineProperties(reference, {
    [ROLE_REFERENCE]: { value: true },
    toJSON: {
      value() {
        return Object.freeze({ $circular: "export-role", role });
      },
    },
  });
  return Object.freeze(reference);
}

function assertRoleReference(value, expected, label) {
  if (!isRoleReference(value)) throw new TypeError(`${label} must be an export role reference`);
  if (expected === "request" && value.role !== "request") {
    throw new TypeError(`${label} requires the writable request role`);
  }
  if (expected === "observed" && !OBSERVED_ROLE_SET.has(value.role)) {
    throw new TypeError(`${label} requires progress, result, or error`);
  }
  return value;
}

function descriptorOf(mark) {
  const state = mark[SURFACE_STATE];
  const descriptor = { mark: state.mark };
  if (state.spec !== undefined) descriptor.spec = state.spec;
  if (state.children !== undefined) descriptor.children = state.children;
  if (Object.keys(state.modifiers).length > 0) descriptor.modifiers = state.modifiers;
  return descriptor;
}

function replaceModifier(mark, name, value) {
  const state = mark[SURFACE_STATE];
  return createMark(state.mark, {
    spec: state.spec,
    children: state.children,
    modifiers: { ...state.modifiers, [name]: immutableCopy(value) },
  });
}

function createMark(kind, input = EMPTY_OBJECT) {
  const state = Object.freeze({
    mark: kind,
    spec: input.spec,
    children: input.children,
    modifiers: immutableCopy(input.modifiers ?? EMPTY_OBJECT),
  });
  const mark = { mark: kind };
  Object.defineProperties(mark, {
    [SURFACE_MARK]: { value: true },
    [SURFACE_SELECTION]: { value: true },
    [SURFACE_STATE]: { value: state },
    cell: {
      value(col, row, width, height) {
        return replaceModifier(mark, "cell", Object.freeze({
          col: assertNonNegativeInteger(col, "cell col"),
          row: assertNonNegativeInteger(row, "cell row"),
          width: assertPositiveInteger(width, "cell width"),
          height: assertPositiveInteger(height, "cell height"),
        }));
      },
    },
    toJSON: {
      value() {
        return descriptorOf(mark);
      },
    },
  });
  if (kind === "textInput") {
    Object.defineProperty(mark, "placeholder", {
      value(value) {
        assertStaticString(value, "text input placeholder");
        return replaceModifier(mark, "placeholder", value);
      },
    });
  }
  return Object.freeze(mark);
}

function validateGridSpec(input = EMPTY_OBJECT) {
  const spec = assertPlainObject(input, "grid spec");
  assertClosedKeys(spec, new Set(["cols", "row_h"]), "grid spec");
  const result = {};
  if (hasOwn(spec, "cols")) {
    assertStaticPositiveInteger(spec.cols, "grid cols");
    result.cols = spec.cols;
  }
  if (hasOwn(spec, "row_h")) {
    assertStaticPositiveNumber(spec.row_h, "grid row_h");
    result.row_h = spec.row_h;
  }
  return immutableCopy(result);
}

function validateBoundaryType(value, label) {
  if (value === "json" || value === "text") return value;
  if (!isPlainObject(value) || (value.kind !== "stream" && value.kind !== "signal")) {
    throw new TypeError(`${label} must be json, text, or a generated Flow`);
  }
  return immutableCopy(value);
}

function validateRoles(input = EMPTY_OBJECT) {
  const roles = assertPlainObject(input, "export roles");
  assertClosedKeys(roles, ROLE_NAME_SET, "export roles");
  const result = {};
  for (const role of ROLE_NAMES) {
    if (!hasOwn(roles, role)) continue;
    const declaration = assertPlainObject(roles[role], `export role ${role}`);
    assertClosedKeys(declaration, new Set(["type"]), `export role ${role}`);
    if (!hasOwn(declaration, "type")) throw new TypeError(`export role ${role} requires type`);
    result[role] = Object.freeze({ type: validateBoundaryType(declaration.type, `export role ${role} type`) });
  }
  return Object.freeze(result);
}

function roleReferences(roles) {
  const result = {};
  for (const role of ROLE_NAMES) {
    if (hasOwn(roles, role)) result[role] = createRoleReference(role);
  }
  return Object.freeze(result);
}

function validateParameterSchema(input = EMPTY_OBJECT) {
  const schema = assertPlainObject(input, "export parameter schema");
  const result = {};
  for (const [name, declarationInput] of Object.entries(schema)) {
    if (name.length === 0) throw new TypeError("export parameter names may not be empty");
    const declaration = assertPlainObject(declarationInput, `export parameter ${JSON.stringify(name)}`);
    assertClosedKeys(declaration, new Set(["type", "default"]), `export parameter ${JSON.stringify(name)}`);
    if (!PARAMETER_TYPES.has(declaration.type)) {
      throw new TypeError(`export parameter ${JSON.stringify(name)} has an unsupported type`);
    }
    const normalized = { type: declaration.type };
    if (hasOwn(declaration, "default")) {
      if (typeof declaration.default !== declaration.type) {
        throw new TypeError(`export parameter ${JSON.stringify(name)} default does not match its type`);
      }
      if (declaration.type === "number") assertFiniteNumber(declaration.default, `export parameter ${name} default`);
      normalized.default = declaration.default;
    }
    result[name] = Object.freeze(normalized);
  }
  return Object.freeze(result);
}

function parameterReferences(schema) {
  const result = {};
  for (const name of Object.keys(schema)) result[name] = createStaticReference(name);
  return Object.freeze(result);
}

function validateConfiguration(schema, input = EMPTY_OBJECT) {
  const config = assertPlainObject(input, "export configuration");
  const result = {};
  for (const [name, value] of Object.entries(config)) {
    const declaration = schema[name];
    if (declaration === undefined) {
      throw new TypeError(`export configuration contains unknown parameter ${JSON.stringify(name)}`);
    }
    if (typeof value !== declaration.type) {
      throw new TypeError(`export parameter ${JSON.stringify(name)} must be ${declaration.type}`);
    }
    if (declaration.type === "number") assertFiniteNumber(value, `export parameter ${name}`);
    result[name] = value;
  }
  return Object.freeze(result);
}

function assertTopLevelSurfaces(input) {
  if (!Array.isArray(input) || input.length === 0) {
    throw new TypeError("export surfaces must be a non-empty array");
  }
  input.forEach((surface, index) => {
    assertViewMark(surface, `export surface ${index}`);
    if (surface.mark !== "tab" && surface.mark !== "window") {
      throw new TypeError(`export surface ${index} must be a tab or window`);
    }
  });
  return Object.freeze([...input]);
}

function validateName(input) {
  if (typeof input !== "string") throw new TypeError("export mount name must be a string");
  const name = input.normalize("NFC");
  if (name.length === 0 || /[\u0000-\u001f\u007f]/u.test(name)) {
    throw new TypeError("export mount name must be a non-empty canonical text value");
  }
  return name;
}

export function tab(title, child) {
  assertStaticString(title, "tab title");
  return createMark("tab", { spec: immutableCopy({ title }), children: Object.freeze([assertViewMark(child)]) });
}

export function window(title, child) {
  assertStaticString(title, "window title");
  return createMark("window", { spec: immutableCopy({ title }), children: Object.freeze([assertViewMark(child)]) });
}

export function grid(specOrChildren, maybeChildren) {
  const firstIsChildren = Array.isArray(specOrChildren);
  const spec = firstIsChildren ? undefined : specOrChildren;
  const children = firstIsChildren ? specOrChildren : (maybeChildren ?? []);
  if (!Array.isArray(children)) throw new TypeError("grid children must be an array");
  children.forEach((child, index) => assertViewMark(child, `grid child ${index}`));
  return createMark("grid", {
    spec: validateGridSpec(spec),
    children: Object.freeze([...children]),
  });
}

export function messages(role) {
  return createMark("messages", { spec: immutableCopy({ role: assertRoleReference(role, "observed", "messages role") }) });
}

export function terminal(role) {
  return createMark("terminal", { spec: immutableCopy({ role: assertRoleReference(role, "observed", "terminal role") }) });
}

export function label(value) {
  assertStaticString(value, "label value");
  return createMark("label", { spec: immutableCopy({ value }) });
}

export function transcript(role, own) {
  const spec = { role: assertRoleReference(role, "observed", "transcript role") };
  if (own !== undefined) spec.own = assertRoleReference(own, "observed", "transcript own role");
  return createMark("transcript", { spec: immutableCopy(spec) });
}

export function textInput(role) {
  return createMark("textInput", { spec: immutableCopy({ role: assertRoleReference(role, "request", "text input role") }) });
}

export function button(role, specInput = EMPTY_OBJECT) {
  const reference = assertRoleReference(role, "request", "button role");
  const spec = assertPlainObject(specInput, "button spec");
  assertClosedKeys(spec, new Set(["label", "send"]), "button spec");
  const normalized = { role: reference };
  if (hasOwn(spec, "label")) {
    assertStaticString(spec.label, "button label");
    normalized.label = spec.label;
  }
  if (hasOwn(spec, "send")) {
    assertStaticString(spec.send, "button send");
    normalized.send = spec.send;
  }
  return createMark("button", { spec: immutableCopy(normalized) });
}

export function toggle(role, specInput = EMPTY_OBJECT) {
  const reference = assertRoleReference(role, "request", "toggle role");
  const spec = assertPlainObject(specInput, "toggle spec");
  assertClosedKeys(spec, new Set(["bind", "label"]), "toggle spec");
  const normalized = { role: reference };
  if (hasOwn(spec, "bind")) {
    if (typeof spec.bind !== "string") throw new TypeError("toggle bind must be a string");
    normalized.bind = spec.bind;
  }
  if (hasOwn(spec, "label")) {
    assertStaticString(spec.label, "toggle label");
    normalized.label = spec.label;
  }
  return createMark("toggle", { spec: immutableCopy(normalized) });
}

export function select(role, specInput) {
  const reference = assertRoleReference(role, "request", "select role");
  const spec = assertPlainObject(specInput, "select spec");
  assertClosedKeys(spec, new Set(["options", "label"]), "select spec");
  if (!Array.isArray(spec.options) || spec.options.length === 0 || spec.options.some((value) => typeof value !== "string")) {
    throw new TypeError("select options must be a non-empty string array");
  }
  if (new Set(spec.options).size !== spec.options.length) {
    throw new TypeError("select options must be unique");
  }
  const normalized = { role: reference, options: Object.freeze([...spec.options]) };
  if (hasOwn(spec, "label")) {
    assertStaticString(spec.label, "select label");
    normalized.label = spec.label;
  }
  return createMark("select", { spec: immutableCopy(normalized) });
}

export function prompt(role, placeholder) {
  const mark = textInput(role);
  return placeholder === undefined ? mark : mark.placeholder(placeholder);
}

export function composer(role, specInput = EMPTY_OBJECT) {
  const reference = assertRoleReference(role, "request", "composer role");
  const spec = assertPlainObject(specInput, "composer spec");
  assertClosedKeys(spec, new Set(["placeholder", "label"]), "composer spec");
  const normalized = { role: reference };
  for (const key of ["placeholder", "label"]) {
    if (!hasOwn(spec, key)) continue;
    assertStaticString(spec[key], `composer ${key}`);
    normalized[key] = spec[key];
  }
  return createMark("composer", { spec: immutableCopy(normalized) });
}

export function defineExport(specInput) {
  const spec = assertPlainObject(specInput, "export definition");
  assertClosedKeys(spec, new Set(["roles", "params", "operations", "surfaces"]), "export definition");
  if (!hasOwn(spec, "surfaces")) throw new TypeError("export definition requires surfaces");
  if (hasOwn(spec, "operations") && typeof spec.operations !== "string") {
    throw new TypeError("export operations must be a registered opaque declaration string");
  }
  const roles = validateRoles(spec.roles);
  const params = validateParameterSchema(spec.params);
  const roleRefs = roleReferences(roles);
  const parameterRefs = parameterReferences(params);
  const produced = typeof spec.surfaces === "function"
    ? spec.surfaces(roleRefs, parameterRefs)
    : spec.surfaces;
  const surfaces = assertTopLevelSurfaces(produced);
  const operations = spec.operations ?? null;

  const definition = function exportDefinition(config) {
    const normalizedConfig = validateConfiguration(params, config);
    const instance = { definition, config: normalizedConfig };
    Object.defineProperties(instance, {
      [EXPORT_INSTANCE]: { value: true },
      toJSON: {
        value() {
          return { $circular: "export-instance", config: normalizedConfig };
        },
      },
    });
    return Object.freeze(instance);
  };
  Object.defineProperties(definition, {
    roles: { value: roles, enumerable: true },
    params: { value: params, enumerable: true },
    surfaces: { value: surfaces, enumerable: true },
    operations: { value: operations },
    [EXPORT_DEFINITION]: { value: true },
    toJSON: {
      value() {
        return { $circular: "export-definition", roles, params, surfaces, operations };
      },
    },
  });
  return Object.freeze(definition);
}

export function surface(nameInput, definition) {
  const name = validateName(nameInput);
  if (typeof definition !== 'function' || definition[EXPORT_DEFINITION] !== true) {
    throw new TypeError('surface requires a defineExport definition');
  }
  const context = getExecutionContext();
  if (typeof context.declareExportSurface !== 'function') {
    throw new TypeError('surface requires a code execution host');
  }
  context.declareExportSurface({ name, surface: surfaceValue(definition) });
}
