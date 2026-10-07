import { inSpace } from './reasons.mjs';
import { actorCreateInputs } from '@circular/client';
import { CircularUInt, PORT_BASE_SHAPES, decodePortShape, decodePortFlow, encodePortFlow } from '@circular/protocol';
import { decoded } from './session.mjs';
import { portShape } from './scene.mjs';
import { durationMs, durationUnitName } from './view-registry.mjs';
const record = value => value && [Object.prototype, null].includes(Object.getPrototypeOf(value));
export const noEditor = 'CONFIG_EDITOR_UNAVAILABLE';
export const anyKinds = ['string', 'integer', 'number', 'boolean', 'object', 'list'];
const bases = { int: 'integer', uint: 'integer', float: 'number', bool: 'boolean', string: 'string' };
const scalar = value => ({ bigint: 'integer', number: 'number', boolean: 'boolean', string: 'string' })[typeof value];
const uint = value => value instanceof CircularUInt;
const plain = value => record(value) && !uint(value);
const ANY = Object.freeze({ kind: 'Any' });

export async function readCreateInputs(session) {
  try {
    const result = await actorCreateInputs(session);
    if (result.status !== 'accepted') return { entries: new Map(), diagnostic: inSpace('Query', result.diagnostics?.[0]?.code ?? result.diagnostic?.code) ?? 'READ_UNAVAILABLE' };
    const entries = new Map();
    const slotOf = async slot => ({
      path: slot.path, key: slot.path.length === 1 && slot.path[0][0] === 1n ? slot.path[0][1] : null,
      label: slot.label, description: slot.description, group: slot.group,
      shape: decoded(() => decodePortShape(slot.shape)), constraint: slot.constraint ?? null,
      required: slot.requirement[0] === 1n, ...(slot.requirement[0] === 2n ? { fallback: slot.requirement[1] } : {}),
      ...(slot.requirement[0] === 3n ? { absent: decoded(() => decodePortFlow(slot.requirement[1])) } : {}),
      snippet: slot.snippet ?? null,
      ...(slot.policies ? { policies: await Promise.all(Object.entries(slot.policies).map(async ([name, inputs]) =>
        ({ name, inputs: await Promise.all(inputs.map(slotOf)) }))) } : {}),
    });
    for (const { actor_type, state } of result.value.items) {
      if (state[0] === 1n) entries.set(actor_type, { slots: [] });
      else if (state[0] === 3n) entries.set(actor_type, { code: state[1] });
      else entries.set(actor_type, { slots: await Promise.all(state[1].slots.map(slotOf)), draft: state[2] });
    }
    return { entries, diagnostic: null };
  } catch (error) {
    return { entries: new Map(), diagnostic: error.code ?? 'READ_UNAVAILABLE' };
  }
}

const leafKind = (shape, value) => shape.kind === 'Base' ? bases[shape.base] : uint(value) ? undefined : value === null ? 'string' : scalar(value);
const structuredKind = (shape, value) => shape.kind === 'Array' || shape.kind === 'Object'
  || (shape.kind !== 'Base' && (Array.isArray(value) || plain(value)));
const itemShape = shape => shape.kind === 'Array' ? shape.item : ANY;
const fieldShape = (shape, key) => shape.kind === 'Object' ? shape.fields.find(f => f.name === key)?.shape ?? ANY : ANY;
const openObject = shape => shape.kind !== 'Object' || shape.open;
export const leafName = path => JSON.stringify(path.map(segment => typeof segment === 'number' ? segment : String(segment)));
const shown = value => typeof value === 'bigint' ? String(value) : uint(value) ? String(value.value) : value === null ? '' : value;

function structureRows(shape, value, path, depth = 0) {
  const rows = [];
  if (Array.isArray(value)) {
    value.forEach((item, i) => {
      const [first, ...members] = valueRows(itemShape(shape), item, [...path, i], depth, String(i));
      rows.push({ ...first, item: true }, ...members);
    });
    rows.push({ row: 'insert', target: leafName(path), label: 'item', depth });
  } else {
    const declared = shape.kind === 'Object' ? shape.fields.map(f => f.name) : [];
    for (const [key, child] of Object.entries(value ?? {}))
      rows.push(...valueRows(fieldShape(shape, key), child, [...path, key], depth, key, !declared.includes(key)));
    for (const key of declared.filter(key => !Object.hasOwn(value ?? {}, key)))
      rows.push({ row: 'insert', target: leafName(path), key, label: key, depth });
    if (openObject(shape)) rows.push({ row: 'insert', target: leafName(path), label: 'entry', keyName: leafName([...path, '']), depth });
  }
  return rows;
}
const heldKind = (shape, value) => Array.isArray(value) ? 'list' : plain(value) ? 'object' : leafKind(shape, value);
const anyOf = (shape, value, path) => {
  const held = shape.kind === 'Any' ? heldKind(shape, value) : undefined;
  return held ? { retype: leafName(path), held } : {};
};
function valueRows(shape, value, path, depth, label, removable = true) {
  const remove = removable ? leafName(path) : undefined;
  if (structuredKind(shape, value)) return [{ row: 'group', label, depth, remove, ...anyOf(shape, value, path) },
    ...structureRows(shape, Array.isArray(value) || plain(value) ? value : shape.kind === 'Array' ? [] : {}, path, depth + 1)];
  const kind = leafKind(shape, value);
  return [kind ? { row: 'leaf', name: leafName(path), label, kind, value: shown(value), depth, remove, ...anyOf(shape, value, path) }
    : { row: 'fixed', label, value, depth, remove, code: noEditor }];
}

const tagOf = slot => slot.constraint ? Number(slot.constraint[0]) : 0;
const choices = slot => tagOf(slot) === 5 ? [...slot.constraint[1]] : undefined;
const stringList = shape => shape.kind === 'Array' && shape.item.kind === 'Base' && shape.item.base === 'string';
const payloadPath = 'payload path — text items are object keys, whole numbers ≥ 0 are list indices, no items is the whole payload';
const SLOT_TEXT = ['label', 'description', 'group'];
const slotText = slot => Object.fromEntries(SLOT_TEXT.filter(name => typeof slot[name] === 'string').map(name => [name, slot[name]]));
export function slotNotes(slot) {
  if (!slot || !('required' in slot)) return {};
  const c = slot.constraint, tag = tagOf(slot), options = choices(slot), defaulted = !slot.required && 'fallback' in slot;
  const absent = 'absent' in slot ? portShape({ kind: 'Known', flow: slot.absent }) : undefined;
  const words = [
    tag === 2 && (c[2] == null ? `whole number ≥ ${c[1]}` : `whole number ${c[1]}–${c[2]}`),
    tag === 3 && 'finite number', tag === 4 && 'between 0 and 1',
    tag === 6 && c[1].replaceAll('_', ' '), tag === 7 && 'type expression', tag === 8 && payloadPath,
    tag === 9 && 'one base type', absent !== undefined && `blank means ${absent}`,
    slot.snippet && `CEL ${slot.snippet.mode} · reads ${slot.snippet.inlets.join(', ')}`,
  ].filter(Boolean);
  return { ...slotText(slot), ...(options ? { options } : {}), ...(tag === 1 ? { duration: true } : {}),
    ...(tag === 10 ? { durations: true } : {}), ...(tag === 6 ? { domain: c[1] } : {}),
    ...(defaulted ? { fallback: slot.fallback } : {}),
    ...(absent !== undefined ? { absent } : {}),
    ...(slot.snippet || tag === 7 || tag === 8 ? { expression: true } : {}),
    hint: words.join(' · '), required: slot.required };
}
export const GRANT_APPROVAL = Object.freeze({ key: 'approval', all: 'required', none: 'none' });
function grantInput(key, policy, input, value) {
  const name = input.path.at(-1)[1];
  const held = value?.[name];
  const { options, ...notes } = slotNotes(input);
  const kind = options ? 'choice' : stringList(input.shape) ? 'lines' : leafKind(input.shape, held) ?? 'string';
  const approval = name === GRANT_APPROVAL.key && [GRANT_APPROVAL.all, GRANT_APPROVAL.none].every(v => options?.includes(v));
  return { key: name, name: leafName([key, policy, name]), kind, options, shape: input.shape, ...(approval ? { approval } : {}),
    value: kind === 'lines' ? (Array.isArray(held) ? held.join('\n') : '') : held === undefined ? '' : shown(held), ...notes };
}

function typeFields(value) {
  if (value === undefined) return [];
  try {
    const flow = decodePortFlow(value);
    if (flow.kind === 'Stream' && flow.item.kind === 'Object' && !flow.item.open
      && flow.item.fields.every(f => f.shape.kind === 'Base'))
      return flow.item.fields;
  } catch {   }
}
const recordType = fields => encodePortFlow({ kind: 'Stream', item: { kind: 'Object', fields, open: false } });
function namedFields(fields) {
  try { return recordType(fields); }
  catch (error) { throw Object.assign(new TypeError(error.message), { code: 'FIELD_NAME_INVALID' }); }
}
const baseField = (name, base) => ({ name, shape: { kind: 'Base', base } });
const baseStream = base => encodePortFlow({ kind: 'Stream', item: { kind: 'Base', base } });
function streamBase(value) {
  try {
    const flow = decodePortFlow(value);
    if (flow.kind === 'Stream' && flow.item.kind === 'Base') return flow.item.base;
  } catch {   }
}

export function portDraft(entry, availability) {
  const flow = availability?.kind === 'Known' ? availability.flow : undefined;
  if (flow?.kind !== 'Stream' || flow.item.kind !== 'Base') return {};
  return Object.fromEntries((entry?.slots ?? []).filter(slot => slot.key !== null && tagOf(slot) === 9)
    .map(slot => [slot.key, baseStream(flow.item.base)]));
}

export function configFieldList(config, declared, entry, readCode = 'READ_UNAVAILABLE') {
  if (!declared) return { declared: false, fields: [] };
  if (!entry || entry.code) return { declared: true, fields: [], code: entry?.code ?? readCode };
  if (config != null && !plain(config)) return { declared: true, fields: [], code: noEditor };
  return { declared: true, fields: entry.slots.filter(slot => slot.key !== null).map(slot => {
    const { key, shape } = slot, value = config?.[key], { options, ...notes } = slotNotes(slot);
    if (slot.policies) return { key, kind: 'grants', shape, ...notes, policies: slot.policies.map(policy => ({
      name: policy.name, grant: leafName([key, policy.name]), granted: Boolean(plain(value?.[policy.name])),
      inputs: policy.inputs.map(input => grantInput(key, policy.name, input, plain(value?.[policy.name]) ? value[policy.name] : undefined)) })) };
    if (tagOf(slot) === 7) {
      const fields = typeFields(value);
      if (fields) return { key, kind: 'type-fields', shape, ...notes, options: PORT_BASE_SHAPES,
        insert: leafName([key, 1, 1]), rows: fields.map((field, i) => ({
          name: field.name, base: field.shape.base, nameInput: leafName([key, 1, 1, i, 'name']),
          typeInput: leafName([key, 1, 1, i, 'shape', 1]), remove: leafName([key, 1, 1, i]),
        })) };
      notes.hint += ' · This type expression cannot be shown as a field list; use the value editor.';
    }
    if (tagOf(slot) === 9) {
      const base = value === undefined ? '' : streamBase(value);
      if (base !== undefined) return { key, kind: 'base-stream', shape, ...notes, options: PORT_BASE_SHAPES, base };
      notes.hint += ' · This type cannot be shown as one base type; use the value editor.';
    }
    if (options && (value === undefined || typeof value === 'string')) return { key, kind: 'choice', shape, options, ...notes };
    if (structuredKind(shape, value)) {
      const rows = structureRows(shape, Array.isArray(value) || plain(value) ? value : shape.kind === 'Array' ? [] : {}, [key]);
      return { key, kind: 'structured', shape, ...notes, ...anyOf(shape, value, [key]),
        rows: notes.durations ? rows.map(row => row.item && row.row === 'leaf' ? { ...row, duration: true } : row) : rows };
    }
    const kind = value === undefined && shape.kind !== 'Base' ? 'string' : leafKind(shape, value);
    return kind ? { key, kind, shape, ...notes, ...(shape.kind === 'Any' ? { retype: leafName([key]), held: kind } : {}) }
      : { key, kind: 'fixed', shape, code: noEditor, ...notes };
  }) };
}

const invalid = () => Object.assign(new Error(), { code: 'CONFIG_VALUE_INVALID' });
const unsigned = shape => shape?.kind === 'Base' && shape.base === 'uint';
function readLeaf(kind, input, shape) {
  if (unsigned(shape)) { if (!/^\d+$/.test(input)) throw invalid(); try { return new CircularUInt(BigInt(input)); } catch { throw invalid(); } }
  if (kind === 'integer') { if (!/^-?\d+$/.test(input)) throw invalid(); return BigInt(input); }
  if (kind === 'number') { const n = Number(input); if (!input.trim() || !Number.isFinite(n)) throw invalid(); return n; }
  if (kind === 'boolean') { if (!['true', 'false'].includes(input)) throw invalid(); return input === 'true'; }
  return input;
}
const unchanged = (text, value) => text === String(shown(value));
function durationText(form, name) {
  const text = form.elements.namedItem(name)?.value;
  if (text === undefined) return undefined;
  const ms = durationMs(text, form.elements.namedItem(durationUnitName(name))?.value);
  if (ms === null) throw invalid();
  return ms;
}

export function readConfigForm(config, form, fields) {
  const leaf = (shape, value, name, duration) => {
    const field = form.elements.namedItem(name), kind = leafKind(shape, value);
    if (!field || !kind) return value;
    const text = duration ? durationText(form, name) : field.value;
    return unchanged(text, value) ? value : readLeaf(kind, text, shape);
  };
  const walk = (shape, value, path, durations = false) => {
    if (Array.isArray(value)) return value.map((item, i) => walk(itemShape(shape), item, [...path, i], durations));
    if (plain(value) && structuredKind(shape, value))
      return Object.setPrototypeOf(Object.fromEntries(Object.entries(value)
        .map(([key, child]) => [key, walk(fieldShape(shape, key), child, [...path, key])])), Object.getPrototypeOf(value));
    return leaf(shape, value, leafName(path), durations);
  };
  const next = { ...config };
  for (const { key, kind, shape, policies, rows, required, duration, durations } of fields) {
    if (kind === 'base-stream') {
      const chosen = form.elements.namedItem(key)?.value;
      if (chosen === '') delete next[key];
      else if (chosen !== undefined) next[key] = baseStream(chosen);
      continue;
    }
    if (kind === 'type-fields') {
      if (required || key in next || rows.length) next[key] = namedFields(rows.map(row => baseField(
        form.elements.namedItem(row.nameInput)?.value ?? row.name,
        form.elements.namedItem(row.typeInput)?.value ?? row.base)));
      continue;
    }
    if (kind === 'structured') { if (key in next) next[key] = walk(shape, next[key], [key], durations); continue; }
    if (kind === 'grants') { readGrants(next, key, policies, form); continue; }
    if (kind === 'choice' && form.elements.namedItem(key)?.value === '') { delete next[key]; continue; }
    const field = form.elements.namedItem(key);
    if (!field || kind === 'fixed') continue;
    const text = duration ? durationText(form, key) : field.value;
    if (!(key in next)) { if (text !== '') next[key] = readLeaf(kind, text, shape); continue; }
    if (!unchanged(text, next[key])) next[key] = readLeaf(kind, text, shape);
  }
  return Object.setPrototypeOf(next, Object.getPrototypeOf(config ?? {}));
}

function readGrants(next, key, policies, form) {
  const grants = plain(next[key]) ? { ...next[key] } : {};
  let drawn = false;
  for (const policy of policies) {
    const grant = form.elements.namedItem(policy.grant)?.value;
    if (grant === undefined) continue;
    drawn = true;
    if (grant !== 'granted') { delete grants[policy.name]; continue; }
    const was = plain(grants[policy.name]) ? grants[policy.name] : {}, out = { ...was };
    for (const input of policy.inputs) {
      const text = input.duration ? durationText(form, input.name) : form.elements.namedItem(input.name)?.value;
      if (text === undefined) continue;
      if (input.kind === 'lines') {
        const lines = text.split('\n').map(line => line.trim()).filter(Boolean);
        if (lines.length) out[input.key] = lines; else delete out[input.key];
      } else if (text === '') delete out[input.key];
      else if (!(input.key in was) || !unchanged(text, was[input.key])) out[input.key] = readLeaf(input.kind, text, input.shape);
    }
    grants[policy.name] = out;
  }
  if (drawn && (Object.keys(grants).length || key in next)) next[key] = grants;
}

const fresh = shape => shape.kind === 'Array' ? [] : shape.kind === 'Object' ? {}
  : shape.kind === 'Base' ? ({ int: 0n, uint: new CircularUInt(0n), float: 0, bool: false, string: '', null: null, bytes: new Uint8Array() })[shape.base] : '';
function retyped(value, kind) {
  if (kind === 'object') return plain(value) ? value : {};
  if (kind === 'list') return Array.isArray(value) ? value : [];
  const text = Array.isArray(value) || plain(value) || value === null ? '' : String(shown(value));
  try { return readLeaf(kind, text); } catch { return fresh(ANY_BASE[kind]); }
}
const ANY_BASE = { string: { kind: 'Base', base: 'string' }, integer: { kind: 'Base', base: 'int' },
  number: { kind: 'Base', base: 'float' }, boolean: { kind: 'Base', base: 'bool' } };
export function changeStructure(config, fields, { insert, remove, key, retype, kind }) {
  const path = JSON.parse(insert ?? remove ?? retype), field = fields.find(f => f.key === path[0]);
  if (!field) return config;
  if (field.kind === 'type-fields') {
    const rows = typeFields(config[field.key]) ?? [];
    let next;
    if (insert) {
      let n = 1;
      while (rows.some(row => row.name === `field_${n}`)) n++;
      next = [...rows, baseField(`field_${n}`, 'string')];
    } else next = rows.filter((_, i) => i !== path.at(-1));
    try { return { ...config, [field.key]: recordType(next) }; }
    catch (error) { throw Object.assign(new TypeError(error.message), { code: 'FIELD_LIMIT_REACHED' }); }
  }
  const shapeAt = (shape, rest, value) => rest.reduce(([s, v], segment) =>
    [Array.isArray(v) ? itemShape(s) : fieldShape(s, segment), v?.[segment]], [shape, value]);
  if (retype !== undefined) {
    const [shape] = shapeAt(field.shape, path.slice(1), config[path[0]]);
    if (shape.kind !== 'Any' || !anyKinds.includes(kind)) throw invalid();
    const put = (value, rest) => rest.length === 0 ? retyped(value, kind)
      : Array.isArray(value) ? value.map((item, i) => i === rest[0] ? put(item, rest.slice(1)) : item)
        : { ...value, [rest[0]]: put(value?.[rest[0]], rest.slice(1)) };
    return { ...config, [path[0]]: put(config[path[0]], path.slice(1)) };
  }
  const edit = (value, rest, at) => {
    if (rest.length === (remove ? 1 : 0)) {
      if (remove) return Array.isArray(value) ? value.filter((_, i) => i !== rest[0]) : Object.fromEntries(Object.entries(value).filter(([k]) => k !== rest[0]));
      const [shape] = shapeAt(field.shape, path.slice(1), config[path[0]]);
      if (Array.isArray(value)) return [...value, fresh(itemShape(shape))];
      if (!key || Object.hasOwn(value, key)) throw invalid();
      return { ...value, [key]: fresh(fieldShape(shape, key)) };
    }
    const [head, ...tail] = rest;
    return Array.isArray(value) ? value.map((item, i) => i === head ? edit(item, tail) : item)
      : { ...value, [head]: edit(value[head], tail) };
  };
  const container = config[path[0]] ?? (field.shape.kind === 'Array' ? [] : {});
  return { ...config, [path[0]]: edit(container, path.slice(1)) };
}
