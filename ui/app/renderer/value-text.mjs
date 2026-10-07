import { formatReading, readViewPath } from './view-registry.mjs';
import { reasonText } from './reasons.mjs';
import { drawnPorts, recordPort } from './arrivals.mjs';

export const valueNotText = 'VALUE_NOT_TEXT';
const utf8 = new TextDecoder('utf-8', { fatal: true });
const record = value => value !== null && typeof value === 'object' && !Array.isArray(value)
  && !(value instanceof Uint8Array) && !(value instanceof Map);
const carrier = value => record(value) && Object.isFrozen(value) && typeof value.value === 'bigint'
  && Object.keys(value).length === 1;
const composite = value => (Array.isArray(value) && value.length > 0) || value instanceof Map && value.size > 0
  || record(value) && !carrier(value) && Object.keys(value).length > 0;

function shapeAt(shape, path) {
  let at = shape;
  for (const segment of path) {
    if (at?.kind === 'Object' && typeof segment === 'string') at = at.fields.find(field => field.name === segment)?.shape;
    else if (at?.kind === 'Array' && typeof segment === 'bigint') at = at.item;
    else return undefined;
  }
  return at;
}

function spell(value, shape, codes) {
  if (value === null) return 'null';
  if (typeof value === 'boolean') return String(value);
  if (typeof value === 'string') return value === '' ? 'Empty text' : value;
  if (typeof value === 'number' || typeof value === 'bigint' || carrier(value))
    return formatReading(value)?.full ?? String(carrier(value) ? value.value : value);
  if (value instanceof Uint8Array) {
    try { return spell(utf8.decode(value), shape, codes); }
    catch { codes.push(valueNotText); return `${value.length} ${value.length === 1 ? 'byte' : 'bytes'}`; }
  }
  const nested = (item, itemShape, inRecord) => {
    const text = spell(item, itemShape, codes);
    return composite(item) && !(inRecord && Array.isArray(item)) ? `(${text})` : text;
  };
  if (Array.isArray(value)) return value.length
    ? value.map(item => nested(item, shape?.kind === 'Array' ? shape.item : undefined, false)).join(', ') : 'Empty list';
  const entries = value instanceof Map ? [...value].map(([name, item]) => [String(name), item])
    : record(value) ? Object.entries(value) : null;
  if (!entries) return String(value);
  const declared = shape?.kind === 'Object' ? shape.fields : [];
  const names = [...new Set([...declared.map(field => field.name), ...entries.map(([name]) => name)])];
  const of = new Map(entries);
  const shown = names.filter(name => of.has(name) && of.get(name) !== undefined);
  return shown.length ? shown.map(name => `${name}: ${nested(of.get(name),
    declared.find(field => field.name === name)?.shape, true)}`).join(' · ') : 'Empty object';
}

export function valueText(value, { shape, fields } = {}) {
  if (value === undefined) return { text: reasonText('BODY_UNRECORDED'), code: 'BODY_UNRECORDED' };
  let shown = value, at = shape;
  if (fields?.value !== undefined) {
    const reading = readViewPath(value, fields.value);
    if (reading.code) return { text: reasonText(reading.code), code: reading.code };
    shown = reading.value;
    at = shapeAt(shape, fields.value);
  }
  const codes = [];
  const text = spell(shown, at, codes);
  return { text, code: codes[0] ?? null };
}

export function rowText(node, row, side = 'arrivals', fields = node.viewConfig?.fields) {
  const port = recordPort(node, row);
  const flow = drawnPorts(node, side === 'arrivals' ? 'in' : 'out')?.find(([id]) => id === port)?.[1];
  return valueText(row?.body, { shape: flow?.kind === 'Known' ? flow.flow.item : undefined, fields });
}
