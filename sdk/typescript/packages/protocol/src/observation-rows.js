import { sameValue } from './internal/value-equal.js';
import { CircularUInt } from './value.js';
import { actorIdentityFromValue, scopeIdentityFromValue } from './establishment.js';
import { recordStampFromValue } from './internal/record-values.js';
import { DeadLetterReasonKind, EvalMode, LifecyclePhase } from './internal/closed-tables.js';
import { decodePortFlow } from './port-type.js';

function fail(place) { throw new TypeError(`invalid observation row: ${place}`); }
function record(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    && !(value instanceof Uint8Array) && !(value instanceof CircularUInt);
}
function fields(value, required, optional, place) {
  if (!record(value) || required.some(key => !Object.hasOwn(value, key))
    || Object.keys(value).some(key => !required.includes(key) && !optional.includes(key))) fail(place);
}
const text = value => typeof value === 'string';
const int = value => typeof value === 'bigint';
const uintValue = (value, place) => {
  if (!(value instanceof CircularUInt)) fail(place);
  return value.value;
};

/** `DeadLetterReasonKind` spellings in declaration order, from `@circular/protocol/tables`. */
export const DEAD_LETTER_REASONS = DeadLetterReasonKind;
const UNIT_REASONS = Object.freeze(['outcome_unclaimed', 'destination_gone', 'poisoned', 'capacity']);

/** `decode_dead_letter_reason`: `{code, detail}` with the arm's own detail. */
export function deadLetterReasonFromValue(value) {
  fields(value, ['code', 'detail'], [], 'dead letter reason');
  if (!DEAD_LETTER_REASONS.includes(value.code)) fail('dead letter reason code');
  if (UNIT_REASONS.includes(value.code) && value.detail !== null) fail('dead letter unit reason detail');
  if (value.code === 'actor_declared' && !text(value.detail)) fail('dead letter actor_declared name');
  if (value.code === 'processing') preprocessFailurePoint(value.detail);
  return value;
}
function deadLetterEndpoint(value) {
  fields(value, ['actor', 'port'], [], 'dead letter endpoint');
  actorIdentityFromValue(value.actor);
  if (!text(value.port)) fail('dead letter endpoint port');
}
export function deadLetterTargetFromValue(value) {
  if (Array.isArray(value)) {
    if (value[0] === 1n && value.length === 4) {
      deadLetterEndpoint(value[1]);
      deadLetterEndpoint(value[2]);
      if (!int(value[3]) || value[3] < 0n || value[3] > 0xffffffffn) fail('dead letter edge ordinal');
    } else if (value[0] === 2n && value.length === 2) {
      actorIdentityFromValue(value[1]);
    } else fail('dead letter edge');
  } else deadLetterEndpoint(value);
  return value;
}
function readDeadLetterItem(value, bucketField) {
  fields(value, ['dropped', 'origin', 'reason', 'scope', 'subject'], ['target', bucketField], 'dead letter item');
  recordStampFromValue(value.dropped);
  fields(value.origin, ['actor', 'port'], [], 'dead letter origin');
  actorIdentityFromValue(value.origin.actor);
  if (value.origin.port !== null && !text(value.origin.port)) fail('dead letter origin port');
  deadLetterReasonFromValue(value.reason);
  scopeIdentityFromValue(value.scope);
  fields(value.subject, ['shape', 'value'], [], 'dead letter subject');
  if (Object.hasOwn(value, 'target')) deadLetterTargetFromValue(value.target);
  if (!Object.hasOwn(value, bucketField)) return value;
  const bucket = value[bucketField];
  if (bucketField === 'observationBucket') {
    if (!Number.isSafeInteger(bucket) || bucket < 0) fail('observationBucket');
    return value;
  }
  if (!(bucket instanceof CircularUInt) || bucket.value > BigInt(Number.MAX_SAFE_INTEGER)) fail('observation_bucket');
  const { observation_bucket, ...item } = value;
  return { ...item, observationBucket: Number(observation_bucket.value) };
}
/** One recorded dead letter, with its optional recorded millisecond bucket decoded. */
export function deadLetterItemFromValue(value) {
  return readDeadLetterItem(value, 'observation_bucket');
}
export function preprocessFailurePoint(detail) {
  if (!Array.isArray(detail) || !(detail[0] instanceof CircularUInt) || detail[0].value !== 3n
    || detail.length === 1) return null;
  if (detail.length !== 2) fail('preprocess failure detail');
  fields(detail[1], ['failure_point'], ['code'], 'preprocess failure metadata');
  const code = Object.hasOwn(detail[1], 'code') ? detail[1].code : null;
  if (code !== null && !failureCode(code)) fail('preprocess failure code');
  const point = detail[1].failure_point;
  fields(point, ['edge', 'step'], [], 'preprocess failure point');
  fields(point.step, ['index', 'kind'], [], 'preprocess failure step');
  deadLetterTargetFromValue(point.edge);
  if (!Array.isArray(point.edge)) fail('preprocess failure edge');
  const index = uintValue(point.step.index, 'preprocess failure index');
  if (!text(point.step.kind)) fail('preprocess failure kind');
  return Object.freeze({ edge: point.edge, index, kind: point.step.kind, code });
}
function failureCode(value) {
  return typeof value === 'string' && /^[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*$/.test(value);
}
function streamAnchor(anchor, query) {
  if (anchor !== null) fail(`${query} anchor is not Null`);
  return anchor;
}
/** `decode_dead_letters`: one complete stream answer as ordinal-stable rows. */
export function decodeDeadLetters(anchor, items) {
  streamAnchor(anchor, 'dead.letters');
  if (!Array.isArray(items)) fail('dead.letters items');
  const rows = items.map((item, index) => {
    item = readDeadLetterItem(item, record(item) && Object.hasOwn(item, 'observationBucket')
      ? 'observationBucket' : 'observation_bucket');
    return Object.freeze({
      ordinal: BigInt(index),
      dropped: item.dropped,
      actor: item.origin.actor,
      port: item.origin.port,
      target: Object.hasOwn(item, 'target') ? item.target : null,
      reason: item.reason,
      scope: item.scope,
      subject_shape: item.subject.shape,
      subject: item.subject.value,
      ...(Object.hasOwn(item, 'observationBucket') ? { observationBucket: item.observationBucket } : {}),
    });
  });
  return Object.freeze({ rows: Object.freeze(rows) });
}

const INSTANCE_TRANSITION_KINDS = Object.freeze({ 1: 'Minted', 2: 'Retired' });
/** `incarnation_transition.rs` LIFECYCLE_CARRIER — the first cell of an incarnation transition. */
const LIFECYCLE_CARRIER = 'daemon-actor-lifecycle-v2';
/** `LifecyclePhase` names in declaration order, from `@circular/protocol/tables`. */
export const INCARNATION_PHASES = Object.freeze(LifecyclePhase.map(({ name }) => name));
const PHASE_BY_TAG = new Map(LifecyclePhase.map(({ name, tag }) => [BigInt(tag), name]));
const INCARNATION_FIELDS = Object.freeze([
  'generation', 'declaration_revision', 'config_revision', 'at',
  'retired', 'prepared', 'remaining', 'next_recovery',
]);
const scalarKey = value => text(value) || int(value) || typeof value === 'boolean';
function instanceKey(value) {
  if (Array.isArray(value) ? !value.every(scalarKey) : !scalarKey(value)) fail('instance key scalar');
  return value;
}
/**
 * One `instance.transitions` row. An instance-set transition reads as `InstanceTransition`
 * `{kind, container, key}`; an incarnation transition reads as `IncarnationTransition`
 * `{phase, actor, generation, …}` (`LifecycleIdentity` + `LifecycleProjection`).
 */
export function instanceTransitionFromValue(value) {
  if (!Array.isArray(value)) fail('instance transition row');
  if (value[0] === LIFECYCLE_CARRIER) {
    if (value.length !== 11) fail('incarnation transition arity');
    const phase = PHASE_BY_TAG.get(uintValue(value[1], 'incarnation phase'));
    if (phase === undefined) fail('incarnation phase');
    actorIdentityFromValue(value[2]);
    const row = { phase, actor: value[2] };
    INCARNATION_FIELDS.forEach((name, index) => { row[name] = uintValue(value[3 + index], `incarnation ${name}`); });
    return Object.freeze(row);
  }
  if (value.length !== 3) fail('instance transition arity');
  const kind = int(value[0]) ? INSTANCE_TRANSITION_KINDS[value[0]] : undefined;
  if (kind === undefined) fail('instance transition kind');
  scopeIdentityFromValue(value[1]);
  return Object.freeze({ kind, container: value[1], key: instanceKey(value[2]) });
}
export function decodeInstanceTransitions(items) {
  if (!Array.isArray(items)) fail('instance.transitions items');
  return Object.freeze(items.map(instanceTransitionFromValue));
}
/**
 * `project_active_instance_facts`: fold one complete stream answer into the live
 * instance set of one exact container. Re-mint after a recorded retirement is valid; a
 * duplicate mint while live and a retirement while absent refuse the answer.
 */
export function projectActiveInstanceFacts(anchor, items, container) {
  streamAnchor(anchor, 'instance.transitions');
  const live = [];
  decodeInstanceTransitions(items).forEach((row, index) => {
    if (!Object.hasOwn(row, 'kind') || !sameValue(row.container, container)) return;
    const at = live.findIndex(key => sameValue(key, row.key));
    if (row.kind === 'Minted') {
      if (at !== -1) fail(`instance.transitions row ${index} mints an already-live key`);
      live.push(row.key);
    } else {
      if (at === -1) fail(`instance.transitions row ${index} retires a non-live key`);
      live.splice(at, 1);
    }
  });
  return Object.freeze({ container, rows: Object.freeze(live.map(key => Object.freeze({ key }))) });
}

const INTERVAL_DOMAINS = Object.freeze(['milliseconds', 'nonzero_milliseconds']);
const TEXT_DOMAINS = Object.freeze(['label', 'name', 'tool_name', 'model_provider_name', 'model_name']);
/** Rust `circular_expr::EvalMode` spellings, from `@circular/protocol/tables`. */
const SNIPPET_MODES = EvalMode;
const nonempty = (value, place) => { if (!text(value) || value.length === 0) fail(place); return value; };
const utf8 = new TextEncoder();
function compareText(left, right) {
  const a = utf8.encode(left), b = utf8.encode(right);
  for (let i = 0; i < Math.min(a.length, b.length); i++) if (a[i] !== b[i]) return a[i] - b[i];
  return a.length - b.length;
}
function compareSegment(left, right) {
  if (left[0] !== right[0]) return left[0] < right[0] ? -1 : 1;
  return left[0] === 1n ? compareText(left[1], right[1]) : left[1] < right[1] ? -1 : left[1] > right[1] ? 1 : 0;
}
function comparePath(left, right) {
  for (let i = 0; i < Math.min(left.length, right.length); i++) {
    const order = compareSegment(left[i], right[i]);
    if (order) return order;
  }
  return left.length - right.length;
}
function strictOrder(values, compare, place) {
  for (let i = 1; i < values.length; i++) if (compare(values[i - 1], values[i]) >= 0) fail(`${place} order`);
}
function configPath(value, place) {
  if (!Array.isArray(value) || value.length === 0) fail(place);
  for (const segment of value) {
    if (!Array.isArray(segment) || segment.length !== 2) fail(`${place} segment`);
    if (segment[0] === 1n ? !text(segment[1]) : segment[0] !== 2n || !int(segment[1]) || segment[1] < 0n) fail(`${place} segment`);
  }
  return value;
}
function constraint(value) {
  if (value === null) return;
  if (!Array.isArray(value)) fail('create-input constraint');
  const [tag, ...args] = value;
  const ok = ((tag === 1n || tag === 10n) && args.length === 1 && INTERVAL_DOMAINS.includes(args[0]))
    || (tag === 2n && args.length === 2 && (args[0] === 0n || args[0] === 1n)
      && (args[1] === null || (int(args[1]) && args[1] >= args[0])))
    || ((tag === 3n || tag === 4n || tag === 7n || tag === 8n || tag === 9n) && args.length === 0)
    || (tag === 6n && args.length === 1 && TEXT_DOMAINS.includes(args[0]));
  if (tag === 5n && args.length === 1 && Array.isArray(args[0]) && args[0].length > 0 && args[0].every(text)) {
    strictOrder(args[0], compareText, 'create-input closed tags');
    return;
  }
  if (!ok) fail('create-input constraint arm');
}
function snippet(value) {
  if (value === null) return;
  fields(value, ['inlets', 'mode'], [], 'create-input snippet');
  if (!SNIPPET_MODES.includes(value.mode)) fail('create-input snippet mode');
  if (!Array.isArray(value.inlets)) fail('create-input snippet inlets');
  value.inlets.forEach(inlet => nonempty(inlet, 'create-input snippet inlet'));
  if (new Set(value.inlets).size !== value.inlets.length) fail('create-input snippet repeats inlet');
}
/** `decode_slot`: one config slot — also the `query.catalog` preprocess slot projection. */
export function actorCreateInputSlotFromValue(value) {
  fields(value, ['constraint', 'path', 'requirement', 'shape', 'snippet', 'label', 'description', 'group'], ['policies'], 'create-input slot');
  for (const key of ['label', 'description', 'group']) {
    if (value[key] !== null && typeof value[key] !== 'string') fail(`create-input slot ${key}`);
  }
  constraint(value.constraint);
  configPath(value.path, 'create-input slot path');
  const requirement = value.requirement;
  if (!Array.isArray(requirement) || !((requirement[0] === 1n && requirement.length === 1)
    || ((requirement[0] === 2n || requirement[0] === 3n) && requirement.length === 2))) fail('create-input requirement');
  if (requirement[0] === 3n) {
    try { decodePortFlow(requirement[1]); } catch { fail('create-input omittable Flow'); }
  }
  snippet(value.snippet);
  if (Object.hasOwn(value, 'policies')) {
    const names = record(value.policies) ? Object.keys(value.policies) : [];
    if (names.length === 0) fail('capability policies');
    for (const name of names) {
      nonempty(name, 'capability policy name');
      const inputs = value.policies[name];
      if (!Array.isArray(inputs) || inputs.length === 0) fail('capability policy inputs');
      const prefix = [...value.path, [1n, name]];
      for (const input of inputs) {
        actorCreateInputSlotFromValue(input);
        if (Object.hasOwn(input, 'policies') || input.requirement[0] !== 1n
          || input.path.length !== prefix.length + 1
          || !sameValue(input.path.slice(0, prefix.length), prefix)) fail('capability policy input');
      }
      strictOrder(inputs.map(input => input.path), comparePath, 'capability policy inputs');
    }
  }
  return value;
}
function contract(schema, draft, missing) {
  fields(schema, ['relations', 'slots'], [], 'create-input schema');
  if (!record(draft)) fail('create-input draft');
  if (!Array.isArray(schema.slots) || schema.slots.length === 0) fail('create-input slots');
  schema.slots.forEach(actorCreateInputSlotFromValue);
  const paths = schema.slots.map(slot => slot.path);
  strictOrder(paths, comparePath, 'create-input slots');
  if (!Array.isArray(schema.relations)) fail('create-input relations');
  schema.relations.forEach((relation, index) => {
    if (!Array.isArray(relation) || relation.length !== 2 || relation[0] !== 1n) fail('create-input relation');
    configPath(relation[1], 'create-input relation path');
    if (!paths.some(path => sameValue(path, relation[1]))) fail('create-input relation targets undeclared path');
    if (schema.relations.slice(0, index).some(prior => sameValue(prior, relation))) fail('create-input relation repeats');
  });
  if (!Array.isArray(missing)) fail('create-input missing');
  missing.forEach(path => configPath(path, 'create-input missing path'));
  strictOrder(missing, comparePath, 'create-input missing paths');
  const mandatory = schema.slots.filter(slot => slot.requirement[0] === 1n).map(slot => slot.path);
  if (!sameValue(missing, mandatory)) fail('create-input missing paths do not match mandatory slots');
}
/** One `actor.create-inputs` entry: `{actor_type, state}` with its closed state arm. */
export function actorCreateInputCatalogEntryFromValue(value) {
  fields(value, ['actor_type', 'state'], [], 'create-input item');
  nonempty(value.actor_type, 'create-input actor_type');
  const state = value.state;
  if (!Array.isArray(state)) fail('create-input state');
  if (state[0] === 1n && state.length === 1) return value;
  if (state[0] === 2n && state.length === 4) { contract(state[1], state[2], state[3]); return value; }
  if (state[0] === 3n && state.length === 2) { nonempty(state[1], 'create-input unavailable reason'); return value; }
  return fail('create-input state arm');
}
/** `decode_actor_create_inputs`: the anchor names every entry, in order, once. */
export function decodeActorCreateInputs(anchor, items) {
  if (!Array.isArray(anchor) || !Array.isArray(items)) fail('actor.create-inputs page');
  anchor.forEach(name => nonempty(name, 'create-input anchor identity'));
  if (new Set(anchor).size !== anchor.length) fail('create-input anchor repeats identity');
  if (anchor.length !== items.length) fail('create-input anchor and items differ in length');
  items.forEach((item, index) => {
    actorCreateInputCatalogEntryFromValue(item);
    if (item.actor_type !== anchor[index]) fail('create-input anchor identity does not match item');
  });
  return Object.freeze([...items]);
}
