import { sameValue } from './value-equal.js';
import { authoringCommitArgumentsValue, authoringCommitFrameFromValue } from '../authoring-query.js';
import { CircularUInt } from '../value.js';
import { scopeIdentityFromValue, scopeIdentityValue, actorIdentityFromValue } from '../establishment.js';
import { logCutFromValue } from '../replay-values.js';
import { edgeKeyFromValue } from '../declaration-values.js';
import {
  ActorHealthState, ActorHealthReasonCode, ApprovalDecisionKind, BuiltinObservationName, LifecycleWord, QueryId,
  SubscriptionTarget, rejectionCode,
} from './closed-tables.js';

function fail(place) { throw new TypeError(`invalid record value: ${place}`); }
function fields(value, required, optional=[]) {
  if (!value || typeof value !== 'object' || Array.isArray(value) || value instanceof Uint8Array
    || required.some(key=>!Object.hasOwn(value,key))
    || Object.keys(value).some(key=>![...required,...optional].includes(key))) fail(required.join(','));
}
function scope(value) {
  const decoded=scopeIdentityFromValue(value);
  if (!sameValue(scopeIdentityValue(decoded),value)) fail('scope');
}
export function recordStampFromValue(value) {
  if (!Array.isArray(value) || value.length!==5
    || [0,1,3,4].some(index=>!(value[index] instanceof CircularUInt))
    || value[4].value===0n) fail('origin Stamp');
  if (value[2]!==1n && value[2]!==2n) actorIdentityFromValue(value[2]);
  return value;
}
export function actorEventsItemFromValue(value) {
  if (!value || typeof value!=='object' || Array.isArray(value) || Object.hasOwn(value,'retired')) fail('actor.events item');
  if (Object.hasOwn(value,'origin')) recordStampFromValue(value.origin);
  if (Object.hasOwn(value,'at')) recordStampFromValue(value.at);
  if (Object.hasOwn(value,'causal_parents')) {
    if (!Array.isArray(value.causal_parents)) fail('actor.events causal_parents');
    for (const parent of value.causal_parents) recordStampFromValue(parent);
  }
  if (Object.hasOwn(value,'occasion')) {
    fields(value.occasion,['origin'],['edge']);
    recordStampFromValue(value.occasion.origin);
  }
  if (Object.hasOwn(value,'port') && typeof value.port !== 'string') fail('actor.events port');
  if (Object.hasOwn(value,'approval')) {
    const approval = value.approval;
    fields(approval,['item','decision'],['target_effect']);
    effectKey(approval.item,'approval item');
    if (!ApprovalDecisionKind.includes(approval.decision)) fail('approval decision');
    if (Object.hasOwn(approval,'target_effect') !== (approval.decision === 'approved')) fail('approval target_effect');
    if (Object.hasOwn(approval,'target_effect')) effectKey(approval.target_effect,'approval target_effect');
  }
  if (Object.hasOwn(value,'ticket')) {
    fields(value.ticket,['item']);
    effectKey(value.ticket.item,'ticket item');
  }
  return value;
}
function effectKey(value, place) { if (!Array.isArray(value) || value.length!==4) fail(place); }
export function reachedFromValue(value) {
  fields(value,['revision_epoch','cut']);
  if (!(value.revision_epoch instanceof CircularUInt) || value.revision_epoch.value===0n) fail('reached revision');
  logCutFromValue(value.cut);
  return value;
}
export function recordsCursorFromValue(value) {
  fields(value,['anchor','domain','position']);
  if (value.domain!=='records' || !(value.position instanceof Uint8Array)) fail('records cursor');
  if (value.position[0] < 1 || value.position[0] > 5 || value.position.length === 0) fail('records cursor position');
  if (!Array.isArray(value.anchor) || value.anchor.length!==2
    || !(value.anchor[0] instanceof Uint8Array) || value.anchor[0].length!==32) fail('records anchor');
  scope(value.anchor[1]);
  return value;
}
export function recordsArgumentsValue(value) {
  fields(value,['scope'],['since']);
  scope(value.scope);
  if (Object.hasOwn(value,'since')) {
    recordsCursorFromValue(value.since);
    if (!sameValue(value.scope,value.since.anchor[1])) fail('records since scope');
  }
  return value;
}
const SYSTEM_KINDS = Object.freeze(Object.fromEntries(
  BuiltinObservationName.filter(row => row.as_str.startsWith('system_'))
    .map(row => [row.as_str.slice('system_'.length), row.name.slice('System'.length)]),
));
function systemFromValue(value) {
  const kind = Object.hasOwn(SYSTEM_KINDS, value?.kind) ? SYSTEM_KINDS[value.kind] : undefined;
  if (!kind) fail('system kind');
  const extra = kind === 'PauseAccepted' ? ['force'] : kind === 'ResumeAccepted' ? [] : ['code'];
  fields(value, ['producer', 'kind', 'revision', ...extra]);
  if (value.producer !== 'pipeline') fail('system producer');
  if (!(value.revision instanceof CircularUInt) || value.revision.value === 0n) fail('system revision');
  const system = { producer: 'Pipeline', kind, revision: value.revision };
  if (extra[0] === 'force') {
    if (typeof value.force !== 'boolean') fail('system force');
    system.force = value.force;
  } else if (extra[0] === 'code') {
    if (value.code !== null && (!(value.code instanceof CircularUInt) || value.code.value > 0xffffffffn)) {
      fail('system code');
    }
    system.code = value.code === null ? null : Number(value.code.value);
  }
  return system;
}
export function recordsItemFromValue(value) {
  fields(value, ['cursor', 'fact'], ['reached', 'system', 'observation_bucket']);
  recordsCursorFromValue(value.cursor);
  if (!(value.fact instanceof Uint8Array)) fail('ProductRecordCodec Bytes');
  if (Object.hasOwn(value, 'reached')) reachedFromValue(value.reached);
  const decoded = Object.hasOwn(value, 'system') ? { ...value, system: systemFromValue(value.system) } : value;
  if (!Object.hasOwn(decoded, 'observation_bucket')) return decoded;
  if (!(decoded.observation_bucket instanceof CircularUInt)
    || decoded.observation_bucket.value > BigInt(Number.MAX_SAFE_INTEGER)) fail('observation_bucket');
  const { observation_bucket, ...item } = decoded;
  return { ...item, observationBucket: Number(observation_bucket.value) };
}
export function recordQueryPageFromValue(value, readItem) {
  fields(value,['anchor','items','terminal'],['reached','cut','folded_from']);
  if (Object.hasOwn(value,'cut')) logCutFromValue(value.cut);
  if (Object.hasOwn(value,'folded_from')) logCutFromValue(value.folded_from);
  if (!Array.isArray(value.items)) fail('page items');
  value.items.forEach(readItem);
  const terminal=value.terminal;
  if (terminal!==2n && (!Array.isArray(terminal) || terminal.length!==2
    || ![1n,3n].includes(terminal[0]))) fail('page terminal');
  if (Array.isArray(terminal) && terminal[0]===3n
    && (typeof terminal[1]!=='bigint' || terminal[1]<0n || terminal[1]>0xffffffffn)) fail('page diagnostic');
  if (Object.hasOwn(value,'reached')) reachedFromValue(value.reached);
  return value;
}

const EDGE_DEPTH_ENDED = BigInt(rejectionCode('EndedBeforeAnswering'));
export function edgeDepthsItemFromValue(value) {
  if (value && typeof value === 'object' && Object.hasOwn(value, 'actor')) {
    fields(value, ['actor', 'code']);
    try { actorIdentityFromValue(value.actor); } catch { fail('edge.depths actor'); }
    if (!(value.code instanceof CircularUInt) || value.code.value !== EDGE_DEPTH_ENDED) fail('edge.depths code');
    return value;
  }
  fields(value, ['edge', 'depth', 'queued', 'capacity']);
  try { edgeKeyFromValue(value.edge); } catch { fail('edge.depths edge'); }
  if (!(value.depth instanceof CircularUInt)) fail('edge.depths depth');
  if (!(value.queued instanceof CircularUInt)) fail('edge.depths queued');
  if (value.capacity !== null
    && (!(value.capacity instanceof CircularUInt) || value.capacity.value === 0n)) fail('edge.depths capacity');
  return value;
}

function registeredQuery(row) {
  return Object.freeze({ name: row.name, paging: row.paging === 'None' ? 'none' : 'cursor', anchorKind: 'Records' });
}
function registeredSubscription(row) {
  return Object.freeze({ name: row.name, discipline: row.delivery.toLowerCase() });
}
export const recordRegistrations=Object.freeze({
  [SubscriptionTarget.AuthoringCommits.name]: Object.freeze({
    subscription: registeredSubscription(SubscriptionTarget.AuthoringCommits),
    item: authoringCommitFrameFromValue,
    args(value) {
      fields(value, ['after', 'scope']);
      authoringCommitArgumentsValue(scopeIdentityFromValue(value.scope), value.after);
      return value;
    },
  }),
  [QueryId.ActorEvents.name]:Object.freeze({
    query:registeredQuery(QueryId.ActorEvents),
    subscription:registeredSubscription(SubscriptionTarget.ActorEvents),
    item:actorEventsItemFromValue,
    args(value) { if(value!==null)fail('actor.events Null args');return value; },
  }),
  [SubscriptionTarget.EdgeDepths.name]: Object.freeze({
    subscription: registeredSubscription(SubscriptionTarget.EdgeDepths),
    item: edgeDepthsItemFromValue,
    args(value) { if (value !== null) fail('edge.depths Null args'); return value; },
  }),
  [QueryId.Records.name]:Object.freeze({
    query:registeredQuery(QueryId.Records),
    subscription:registeredSubscription(SubscriptionTarget.Records),
    item:recordsItemFromValue,
    subscriptionItem:recordsItemFromValue,
    args:recordsArgumentsValue,
  }),
});

const DAEMON_HEALTH_BODY_VERSION = 1n;
const DAEMON_HEALTH_LIFECYCLES = Object.freeze(Object.fromEntries(
  LifecycleWord.map(word => [word.as_str, word.pipeline_stands]),
));
const DAEMON_HEALTH_STORAGE_CODES = Object.freeze([BigInt(rejectionCode('ArrivalRecorderStopped'))]);
const DAEMON_HEALTH_STATES = ActorHealthState;
export const DAEMON_HEALTH_REASONS = ActorHealthReasonCode;
const DAEMON_HEALTH_WAITING = Object.freeze(['harness_unbound', 'harness_unusable']);
const DAEMON_HEALTH_JOURNAL_USAGE = Object.freeze(['arrivals_max_bytes', 'arrivals_max_records', 'bytes', 'file_bytes', 'records', 'total_max_bytes']);
function daemonHealthJournalFromValue(value) {
  fields(value, ['ceiling', 'usage']);
  if (value.ceiling !== null) {
    fields(value.ceiling, ['code', 'record', 'since_ms']);
    if (typeof value.ceiling.code !== 'string' || !/^journal\.[a-z][a-z0-9_]*$/.test(value.ceiling.code)) {
      fail('daemon health journal ceiling code');
    }
    if (!(value.ceiling.record instanceof CircularUInt) || value.ceiling.record.value === 0n) fail('daemon health journal ceiling record');
    if (!(value.ceiling.since_ms instanceof CircularUInt)) fail('daemon health journal ceiling since_ms');
  }
  if (value.usage !== null) {
    fields(value.usage, DAEMON_HEALTH_JOURNAL_USAGE);
    if (DAEMON_HEALTH_JOURNAL_USAGE.some(key => !(value.usage[key] instanceof CircularUInt))) fail('daemon health journal usage');
  }
}
export function daemonHealthAnchorFromValue(value) {
  fields(value, ['config_defaults', 'dead_letter', 'journal', 'lifecycle', 'storage', 'version', 'wall_clock']);
  daemonHealthJournalFromValue(value.journal);
  if (!(value.version instanceof CircularUInt) || value.version.value !== DAEMON_HEALTH_BODY_VERSION) {
    fail('daemon health body version');
  }
  if (value.lifecycle !== null && !Object.hasOwn(DAEMON_HEALTH_LIFECYCLES, value.lifecycle)) {
    fail('daemon health lifecycle');
  }
  if (value.storage !== null && (!(value.storage instanceof CircularUInt)
    || !DAEMON_HEALTH_STORAGE_CODES.includes(value.storage.value))) {
    fail('daemon health storage code');
  }
  if (value.dead_letter !== null) recordStampFromValue(value.dead_letter);
  if (value.wall_clock !== null) {
    fields(value.wall_clock, ['at_ms', 'wall_ms']);
    if (!(value.wall_clock.at_ms instanceof CircularUInt) || !(value.wall_clock.wall_ms instanceof CircularUInt)) {
      fail('daemon health wall clock');
    }
  }
  if (!Array.isArray(value.config_defaults)) fail('daemon health config defaults');
  for (const entry of value.config_defaults) {
    fields(entry, ['key', 'value']);
    if (typeof entry.key !== 'string' || entry.key === '' || !(entry.value instanceof CircularUInt)) {
      fail('daemon health config default');
    }
  }
  return value;
}
const FAILURE_CODE = /^[a-z][a-z0-9_]*\.[a-z][a-z0-9_]*$/;
export function daemonHealthItemFromValue(value) {
  fields(value, ['actor', 'detail', 'reason', 'record', 'since_ms', 'state']);
  actorIdentityFromValue(value.actor);
  if (!DAEMON_HEALTH_STATES.includes(value.state)) fail('daemon health state');
  if (value.state === 'failed' || value.state === 'backpressure') {
    if (!DAEMON_HEALTH_REASONS.includes(value.reason) || DAEMON_HEALTH_WAITING.includes(value.reason)) {
      fail('daemon health reason code');
    }
  } else if (value.reason !== null && !(value.state === 'waiting' && DAEMON_HEALTH_WAITING.includes(value.reason))) {
    fail('daemon health unexpected reason');
  }
  if (value.detail !== null) {
    fields(value.detail, ['code', 'slot']);
    if (value.reason === null) fail('daemon health detail without a reason');
    if (typeof value.detail.code !== 'string' || !FAILURE_CODE.test(value.detail.code)) {
      fail('daemon health detail code');
    }
    if (value.detail.slot !== null && (typeof value.detail.slot !== 'string' || value.detail.slot === '')) {
      fail('daemon health detail slot');
    }
  }
  if (!(value.record instanceof CircularUInt) || value.record.value === 0n) fail('daemon health record');
  if (!(value.since_ms instanceof CircularUInt)) fail('daemon health since_ms');
  return value;
}
/// Registered `daemon.health`: Null args, no page cursor, and no standing run
/// requirement — "no run is standing" is an answer, not an unavailability.
export const daemonHealthRegistration = Object.freeze({
  query: Object.freeze({ name: 'daemon.health', paging: 'none', anchorKind: 'DaemonHealth' }),
  anchor: daemonHealthAnchorFromValue,
  item: daemonHealthItemFromValue,
  args(value) { if (value !== null) fail('daemon.health Null args'); return value; },
});

import { deadLetterItemFromValue } from '../observation-rows.js';
export { deadLetterItemFromValue, deadLetterTargetFromValue } from '../observation-rows.js';
export const deadLetterRegistration = Object.freeze({
  query: Object.freeze({ name: 'dead.letters', paging: 'cursor', anchorKind: 'Records' }),
  item: deadLetterItemFromValue,
  args(value) { if (value !== null) fail('dead.letters Null args'); return value; },
});
