import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { identity } from './query.mjs';
import { spark, table, bytesText, escape, kicker, formatTime, empty } from './view-registry.mjs';
import { viewRecords, causingArrival, isOwnOutcome, isEmission, recordedArrivalsText } from './arrivals.mjs';
import { recordedCall } from './approval-call.mjs';
import { reasonText } from './reasons.mjs';
import { kindText, causedBy, outcomeState, effectText } from './record-words.mjs';

const undeclared = 'TOOLS_UNDECLARED';
const unobserved = 'TOOL_RESULT_UNOBSERVED';
const callPreprocessed = 'TOOL_CALL_PREPROCESSED';
const callWireUnread = 'TOOL_CALL_UNREAD';
const causeNotCall = 'TOOL_CAUSE_NOT_CALL';
const valueNotBytes = 'RESULT_VALUE_NOT_BYTES';
const pageUnavailable = reasonText('READ_UNAVAILABLE');

function declaredTools(config) {
  const tools = config?.tools;
  if (!tools || typeof tools !== 'object' || Array.isArray(tools)) return [];
  return Object.entries(tools).map(([name, template]) => ({ name,
    effect: typeof template?.effect === 'string' ? template.effect : null }));
}

const byThis = (row, id) => identity(actorIdentityFromValue(row.actor)) === id;

function toolResults(rows, id) {
  return rows.flatMap(row => {
    if (id !== null && !(isEmission(row) && byThis(row, id))) return [];
    const body = row.body;
    if (!body || typeof body !== 'object' || Array.isArray(body)) return [];
    if (typeof body.effect !== 'string' || typeof body.ok !== 'boolean') return [];
    if (typeof row.observed_at_ms !== 'bigint') return [];
    return [{ at: Number(row.observed_at_ms) / 1000, at_ms: String(row.observed_at_ms),
      effect: body.effect, ok: body.ok,
      bytes: body.value instanceof Uint8Array ? body.value.length
        : body.value?.stdout instanceof Uint8Array ? body.value.stdout.length : null }];
  });
}

export function toolsPage(node, page) {
  const rows = (page?.items ?? []).filter(row => row.kind === 'actor_arrival' || isEmission(row));
  return { toolFacts: { observed: rows.some(row => typeof row.observed_at_ms === 'bigint'),
    results: toolResults(rows, node.portRecords ? null : node.id) } };
}

const later = (a, b) => !a || b.at >= a.at ? b : a;
const latestAt = (results, t) => results.filter(result => result.at <= t).reduce(later, null);
const integer = value => typeof value === 'bigint' ? value : typeof value?.value === 'bigint' ? value.value : null;

export function toolOutcomes(node, graph) {
  const column = viewRecords(node);
  return column.filter(row => isOwnOutcome(row) && typeof row.observed_at_ms === 'bigint').map(row => {
    const cause = causingArrival(row, column);
    const read = cause.row ? recordedCall(cause.row, graph) : null;
    const call = read?.call ?? null;
    return { index: String(row.index), at: Number(row.observed_at_ms) / 1000, at_ms: String(row.observed_at_ms),
      kind: row.body.kind, ok: row.body.ok, exit: integer(row.body.exit),
      cause: cause.row ? String(cause.row.index) : null, tool: call?.tool ?? null,
      code: cause.code ?? (call ? null : read.steps === null ? callWireUnread
        : read.steps.length ? callPreprocessed : causeNotCall) };
  });
}
const outcomeText = outcome => [outcomeState(outcome), kindText(outcome.kind),
  outcome.cause === null ? null : causedBy(outcome.cause), formatTime(outcome.at_ms)?.detail].filter(Boolean).join(' · ');

function outcomeAt(outcomes, t) {
  const outcome = latestAt(outcomes, t);
  const code = outcome ? outcome.code : unobserved;
  return { state: outcome ? outcomeState(outcome) : reasonText(code),
    code, sentence: outcome ? null : code,
    title: [outcome ? outcomeText(outcome) : null, code ? reasonText(code) : null].filter(Boolean).join(' · ') };
}

function renderOutcome(node) {
  const outcome = outcomeAt(node.toolOutcomes ?? [], Infinity);
  return `<div class="viewer-transfer" data-tool-outcome data-reason="${escape(outcome.code)}" title="${escape(outcome.title)}"><span>OUTCOME</span><strong data-tool-outcome-state${outcome.sentence ? ` data-reason="${escape(outcome.sentence)}"` : ''}>${escape(outcome.state)}</strong></div>`;
}

const toolsCount = count => `${count} ${count === 1 ? 'tool' : 'tools'}`;

function rowsAt(declared, outcomes, observed, t) {
  if (!declared.length) return [];
  return declared.map(({ name, effect }) => {
    const latest = latestAt(outcomes.filter(outcome => outcome.code !== null || outcome.tool === name), t);
    const code = effect === null ? undeclared : latest ? latest.code : unobserved;
    return { name, at: code ? null : latest.at, code, state: code ? reasonText(code) : outcomeState(latest),
      title: [name, effect === null ? reasonText(undeclared) : effectText(effect), code ? reasonText(code) : null,
        effect !== null && latest ? outcomeText(latest) : null, observed].filter(Boolean).join(' · ') };
  });
}

function bytesAt(results, t) {
  const result = latestAt(results, t);
  return { bytes: result?.bytes ?? null,
    code: result === null ? unobserved : result.bytes === null ? valueNotBytes : null };
}

export function toolsInput(node, { graph } = {}) {
  const declared = declaredTools(node.declaration.config);
  const facts = node.toolFacts;
  const results = facts?.results ?? [];
  const outcomes = facts?.observed ? toolOutcomes(node, graph) : [];
  const observed = facts?.observed ? recordedArrivalsText(node) : pageUnavailable;
  const toolRows = rowsAt(declared, outcomes, observed, Infinity);
  const result = bytesAt(results, Infinity);
  return {
    toolDeclared: declared, toolOutcomes: outcomes, toolObserved: observed, toolCode: facts?.observed ? null : unobserved,
    toolResults: results,
    toolRowCodes: toolRows.map(row => row.code),
    preview: { rows: toolRows.map(row => [row.name, row.state]),
      ...(result?.bytes == null ? {} : { bytes: result.bytes }) },
  };
}

export function updateToolsView(card, node, t = globalThis.window?.StudyApp?.displayTime?.() ?? Infinity) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="tools"]');
  if (!viewer) return;
  const projected = Boolean(node);
  const observed = (projected && node.toolObserved) || pageUnavailable;
  const rows = projected ? rowsAt(node.toolDeclared ?? [], node.toolOutcomes ?? [], observed, t) : [];
  const toolsSummary = viewer.querySelector('.tools-summary');
  if (toolsSummary) {
    toolsSummary.textContent = toolsCount(node?.toolDeclared?.length ?? 0);
    toolsSummary.title = rows.map(row => row.title).join('\n');
  }
  const code = projected ? node.toolCode ?? null : unobserved;
  viewer.setAttribute('aria-disabled', String(Boolean(code)));
  if (code) viewer.setAttribute('data-reason', code); else viewer.removeAttribute?.('data-reason');
  viewer.title = code ? `${reasonText(code)} · ${observed}` : observed;
  const summary = viewer.querySelector('[data-tool-outcome]');
  if (summary) {
    const outcome = outcomeAt(node?.toolOutcomes ?? [], t);
    const state = summary.querySelector('[data-tool-outcome-state]');
    state.textContent = outcome.state;
    if (outcome.sentence) state.setAttribute('data-reason', outcome.sentence); else state.removeAttribute('data-reason');
    summary.setAttribute('data-reason', outcome.code ?? '');
    summary.title = outcome.title;
  }
  const current = rows.reduce((best, row, index) => row.at !== null
    && (best < 0 || row.at >= rows[best].at) ? index : best, -1);
  const elements = [...viewer.querySelectorAll('.tools-table .viewer-table-row')];
  for (const [index, element] of elements.entries()) {
    element.classList.toggle('current', index === current);
    const row = rows[index];
    if (!row) continue;
    element.firstElementChild.textContent = row.name;
    element.lastElementChild.textContent = row.state;
    if (row.code) element.setAttribute?.('data-reason', row.code); else element.removeAttribute?.('data-reason');
    element.title = row.title;
  }
  const bytes = viewer.querySelector('[data-live="bytes"]');
  if (bytes) {
    const result = bytesAt(node?.toolResults ?? [], t);
    const byteCode = result.code;
    bytes.textContent = bytesText(result.bytes);
    bytes.setAttribute('aria-disabled', String(Boolean(byteCode)));
    bytes.title = byteCode ? `${reasonText(byteCode)} · ${observed}` : observed;
    if (byteCode) bytes.setAttribute('data-reason', byteCode); else bytes.removeAttribute?.('data-reason');
  }
  const spark = viewer.querySelector('.viewer-spark');
  if (spark) {
    const label = code ? `${reasonText(code)} · ${observed}` : observed;
    spark.setAttribute('aria-disabled', String(Boolean(code)));
    spark.setAttribute('aria-label', label);
    spark.title = label;
    if (code) spark.getContext('2d')?.clearRect(0, 0, spark.width, spark.height);
  }
}

export default {
  kind: 'tools',
  render: n => {
    const rows = n.preview?.rows || [];
    const declared = rows.length ? table(['OPERATION', 'STATE'], rows, 'tools-table') : empty(undeclared);
    return `${kicker(n.viewConfig?.heading)}${declared}<div class="viewer-transfer"><span>RESULT BYTES</span><strong data-live="bytes">${bytesText(n.preview?.bytes)}</strong></div>${renderOutcome(n)}${spark}`;
  },
  glance: {
    cells: (n, t = Infinity) => {
      const outcome = outcomeAt(n.toolOutcomes ?? [], t), declared = n.toolDeclared ?? [];
      const rows = rowsAt(declared, n.toolOutcomes ?? [], n.toolObserved ?? pageUnavailable, t);
      return { outcome: outcome.sentence ? { reason: outcome.sentence, title: outcome.title }
        : { reading: outcome.state, caption: 'Outcome', title: outcome.title },
        tools: { words: toolsCount(declared.length), title: rows.map(row => row.title).join('\n') } };
    },
    reduced: ['outcome', 'tools'],
    names: ['outcome'],
  },
  defaultFor: ['tool_executor'],
  reads: ['tools'],
  size: { height: 280, min: 240 },
  input: toolsInput,
  update: updateToolsView,
  tick: 'after',
  page: toolsPage,
};
