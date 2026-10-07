import { authoringSnapshotArgumentsValue } from '@circular/protocol/authoring-query';
import { sameValue } from '@circular/protocol';
import { agentHarnessCandidates, agentHarnessCandidatesPageFromValue, agentHarnesses, agentHarnessesPageFromValue } from '@circular/client';
import { readFirstPage } from './query.mjs';
import { decoded } from './session.mjs';
import { reason } from './reasons.mjs';
import { esc, noticeHTML, refusalText, foundHTML, candidatesRefusedHTML, bindingNoteHTML } from './authoring-forms.mjs';
import { configPath, errorLine } from './config-form.mjs';
import { actorAccess } from './inspect-tab.mjs';

export async function readHarnesses(session) {
  const page = await readFirstPage(session, agentHarnesses.name);
  return decoded(() => agentHarnessesPageFromValue(page)).items
    .map(({ name, program, saved }) => ({ name, program: saved ?? program, saved, held: program }));
}

const arm = value => Array.isArray(value) ? value[0] : undefined;

const needsOf = item => (item?.requirements ?? [])
  .filter(r => r.capability === 'agent_harness' && arm(r.subject) === 1n)
  .map(r => ({ name: r.subject[1], allowed: arm(r.decision) === 1n }));

export function harnessNeeds(page, address) {
  return needsOf(actorAccess(page, address));
}
const decisionCode = need => `agent_harness · ${need.allowed ? 'allowed' : 'denied'}`;

export function projectHarnesses(page, bound, declared = []) {
  const rows = new Map();
  const row = name => rows.get(name) ?? rows.set(name, { name, program: undefined, found: undefined, actors: 0, denied: 0 }).get(name);
  for (const adapter of declared) row(adapter.name).found = adapter.found;
  for (const binding of bound) Object.assign(row(binding.name), { program: binding.program, binding });
  for (const item of page?.items ?? []) {
    const needs = new Map();
    for (const need of needsOf(item)) needs.set(need.name, (needs.get(need.name) ?? true) && need.allowed);
    for (const [name, allowed] of needs) {
      row(name).actors += 1;
      if (!allowed) row(name).denied += 1;
    }
  }
  return [...rows.values()].sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
}

let answered;
export function harnessReading(sdk, revision) {
  if (answered && answered.sdk === sdk && sameValue(answered.revision, revision)) return answered;
  const next = { sdk, revision };
  next.access = readFirstPage(sdk, 'authoring.actor-access', authoringSnapshotArgumentsValue([]))
    .then(page => next.accessValue = { page }, error => next.accessValue = { error: error.code ?? 'READ_UNAVAILABLE' });
  next.bound = readHarnesses(sdk)
    .then(rows => next.boundValue = { rows }, error => next.boundValue = { error: error.code ?? 'READ_UNAVAILABLE' });
  next.candidates = readFirstPage(sdk, agentHarnessCandidates.name)
    .then(page => decoded(() => agentHarnessCandidatesPageFromValue(page)).items.map(({ name, found }) => ({ name, found })))
    .then(rows => next.candidatesValue = { rows }, error => next.candidatesValue = { error: error.code ?? 'READ_UNAVAILABLE' });
  answered = next;
  return next;
}
export const readingSettled = reading => Boolean(reading.accessValue && reading.boundValue && reading.candidatesValue);
export const readingDone = reading => Promise.all([reading.access, reading.bound, reading.candidates]).then(() => reading);
export function forgetReading() { answered = undefined; }
export const readingRows = reading =>
  projectHarnesses(reading.accessValue.page, reading.boundValue.rows ?? [], reading.candidatesValue.rows ?? []);

const entries = new Map();
const entry = name => entries.get(name) ?? entries.set(name, {}).get(name);

export function harnessSectionHTML(node, reading, { historical = false, choose = false } = {}) {
  const section = body => `<section class="detail-section harness-binding" data-harness-binding="${esc(node.id)}">${body}</section>`;
  if (!readingSettled(reading)) return section(noticeHTML('Reading whether this actor needs a harness…', 'pending'));
  const access = reading.accessValue, bound = reading.boundValue, candidates = reading.candidatesValue;
  const needs = access.page ? harnessNeeds(access.page, node.address) : [];
  if (!needs.length) return '';
  if (bound.error) {
    const why = reason(bound.error);
    return section(`<div class="detail-heading"><h3>Harness</h3></div>${noticeHTML(why.label, 'failed', why.code)}`);
  }
  const binding = name => bound.rows.find(b => b.name === name);
  const program = name => binding(name)?.program;
  const found = name => candidates.rows?.find(c => c.name === name)?.found;
  const row = (name, value) => `<div class="detail-row" data-harness-need="${esc(name)}"><span>${esc(name)}</span><span title="${esc(value)}">${esc(value)}</span></div>`;
  const others = bound.rows.filter(b => !needs.some(n => n.name === b.name)).map(b => row(b.name, b.program) + bindingNoteHTML(b)).join('');
  return section(`<div class="detail-heading"><h3>Harness</h3></div>`
    + needs.map(need => program(need.name)
      ? row(need.name, program(need.name)) + bindingNoteHTML(binding(need.name))
        + (need.allowed ? '' : noticeHTML(`The daemon does not run ${need.name} for this agent.`, 'failed', decisionCode(need)))
      : row(need.name, 'Not bound') + unboundHTML(need, historical, choose, found(need.name))).join('')
    + (candidates.error !== undefined && needs.some(need => !program(need.name)) ? candidatesRefusedHTML(candidates.error) : '')
    + (others ? `<p class="subtle-note">Also bound in this project</p>${others}` : ''));
}

function unboundHTML(need, historical, choose, found) {
  return (need.allowed ? '' : noticeHTML(`This agent cannot start: no program is bound to ${need.name}`, 'failed', decisionCode(need)))
    + bindFormHTML(need.name, 'data-bind-agent-harness', historical, choose, found);
}

export function bindFormHTML(name, attribute, historical, choose, found) {
  const { program = '', pending, refusal } = entries.get(name) ?? {};
  const off = historical || pending ? ' disabled' : '';
  const refused = refusal && refusalText(refusal);
  const line = refused && errorLine(refused.text, refused.code, refused.said);
  const onProgram = line && configPath(refusal.at)?.[0] === 'program';
  const answer = pending ? noticeHTML(`Binding ${name}…`, 'pending') : line && !onProgram ? line : '';
  return `<form ${attribute}="${esc(name)}" novalidate>`
    + foundHTML(found, off)
    + `<label class="config-field"><span class="config-label">Program</span><input name="harness-program" value="${esc(program)}" autocomplete="off" spellcheck="false" placeholder="/absolute/path/to/${esc(name)}"${historical ? ' disabled' : ''}>`
    + `<small>In a terminal, <code>command -v ${esc(name)}</code> prints the path.</small>${onProgram ? line : ''}</label>`
    + '<p class="subtle-note">The program keeps its own sign-in. If it has not signed in on this machine yet, run it once in a terminal first.</p>'
    + `<div data-harness-answer>${answer}</div>`
    + `<div class="dialog-actions">${choose ? `<button class="quiet-button" type="button" data-harness-choose${off}>Choose…</button>` : ''}`
    + `<button class="dark-button" type="submit"${off}>Bind ${esc(name)}</button></div></form>`;
}

function typeProgram(name, program) { entry(name).program = program; }
async function chooseProgram(name, context) {
  let program;
  try { program = await context.choose(name); }
  catch (error) { entry(name).refusal = { code: error.code ?? 'TRANSPORT_FAILED' }; return; }
  if (program) entry(name).program = program;
}
function useFound(name) {
  const found = answered?.candidatesValue?.rows?.find(row => row.name === name)?.found;
  if (found) entry(name).program = found;
}
export async function bindProgram(name, program, context) {
  Object.assign(entry(name), { program, pending: true, refusal: undefined });
  const answer = await context.bind(name, program.trim());
  if (answer === true) { entries.delete(name); forgetReading(); }
  else Object.assign(entry(name), { pending: false, refusal: answer });
  return answer;
}

const attached = new WeakMap();
export function bindHarnessForms(root, attribute, surface) {
  if (!root?.addEventListener) return;
  const done = attached.get(root) ?? attached.set(root, new Set()).get(root);
  if (done.has(attribute)) return;
  done.add(attribute);
  const formOf = target => target?.closest?.(`form[${attribute}]`);
  root.addEventListener('input', event => {
    const form = formOf(event.target);
    if (form && event.target.name === 'harness-program') typeProgram(form.getAttribute(attribute), event.target.value);
  });
  root.addEventListener('click', async event => {
    const choose = event.target?.closest?.('[data-harness-choose]');
    const form = formOf(choose ?? event.target?.closest?.('[data-harness-use-found]')), at = form && surface();
    if (!at) return;
    if (choose) await chooseProgram(form.getAttribute(attribute), at.context);
    else useFound(form.getAttribute(attribute));
    at.redraw();
  });
  root.addEventListener('submit', async event => {
    const form = formOf(event.target);
    if (!form) return;
    event.preventDefault();
    const at = surface();
    if (!at) return;
    const outcome = bindProgram(form.getAttribute(attribute), form.elements.namedItem('harness-program').value, at.context);
    at.redraw();
    await outcome;
    at.redraw();
  });
}
