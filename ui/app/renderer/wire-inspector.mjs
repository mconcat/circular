import { identity } from './query.mjs';
import { reason } from './reasons.mjs';
import { decodeDeadLetters, preprocessFailurePoint } from '@circular/protocol';
import { decoded, readDeadLetters } from './session.mjs';

export async function readProblems(session, limit, lens) {
  try {
    const page = await readDeadLetters(session, limit, lens);
    const { rows } = decoded(() => decodeDeadLetters(page.anchor, page.items));
    const points = await Promise.all(rows.map(row =>
      row.reason.code === 'processing' ? decoded(() => preprocessFailurePoint(row.reason.detail)) : null));
    return { rows: rows.map((row, i) => ({ ...row, point: points[i] })), diagnostic: null };
  } catch (error) {
    return { rows: [], diagnostic: error.code ?? 'READ_UNAVAILABLE' };
  }
}

const wireOf = edge => Array.isArray(edge) && edge[0] === 1n ? identity(edge) : null;

export function wireProblems(problems) {
  const byWire = new Map();
  for (const row of problems?.rows ?? []) {
    const wire = wireOf(row.point?.edge) ?? wireOf(row.target);
    if (!wire) continue;
    if (!byWire.has(wire)) byWire.set(wire, []);
    byWire.get(wire).push(row);
  }
  return byWire;
}

export const failureCode = row => row.point?.code ?? null;
export function wireIssue(rows) {
  const row = rows?.at(-1);
  if (!row) return undefined;
  const step = row.point ? `at step ${String(row.point.index)} (${row.point.kind})` : null;
  return { code: row.reason.code, failure: failureCode(row), count: rows.length, step: row.point?.kind ?? null,
    ordinal: String(row.ordinal), message: [reason(row.reason.code).label, step].filter(Boolean).join(' ') };
}

export function issueUnobserved(problems) {
  const code = problems ? problems.diagnostic : 'READ_UNAVAILABLE';
  if (!code) return undefined;
  const value = reason(code);
  return { code: value.code, message: value.label };
}

export function markIssueLayer(root, problems) {
  const layer = root.querySelector('#wire-issues');
  if (!layer?.setAttribute) return;
  const unobserved = issueUnobserved(problems);
  if (unobserved) layer.setAttribute('data-code', unobserved.code);
  else layer.removeAttribute?.('data-code');
}

export const stepCue = step => Object.values(step?.config ?? {})
  .map(value => typeof value === 'string' ? value : JSON.stringify(value)).join(' · ');

export const rawDeclaration = edge => JSON.stringify(edge.declaration, null, 2);

const write = (element, text) => {
  if (element && element.textContent !== text) element.textContent = text;
};

const stepLine = step => [step.kind, stepCue(step)].filter(Boolean).join(' · ');

export function updateWireInspector(root, edge) {
  const step = id => edge.combinators.find(c => c.id === id);
  for (const row of root.querySelectorAll('#inspector .processing-row:not(.draft)'))
    write(row.querySelector('small'), stepCue(step(row.dataset.comb)));
  const active = step(root.querySelector('#inspector #preprocess-form')?.dataset.component);
  if (active) write(root.querySelector('#inspector .processing-caption > span'), stepLine(active));
  if (edge.declaration)
    write(root.querySelector('#inspector .inspector-content > details:not(.wire-policy) > pre.code-block'),
      rawDeclaration(edge));
}
