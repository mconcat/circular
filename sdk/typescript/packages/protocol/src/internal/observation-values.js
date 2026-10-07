import { CircularUInt } from '../value.js';
import { scopeIdentityFromValue, actorIdentityFromValue } from '../establishment.js';
import { logCutFromValue } from '../replay-values.js';
import { PREPROCESS_KINDS } from '../declaration-values.js';
import {
  recordQueryPageFromValue, recordsItemFromValue, actorEventsItemFromValue,
  daemonHealthAnchorFromValue, daemonHealthItemFromValue, deadLetterRegistration,
} from './record-values.js';
import { deadLetterItemFromValue, instanceTransitionFromValue } from '../observation-rows.js';
import { QueryId } from './closed-tables.js';

function fail(field) { throw new TypeError(`invalid observation value: ${field}`); }
function fields(value, names) {
  if (!value || typeof value !== 'object' || Array.isArray(value) || value instanceof Uint8Array
    || names.some(key => !Object.hasOwn(value, key))
    || Object.keys(value).some(key => !names.includes(key))) fail(names.join(','));
}
function uint(value, field) { if (!(value instanceof CircularUInt)) fail(field); }
function int(value, field) {
  if (typeof value !== 'bigint' || value < 0n || value > 0x7fffffffffffffffn) fail(field);
}
function tuple(value, tag, length) { return Array.isArray(value) && value.length === length && value[0] === tag; }
function key(value, field) { if (!Array.isArray(value) || value.length !== 4) fail(field); }
function unavailable(value) { return tuple(value, 2n, 2) && value[1] === 1n; }

export function daemonHealthPageFromValue(value) {
  recordQueryPageFromValue(value, daemonHealthItemFromValue);
  daemonHealthAnchorFromValue(value.anchor);
  if (value.terminal !== 2n) fail('daemon.health terminal');
  return value;
}
export function recordsPageFromValue(value) {
  const items = [];
  const page = recordQueryPageFromValue(value, item => items.push(recordsItemFromValue(item)));
  return { ...page, items };
}
export function actorEventsPageFromValue(value) {
  return recordQueryPageFromValue(value, item => {
    actorEventsItemFromValue(item);
    actorIdentityFromValue(item.actor);
    int(item.index, 'actor.events index');
    int(item.observed_at_ms, 'actor.events observed_at_ms');
    if (!Object.hasOwn(item, 'body')) fail('actor.events body');
  });
}

export function timelinePageFromValue(value) {
  recordQueryPageFromValue(value, item => {
    fields(item, ['at_ms', 'cut', 'revision_epoch', 'stream']);
    int(item.at_ms, 'at_ms');
    int(item.stream, 'stream');
    uint(item.revision_epoch, 'revision_epoch');
    if (item.revision_epoch.value === 0n) fail('revision_epoch');
    logCutFromValue(item.cut);
  });
  fields(value.anchor, ['epoch_plans']);
  const plans = value.anchor.epoch_plans;
  if (tuple(plans, 1n, 2)) {
    fields(plans[1], ['latest_revision_epoch', 'source_checkpoints']);
    uint(plans[1].latest_revision_epoch, 'latest_revision_epoch');
    if (plans[1].latest_revision_epoch.value === 0n) fail('latest_revision_epoch');
    uint(plans[1].source_checkpoints, 'source_checkpoints');
  } else if (tuple(plans, 2n, 2)) {
    fields(plans[1], ['reason']);
    if (typeof plans[1].reason !== 'string' || value.items.length !== 0) fail('epoch_plans reason');
  } else fail('epoch_plans');
  return value;
}

function emitter(value) {
  if (value === 3n || value === 4n) return;
  if (!Array.isArray(value) || value.length !== 3) fail('emitter');
  scopeIdentityFromValue(value[1]);
  if (value[0] === 1n && typeof value[2] === 'string') return;
  if (value[0] === 2n && value[2] instanceof Uint8Array && value[2].length === 16) return;
  fail('emitter');
}
function approvalState(value, approvedTag, emptyTag) {
  if (value === emptyTag) return;
  if (!tuple(value, approvedTag, 3)) fail('approval state');
  key(value[1], 'ledger_item'); key(value[2], 'target_effect');
}
function approvalCause(value) {
  if (value === null) return;
  fields(value, ['actor', 'index']);
  actorIdentityFromValue(value.actor);
  int(value.index, 'cause.index');
}
export function runtimeApprovalsPageFromValue(value) {
  recordQueryPageFromValue(value, row => {
    fields(row, ['item', 'emitter', 'target_effect', 'state', 'summary', 'cause']);
    key(row.item, 'item'); key(row.target_effect, 'target_effect'); emitter(row.emitter);
    approvalState(row.state, 2n, 1n);
    if (!unavailable(row.summary)) fail('summary');
    approvalCause(row.cause);
  });
  if (value.terminal !== 2n) fail('runtime.approvals terminal');
  fields(value.anchor, ['producer', 'persistence']);
  if (value.anchor.producer !== 1n && !unavailable(value.anchor.producer)) fail('producer');
  if (!tuple(value.anchor.persistence, 3n, 1) && !unavailable(value.anchor.persistence)) fail('persistence');
  return value;
}
export function approvalDecisionReceiptFromValue(value) {
  fields(value, ['item', 'outcome']);
  key(value.item, 'item');
  approvalState(value.outcome, 1n, 2n);
  return value;
}

function streamPage(value, readItem, name) {
  recordQueryPageFromValue(value, readItem);
  if (value.anchor !== null) fail(`${name} anchor`);
  return value;
}
export function deadLettersPageFromValue(value) {
  const items = [];
  const page = streamPage(value, item => items.push(deadLetterItemFromValue(item)), 'dead.letters');
  return { ...page, items };
}
export function instanceTransitionsPageFromValue(value) {
  return streamPage(value, instanceTransitionFromValue, 'instance.transitions');
}
export const deadLetters = deadLetterRegistration.query;
export const instanceTransitions = Object.freeze({ name: 'instance.transitions', paging: 'cursor', anchorKind: 'Records' });

export const timeline = Object.freeze({ name: 'timeline', paging: 'cursor', anchorKind: 'Records' });
export const runtimeApprovals = Object.freeze({ name: 'runtime.approvals', paging: 'none', anchorKind: 'Records' });

export const queryCatalog = Object.freeze({ name: 'query.catalog', paging: 'none', anchorKind: 'QueryCatalog' });
export function queryCatalogPageFromValue(value) {
  recordQueryPageFromValue(value, () => {});
  if (value.terminal !== 2n) fail('query.catalog terminal');
  fields(value.anchor, ['preprocess', 'queries']);
  const { preprocess, queries } = value.anchor;
  if (!Array.isArray(queries) || queries.some(name => typeof name !== 'string')) fail('query.catalog queries');
  if (!Array.isArray(preprocess)) fail('query.catalog preprocess');
  for (const entry of preprocess) {
    fields(entry, ['kind', 'slots']);
    if (!PREPROCESS_KINDS.includes(entry.kind)) fail('query.catalog preprocess kind');
    if (!Array.isArray(entry.slots)) fail('query.catalog preprocess slots');
  }
  return value;
}

export const agentHarnessCandidates = Object.freeze({
  name: QueryId.AgentHarnessCandidates.name, paging: 'none', anchorKind: 'Records' });
export function agentHarnessCandidatesPageFromValue(value) {
  recordQueryPageFromValue(value, row => {
    fields(row, ['name', 'found']);
    if (typeof row.name !== 'string' || row.name === '') fail('agent.harness-candidates name');
    if (row.found !== null && (typeof row.found !== 'string' || !row.found.startsWith('/'))) {
      fail('agent.harness-candidates found');
    }
  });
  if (value.terminal !== 2n) fail('agent.harness-candidates terminal');
  if (value.anchor !== null) fail('agent.harness-candidates anchor');
  return value;
}

export const agentHarnesses = Object.freeze({
  name: QueryId.AgentHarnesses.name, paging: 'none', anchorKind: 'Records' });
function agentHarnessItemFromValue(row) {
  fields(row, ['name', 'program', 'saved']);
  if (typeof row.name !== 'string' || row.name === '') fail('agent.harnesses name');
  for (const column of ['program', 'saved']) {
    if (row[column] !== null && (typeof row[column] !== 'string' || !row[column].startsWith('/'))) {
      fail(`agent.harnesses ${column}`);
    }
  }
  if (row.program === null && row.saved === null) fail('agent.harnesses row without a binding');
}
export function agentHarnessesPageFromValue(value) {
  recordQueryPageFromValue(value, agentHarnessItemFromValue);
  if (value.terminal !== 2n) fail('agent.harnesses terminal');
  if (!Array.isArray(value.anchor) || value.anchor.length !== value.items.length) fail('agent.harnesses anchor');
  value.anchor.forEach((row, index) => {
    agentHarnessItemFromValue(row);
    const item = value.items[index];
    if (row.name !== item.name || row.program !== item.program || row.saved !== item.saved) {
      fail('agent.harnesses anchor');
    }
  });
  return value;
}

export function setAgentHarnessAcceptedFromValue(value) {
  fields(value, ['at']);
  if (value.at !== null) fail('SetAgentHarness at');
  return value;
}
