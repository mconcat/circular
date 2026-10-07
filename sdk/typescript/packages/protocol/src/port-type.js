/**
 * The closed, lossless port Flow/Shape carrier — the TypeScript spelling of
 * `crates/protocol/src/port_type.rs` (`decode_port_shape` · `decode_port_flow`).
 *
 * This is the one TypeScript reader of that carrier. `authoring.actor-ports`
 * answers each port with it, the authoring type hints render from its decoded
 * value, and a view reads the same decoded value. Nothing else re-reads the tags.
 * It is also the one TypeScript writer (`encodePortFlow`): the SDK's
 * config lowering and the canvas's type controls write a type expression through it.
 *
 * Decoding does not judge connectability: that is the daemon's Shape layer. It
 * only refuses what the Rust decoder refuses, with the same ceilings.
 */
import { BaseShape } from './internal/closed-tables.js';

const PORT_SHAPE_MAX_DEPTH = 16;
/** Canonical object-shape width ceiling (Rust `PORT_OBJECT_MAX_FIELDS`). */
const PORT_OBJECT_MAX_FIELDS = 64;
/** Rust `circular_core::BaseShape` spellings, from `@circular/protocol/tables`. */
export const PORT_BASE_SHAPES = BaseShape;
const BASE_SHAPES = new Set(PORT_BASE_SHAPES);

function fail(detail) { throw new TypeError(`port-type: ${detail}`); }
const isPlainObject = value => value !== null && typeof value === 'object' && !Array.isArray(value)
  && !(value instanceof Uint8Array);

function shapeAt(value, depth) {
  if (depth > PORT_SHAPE_MAX_DEPTH) fail(`port shape depth exceeds ${PORT_SHAPE_MAX_DEPTH}`);
  if (!Array.isArray(value)) fail('port shape is not an Array');
  const [tag] = value;
  if (tag === 1n && value.length === 1) return Object.freeze({ kind: 'Any' });
  if (tag === 2n && value.length === 2 && typeof value[1] === 'string') {
    if (!BASE_SHAPES.has(value[1])) fail(`port shape has unknown base ${JSON.stringify(value[1])}`);
    return Object.freeze({ kind: 'Base', base: value[1] });
  }
  if (tag === 3n && value.length === 2) return Object.freeze({ kind: 'Array', item: shapeAt(value[1], depth + 1) });
  if (tag === 4n && value.length === 3 && Array.isArray(value[1]) && typeof value[2] === 'boolean') {
    if (value[1].length > PORT_OBJECT_MAX_FIELDS) {
      fail(`port object shape has ${value[1].length} fields; maximum is ${PORT_OBJECT_MAX_FIELDS}`);
    }
    const seen = new Set();
    const fields = value[1].map(raw => {
      if (!isPlainObject(raw)) fail('port shape field is not an Object');
      const extra = Object.keys(raw).find(key => key !== 'name' && key !== 'shape');
      if (extra !== undefined) fail(`port shape field carries unknown field ${JSON.stringify(extra)}`);
      if (!Object.hasOwn(raw, 'name')) fail('port shape field has no "name"');
      if (typeof raw.name !== 'string') fail('field name is not a String');
      if (raw.name.length === 0) fail('port object shape has an empty field name');
      if (!Object.hasOwn(raw, 'shape')) fail('port shape field has no "shape"');
      const shape = shapeAt(raw.shape, depth + 1);
      if (seen.has(raw.name)) fail(`port object shape repeats field ${JSON.stringify(raw.name)}`);
      seen.add(raw.name);
      return Object.freeze({ name: raw.name, shape });
    });
    return Object.freeze({ kind: 'Object', fields: Object.freeze(fields), open: value[2] });
  }
  if (tag === 5n && value.length === 2 && typeof value[1] === 'string' && value[1].length > 0) {
    return Object.freeze({ kind: 'Variable', name: value[1] });
  }
  return fail('port shape has an unknown closed arm');
}

/** Rust `decode_port_shape`. */
export function decodePortShape(value) { return shapeAt(value, 0); }

function decodePortRate(value) {
  if (!Array.isArray(value)) fail('port rate is not an Array');
  if (value.length === 2 && value[0] === 1n && typeof value[1] === 'bigint' && value[1] > 0n) {
    return Object.freeze({ kind: 'Period', ticks: value[1] });
  }
  if (value.length === 2 && value[0] === 2n && typeof value[1] === 'string') {
    return Object.freeze({ kind: 'Variable', name: value[1] });
  }
  return fail('port rate has an unknown or zero arm');
}

/** Rust `decode_port_flow`. */
export function decodePortFlow(value) {
  if (!Array.isArray(value)) fail('port flow is not an Array');
  if (value.length === 2 && value[0] === 1n) return Object.freeze({ kind: 'Stream', item: decodePortShape(value[1]) });
  if (value.length === 3 && value[0] === 2n) {
    return Object.freeze({ kind: 'Signal', item: decodePortShape(value[1]), rate: decodePortRate(value[2]) });
  }
  return fail('port flow has an unknown closed arm');
}

function encodeShape(shape) {
  switch (shape?.kind) {
    case 'Any': return [1n];
    case 'Base': return [2n, shape.base];
    case 'Array': return [3n, encodeShape(shape.item)];
    case 'Object': return [4n, shape.fields.map(field => ({ name: field.name, shape: encodeShape(field.shape) })), shape.open];
    case 'Variable': return [5n, shape.name];
    default: return fail('port shape has an unknown kind');
  }
}

function encodeRate(rate) {
  if (rate?.kind === 'Period') return [1n, rate.ticks];
  if (rate?.kind === 'Variable') return [2n, rate.name];
  return fail('port rate has an unknown kind');
}

/**
 * Rust `encode_port_flow`: a decoded Flow as its wire carrier. Like the Rust encoder it refuses a
 * value its decoder would refuse — the value written is read back by `decodePortFlow` before it is
 * returned, so the two directions keep one set of rules and ceilings.
 */
export function encodePortFlow(flow) {
  const value = flow?.kind === 'Stream' ? [1n, encodeShape(flow.item)]
    : flow?.kind === 'Signal' ? [2n, encodeShape(flow.item), encodeRate(flow.rate)]
      : fail('port flow has an unknown kind');
  decodePortFlow(value);
  return value;
}

/**
 * The `flow` member of one `authoring.actor-ports` port row — the app reader's
 * `PortFlowAvailability`. `[1, PortFlow]` is
 * a known Flow; `[2, reason]` is the producer's non-empty reason, carried as a
 * value rather than dropped.
 */
export function portFlowAvailabilityFromValue(value) {
  if (!Array.isArray(value) || value.length !== 2) fail('port Flow availability is not a two-member Array');
  if (value[0] === 1n) return Object.freeze({ kind: 'Known', flow: decodePortFlow(value[1]) });
  if (value[0] === 2n && typeof value[1] === 'string' && value[1].length > 0) {
    return Object.freeze({ kind: 'Unavailable', reason: value[1] });
  }
  return fail('port Flow availability has an unknown arm');
}
