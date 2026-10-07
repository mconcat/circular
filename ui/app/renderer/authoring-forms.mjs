import { reason, refusalCode } from './reasons.mjs';
import { configFieldList, readConfigForm, changeStructure, leafName } from './config-fields.mjs';
import { fieldRow, configPath, errorLine, setAllApprovals, refreshApprovalAll } from './config-form.mjs';

export const esc = value => String(value ?? '').replace(/[&<>"']/g,
  c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
export const noticeHTML = (text, kind, code) => `<div class="inline-notice ${kind === 'failed' ? 'failure' : kind === 'accepted' ? 'success' : ''}" role="status"${code == null || code === '' ? '' : ` data-reason="${esc(code)}"`}>${esc(text)}</div>`;
export function refusalText(refusal) {
  const code = reason(refusalCode(refusal));
  return { text: code.label, code: code.code, said: refusal?.message || undefined };
}
export const foundHTML = (found, off = '') => found
  ? `<p class="harness-found" data-harness-found>Found: <code title="${esc(found)}">${esc(found)}</code>`
    + `<button class="quiet-button" type="button" data-harness-use-found${off}>Use this path</button></p>`
  : '';
export function bindingNoteHTML({ saved, held } = {}) {
  if (saved === held) return '';
  const code = held === null ? 'HARNESS_NOT_HELD' : saved === null ? 'HARNESS_NOT_SAVED' : 'HARNESS_HELD_OTHER';
  return `<small class="binding-note" data-reason="${code}">${esc(reason(code).label)}`
    + `${code === 'HARNESS_HELD_OTHER' ? `: <code title="${esc(held)}">${esc(held)}</code>` : ''}</small>`;
}
export function candidatesRefusedHTML(code) {
  const why = reason(code);
  return noticeHTML(why.label, 'failed', why.code);
}
export function controlName(at) {
  const path = configPath(at);
  return path && (path.length === 1 ? path[0] : leafName(path));
}

export function openCreateDialog({ dialog, fieldsHTML, title, description, entry, local, place }) {
  let base = { ...(entry.draft ?? {}) };
  for (const field of configFieldList(base, true, entry).fields) {
    if (field.kind === 'structured' && field.required && !Object.hasOwn(base, field.key)) base[field.key] = field.shape.kind === 'Array' ? [] : {};
    if (field.kind === 'grants' && field.required && !Object.hasOwn(base, field.key)) base[field.key] = Object.fromEntries(field.policies.map(policy => [policy.name, {}]));
    if (['string', 'number', 'integer', 'choice', 'boolean'].includes(field.kind) && 'fallback' in field && !Object.hasOwn(base, field.key)) base[field.key] = field.fallback;
  }
  const body = '<form class="dialog-form" data-create-actor novalidate>'
    + (description ? `<p class="create-description">${esc(description)}</p>` : '')
    + '<div class="dialog-form-body">'
    + fieldRow({ label: 'Name' }, `<input data-actor-name value="${esc(local)}" required aria-required="true" autocomplete="off">`, { tag: 'label', className: 'config-field config-name', required: true })
    + '<div class="config-fields create-fields" data-create-fields></div>'
    + '</div>'
    + '<div class="dialog-actions"><div class="dialog-answer" data-create-answer></div>'
    + '<button class="ghost-button" type="button" data-product-close>Cancel</button><button class="primary-button" type="submit">Create</button></div></form>';
  const box = dialog(title, body);
  const form = box.querySelector('form[data-create-actor]');
  const slot = form.querySelector('[data-create-fields]'), answerSlot = form.querySelector('[data-create-answer]');
  const button = form.querySelector('button[type="submit"]');
  const list = () => configFieldList(base, true, entry);
  const typed = () => Object.fromEntries([...form.querySelectorAll('[name]')].map(field => [field.name, field.value]));
  const draw = raw => { slot.innerHTML = fieldsHTML(list(), base, raw); };
  draw();
  const restructure = op => {
    try {
      const fields = list().fields;
      base = changeStructure(readConfigForm(base, form, fields), fields, op);
    } catch (error) { return show({ code: error.code, message: error.message }); }
    const raw = typed();
    for (const name of Object.keys(raw)) if (name.startsWith('[') || op.retype === JSON.stringify([name])) delete raw[name];
    draw(raw);
  };
  slot.addEventListener('click', event => {
    const all = event.target?.closest?.('[data-approval-all]');
    if (all) { event.preventDefault(); setAllApprovals(all); return; }
    const target = event.target?.closest?.('[data-config-insert],[data-config-remove]');
    if (!target) return;
    event.preventDefault();
    const keyName = target.dataset.configKeyName;
    restructure({ insert: target.dataset.configInsert, remove: target.dataset.configRemove,
      key: target.dataset.configKey ?? (keyName ? form.elements.namedItem(keyName)?.value : undefined) });
  });
  slot.addEventListener('change', event => {
    refreshApprovalAll(event.target?.closest?.('select[data-grant-approval]'));
    const target = event.target?.closest?.('[data-config-kind]');
    if (target) restructure({ retype: target.dataset.configKind, kind: target.value });
  });
  form.addEventListener('submit', async event => {
    event.preventDefault();
    for (const marked of form.querySelectorAll('[aria-invalid]')) marked.removeAttribute('aria-invalid');
    for (const error of form.querySelectorAll('.field-error')) error.remove();
    answerSlot.innerHTML = '';
    const name = form.querySelector('[data-actor-name]');
    const required = [[name, 'Give the actor a name.'], ...list().fields
      .filter(field => field.required && ['string', 'number', 'integer', 'choice', 'base-stream', 'boolean'].includes(field.kind))
      .map(field => [form.elements.namedItem(field.key), 'Fill this in to create the actor.'])];
    let first;
    for (const [control, message] of required) {
      if (control && !String(control.value ?? '').trim()) { fieldError(control, errorLine(message)); first ??= control; }
    }
    if (first) { first.focus(); return; }
    let config;
    try { config = readConfigForm(base, form, list().fields); }
    catch (error) { return show({ code: error.code, message: error.message }); }
    button.disabled = true;
    answerSlot.innerHTML = noticeHTML('Creating this actor…', 'pending');
    let answer;
    try { answer = await place(form.querySelector('[data-actor-name]').value.trim(), config); }
    catch (error) { answer = { code: error.code ?? 'EDIT_UNAVAILABLE' }; }
    button.disabled = false;
    if (answer === true) { box.close(); return; }
    show(answer ?? { code: 'EDIT_UNAVAILABLE' });
  });
  function show(refusal) {
    const path = configPath(refusal.at), name = controlName(refusal.at);
    const control = name !== undefined ? form.elements.namedItem(name) : null;
    const row = control ? null : path && [...form.querySelectorAll('[data-config-row]')].find(r => r.dataset.configRow === String(path[0]));
    const { text, code, said } = refusalText(refusal);
    console.info(code, ...(said === undefined ? [] : [said]));
    const line = errorLine(text, code, said);
    answerSlot.innerHTML = '';
    if (control) { fieldError(control, line); control.focus(); }
    else if (row) { row.insertAdjacentHTML('beforeend', line); row.querySelector('input, select, textarea, button')?.focus(); }
    else answerSlot.innerHTML = line;
  }
  function fieldError(control, line) {
    control.setAttribute('aria-invalid', 'true');
    const row = control.closest?.('.config-field');
    (row ?? control).insertAdjacentHTML(row ? 'beforeend' : 'afterend', line);
  }
  return box;
}
