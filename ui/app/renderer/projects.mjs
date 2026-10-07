import { reason } from './reasons.mjs';
import { empty } from './view-registry.mjs';
import { esc, noticeHTML, candidatesRefusedHTML, bindingNoteHTML } from './authoring-forms.mjs';
import { readingRows, readingSettled, forgetReading, bindFormHTML, bindHarnessForms } from './harness-binding.mjs';
import { journalCeiling } from './journal-ceiling.mjs';

export const baseName = path => path.replace(/\/+$/, '').split('/').at(-1) || '/';

export function bindProjects(root, { current, controls } = {}) {
  return () => {
    const page = root.querySelector('#projects-view');
    if (controls && page?.querySelector) bindProjectControls(page, current, controls);
  };
}
export const stateLine = (path, current, code) =>
  reason(path === current ? code ?? 'DAEMON_ANSWERING' : 'PROJECT_NOT_OPEN');
export const recentRefusal = code => code === undefined ? undefined : reason(code);

export function projectDaemonHTML(facts) {
  const actions = [
    facts.start && ['start-daemon', 'Start daemon'],
    facts.running && ['restart-daemon', 'Restart daemon'],
    facts.running && ['stop-daemon', 'Stop daemon'],
  ].filter(Boolean);
  return actions.length ? `<span class="card-daemon" data-card-daemon>${actions.map(([action, label]) =>
    `<button class="quiet-button" type="button" data-action="${action}">${label}</button>`).join('')}</span>` : '';
}

export function projectPaneHTML(current, facts) {
  return `<section class="project-pane" data-project-pane="${esc(current)}"><h2>${esc(baseName(current))}</h2>`
    + `<div class="project-pane-grid"><section class="detail-section" data-project-harnesses>${projectHarnessesHTML(facts)}</section>`
    + `<section class="detail-section" data-project-settings>${settingsHTML(current, facts)}</section></div></section>`;
}

export function projectHarnessesHTML(facts, { titled = true } = {}) {
  const harnessesHeading = (refresh = '') => titled || refresh
    ? `<div class="detail-heading">${titled ? '<h3>Harnesses</h3>' : ''}${refresh}</div>` : '';
  const refreshHTML = '<button class="quiet-button" type="button" data-harness-refresh>Refresh</button>';
  const reading = facts.harness;
  if (!reading) return harnessesHeading() + empty('HARNESSES_UNREAD');
  if (!readingSettled(reading)) return harnessesHeading() + noticeHTML('Reading this project’s harnesses…', 'pending');
  const access = reading.accessValue, bound = reading.boundValue, candidates = reading.candidatesValue;
  const rows = readingRows(reading);
  const refused = [access.error, bound.error].filter(code => code !== undefined)
    .map(code => { const why = reason(code); return noticeHTML(why.label, 'failed', why.code); }).join('')
    + (candidates.error === undefined ? '' : candidatesRefusedHTML(candidates.error));
  const needed = row => !access.page ? 'Not read'
    : !row.actors ? 'No actor'
      : `${row.actors} ${row.actors === 1 ? 'actor' : 'actors'}${row.denied ? ` · ${row.denied} denied` : ''}`;
  const cells = row => `<td><code>${esc(row.name)}</code></td>`
    + `<td${row.program ? ` title="${esc(row.program)}"` : ''}>${row.program ? `<code>${esc(row.program)}</code>${bindingNoteHTML(row.binding)}` : bound.error ? 'Not read' : 'Not bound'}</td>`
    + `<td${row.denied ? ' data-reason="agent_harness · denied"' : ''}>${needed(row)}</td>`;
  const body = rows.map(row => `<tr data-harness-row="${esc(row.name)}" data-binding="${row.program ? 'bound' : 'unbound'}">${cells(row)}</tr>`
    + (row.program || bound.error ? ''
      : `<tr class="harness-bind-row"><td colspan="3">${bindFormHTML(row.name, 'data-project-harness-bind', false, facts.choose, row.found)}</td></tr>`)).join('');
  return harnessesHeading(refreshHTML)
    + refused
    + (rows.length
      ? `<div class="harness-table-wrap"><table class="harness-table"><thead><tr><th>Harness</th><th>Program in this project</th><th>Needed by</th></tr></thead><tbody>${body}</tbody></table></div>`
      : refused ? '' : `<div data-harnesses-empty>${empty('HARNESSES_UNDECLARED')}</div>`);
}

const revealed = new Map();
const whole = value => String(value != null && typeof value === 'object' && 'value' in value ? value.value : value);
function settingsHTML(current, { code, health, reveal }) {
  const file = `${current.replace(/\/+$/, '')}/config.toml`;
  const defaults = health?.anchor?.config_defaults;
  const [inForce, reported] = code !== undefined || !health ? ['Not read', 'unread']
    : !Array.isArray(defaults) ? ['This daemon did not report them', 'unreported']
      : !defaults.length ? ['None', 'none']
        : [defaults.map(entry => `${entry.key} = ${whole(entry.value)}`), 'reported'];
  const ceiling = code === undefined ? journalCeiling(health) : null;
  const shown = revealed.get(current), refusal = shown?.code === undefined ? undefined : reason(shown.code);
  const row = (label, value, attributes = '') => `<div class="detail-row"${attributes}><span>${label}</span><span>${value}</span></div>`;
  return '<div class="detail-heading"><h3>Settings</h3></div>'
    + row('config.toml', `<code title="${esc(file)}">${esc(file)}</code>`, ' data-config-file')
    + row('Defaults in force', Array.isArray(inForce) ? inForce.map(line => `<code>${esc(line)}</code>`).join('<br>') : esc(inForce),
      ` data-config-defaults="${reported}"`)
    + (ceiling ? row('Journal', esc(ceiling.text), ` data-journal="${esc(ceiling.code)}"`) : '')
    + '<p class="subtle-note">Edit config.toml in your own editor; Restart daemon applies an edit.</p>'
    + `<div class="dialog-actions"><button class="quiet-button" type="button" data-reveal-config${reveal ? '' : ' disabled'}>Reveal in Finder</button></div>`
    + `<div data-reveal-answer="${esc(shown?.revealed ?? '')}">${refusal ? noticeHTML(refusal.label, 'failed', refusal.code)
      : shown?.revealed === 'state' ? noticeHTML(reason('CONFIG_FILE_ABSENT').label, 'pending', 'CONFIG_FILE_ABSENT')
        : ''}</div>`;
}

const wired = new WeakSet();
const once = element => Boolean(element) && !wired.has(element) && Boolean(wired.add(element));
export function bindProjectControls(container, current, controls) {
  bindHarnessForms(container, 'data-project-harness-bind', () => {
    const { context, redraw } = controls();
    return context && { context, redraw };
  });
  if (container.addEventListener && once(container)) container.addEventListener('click', event => {
    if (!event.target?.closest?.('[data-harness-refresh]')) return;
    forgetReading();
    controls().redraw();
  });
  const { reveal: ask, redraw } = controls();
  const reveal = container.querySelector('[data-reveal-config]');
  if (!ask || !once(reveal)) return;
  reveal.addEventListener('click', async () => {
    let shown;
    try { shown = await ask(); }
    catch (error) { shown = { code: error.code ?? 'BRIDGE_UNAVAILABLE' }; }
    revealed.set(current, shown ?? { code: 'BRIDGE_UNAVAILABLE' });
    redraw();
  });
}
