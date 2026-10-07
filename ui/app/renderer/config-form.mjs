import { escape, empty, durationShown, durationUnitName, durationUnitSelect } from './view-registry.mjs';
import { reasonText } from './reasons.mjs';
import { GRANT_APPROVAL } from './config-fields.mjs';

const scalarControl = (kind, attrs, value) =>
  kind === 'boolean'
    ? `<select ${attrs}>${['true', 'false'].map(v => `<option ${String(value) === v ? 'selected' : ''}>${v}</option>`).join('')}</select>`
    : kind === 'integer' || kind === 'number'
      ? `<input ${attrs} type="number" step="${kind === 'integer' ? '1' : 'any'}" value="${escape(value)}" required>`
      : `<input ${attrs} value="${escape(value ?? '')}">`;

export const fieldName = ({ label, key }) => label ?? key;

export function labelLine(field, required = false, trailing = '') {
  const words = field.label == null
    ? `<code class="config-key">${escape(fieldName(field))}</code>`
    : `<span class="config-label-text"${field.key == null ? '' : ` title="${escape(field.key)}"`}>${escape(field.label)}</span>`;
  return `<span class="config-label">${words}${required ? '<span class="config-required" aria-hidden="true">*</span>' : ''}${trailing}</span>`;
}

const kindWords = { string: 'text', integer: 'integer', number: 'number', boolean: 'true / false', object: 'object', list: 'list' };
const kindSelect = r =>
  r.retype
    ? `<select class="config-kind" data-config-kind="${escape(r.retype)}" aria-label="${escape(fieldName(r))} kind" title="Kind of value">${Object.entries(kindWords).map(([k, word]) => `<option value="${k}" ${k === r.held ? 'selected' : ''}>${word}</option>`).join('')}</select>`
    : '';

export const configPath = at => Array.isArray(at) && at.length && at.every(s => Array.isArray(s) && s.length === 2)
  ? at.map(([arm, segment]) => arm === 2n ? Number(segment) : String(segment)) : undefined;
export const errorLine = (text, code, said) => `<small class="field-error"${code == null || code === '' ? '' : ` data-reason="${escape(code)}"`}>${escape(text)}`
  + `${said ? `<span class="field-error-said">${escape(said)}</span>` : ''}</small>`;

export function fieldRow(field, control, { tag = 'div', className = 'config-field', required = false, notes = '', attrs = '', error } = {}) {
  const group = tag === 'label' ? 'span' : 'div';
  return `<${tag} class="${className}${required ? ' required' : ''}"${attrs}>${labelLine(field, required)}${control ? `<${group} class="config-control">${control}</${group}>` : ''}`
    + `${field.description ? `<small class="config-help">${escape(field.description)}</small>` : ''}${notes ? `<small>${escape(notes)}</small>` : ''}`
    + `${error ? errorLine(error.text ?? reasonText(error.code), error.code, error.said) : ''}</${tag}>`;
}

const removeButton = r => r.remove
  ? `<button type="button" class="config-remove" data-config-remove="${escape(r.remove)}" aria-label="Remove ${escape(r.label)}" title="Remove">×</button>`
  : '';
function itemHTML(r, control, members = '', why) {
  const remove = removeButton(r), inLine = !control && !r.item;
  const line = r.item ? '' : labelLine({ key: r.label }, false, inLine ? remove : '');
  const row = control || (!inLine && remove) ? `<div class="config-control">${control}${inLine ? '' : remove}</div>` : '';
  return `<div class="config-item${r.row === 'group' ? ' config-group' : ''}"${why ? ` data-reason="${escape(why.code)}"` : ''}>${line}${row}${members}${why ? `<small>${escape(why.label)}</small>` : ''}</div>`;
}
function insertHTML(r) {
  const add = `<button type="button" class="config-add" data-config-insert="${escape(r.target)}"${r.key ? ` data-config-key="${escape(r.key)}"` : ''}${r.keyName ? ` data-config-key-name="${escape(r.keyName)}"` : ''}>+ Add ${r.key ? `<code class="config-key">${escape(r.key)}</code>` : escape(r.label)}</button>`;
  return r.keyName
    ? `<div class="config-add-entry"><input name="${escape(r.keyName)}" aria-label="New ${escape(r.label)} key" placeholder="key">${add}</div>`
    : add;
}
function structureHTML(rows, raw, label) {
  let at = 0;
  const level = depth => {
    let html = '';
    while (at < rows.length && rows[at].depth >= depth) {
      const r = rows[at++];
      html += r.row === 'group' ? itemHTML(r, kindSelect(r), level(depth + 1))
        : r.row === 'leaf' ? itemHTML(r, kindSelect(r) + unitControl(r.kind, `name="${escape(r.name)}" aria-label="${escape(r.label)}"`, raw?.[r.name] ?? r.value,
          r.duration, r.name, raw, r.label))
          : r.row === 'fixed' ? itemHTML(r, '', '', label(r.code))
            : insertHTML(r);
    }
    return `<div class="config-structure">${html}</div>`;
  };
  return level(0);
}

const heldOutside = (value, options) => typeof value === 'string' && value !== '' && !options.includes(value);
const heldReason = (value, options, label) => heldOutside(value, options) ? label('CHOICE_NOT_PUBLISHED') : undefined;
const choiceControl = (attrs, value, options, required, absent) => {
  const held = heldOutside(value, options) ? [value] : [];
  return `<select ${attrs}><option value="" ${value === '' || value == null ? 'selected' : ''}>${required ? 'choose…' : escape(absent ?? 'default')}</option>${[...options, ...held].map(v => `<option value="${escape(v)}" ${v === value ? 'selected' : ''}>${escape(v)}</option>`).join('')}</select>`;
};
const msText = value => value == null ? '' : typeof value?.value === 'bigint' ? String(value.value) : String(value);
function unitControl(kind, attrs, value, duration, name, raw, label) {
  if (!duration) return scalarControl(kind, attrs, value);
  const unitName = durationUnitName(name), typed = raw?.[name] !== undefined && raw?.[unitName] !== undefined;
  const { text, unit } = typed ? { text: value, unit: raw[unitName] } : durationShown(msText(value));
  return `<span class="config-unit-input">${scalarControl(kind, `${attrs} data-duration`, text)}${durationUnitSelect(unitName, unit, label)}</span>`;
}
const approvalAll = values => values.every(v => v === GRANT_APPROVAL.all)
  ? { value: GRANT_APPROVAL.none, text: 'Clear all' } : { value: GRANT_APPROVAL.all, text: 'Require approval for all' };
const approvalAllHTML = values => {
  const next = approvalAll(values);
  return `<div class="config-approval-all"><button type="button" class="quiet-button" data-approval-all="${escape(next.value)}">${escape(next.text)}</button></div>`;
};
const approvalChoices = group => [...group.querySelectorAll('select[data-grant-approval]')];
export function refreshApprovalAll(control) {
  const group = control?.closest?.('[data-grant-approvals]'), button = group?.querySelector('[data-approval-all]');
  if (!button) return;
  const next = approvalAll(approvalChoices(group).map(select => select.value));
  button.dataset.approvalAll = next.value;
  button.textContent = next.text;
}
export function setAllApprovals(button) {
  const choices = approvalChoices(button.closest('[data-grant-approvals]'));
  for (const select of choices) select.value = button.dataset.approvalAll;
  refreshApprovalAll(button);
  choices[0]?.dispatchEvent(new Event('change', { bubbles: true }));
}
const policyName = p => p.inputs.find(i => i.group)?.group;
function grantRows(field, raw, label) {
  const approvals = field.policies.flatMap(p => p.inputs.filter(i => i.approval).map(i => raw?.[i.name] ?? i.value));
  const all = approvals.length > 1 ? approvalAllHTML(approvals) : '';
  return `<div class="config-structure"${all ? ' data-grant-approvals' : ''}>${field.policies
    .map(p => {
      const grant = raw?.[p.grant] ?? (p.granted ? 'granted' : '');
      const heading = policyName(p) ?? p.name;
      const inputs = p.inputs
        .map(i => {
          const attrs = `name="${escape(i.name)}" aria-label="${escape(heading)} ${escape(fieldName(i))}"${i.required ? ' aria-required="true"' : ''}${i.approval ? ' data-grant-approval' : ''}`,
            value = raw?.[i.name] ?? i.value;
          const control =
            i.kind === 'choice'
              ? choiceControl(attrs, value, i.options, true)
              : i.kind === 'lines'
                ? `<textarea ${attrs} rows="2" spellcheck="false">${escape(value)}</textarea>`
                : unitControl(i.kind, attrs, value, i.duration, i.name, raw, `${heading} ${fieldName(i)}`);
          const why = i.kind === 'choice' ? heldReason(value, i.options, label) : undefined;
          return fieldRow({ key: i.key, label: i.label, description: i.description }, control, { tag: 'label', className: 'config-field config-policy-input', required: i.required,
            notes: [why?.label, i.hint ? (i.kind === 'lines' ? `one per line · ${i.hint}` : i.hint) : ''].filter(Boolean).join(' · '),
            attrs: why ? ` data-reason="${escape(why.code)}"` : '' });
        })
        .join('');
      return `<div class="config-policy" role="group" aria-label="${escape(heading)}"><div class="config-policy-heading">${policyName(p) ? `<span class="config-label-text" title="${escape(p.name)}">${escape(heading)}</span>` : `<code class="config-key">${escape(p.name)}</code>`}<select name="${escape(p.grant)}" aria-label="${escape(heading)} grant"><option value="" ${grant === '' ? 'selected' : ''}>not granted</option><option value="granted" ${grant === 'granted' ? 'selected' : ''}>granted</option></select></div>${inputs ? `<div class="config-structure">${inputs}</div>` : ''}</div>`;
    })
    .join('')}${all}</div>`;
}

function typeFieldRows(rows, options, insert, raw) {
  return `<div class="form-field-editor">${rows.map((row, i) => {
    const name = raw?.[row.nameInput] ?? row.name, base = raw?.[row.typeInput] ?? row.base;
    return `<div class="config-control"><input name="${escape(row.nameInput)}" data-field-name="${i}" aria-label="Field ${i + 1} name" value="${escape(name)}"><select name="${escape(row.typeInput)}" data-field-shape="${i}" aria-label="Field ${i + 1} type">${options.map(type => `<option value="${type}" ${type === base ? 'selected' : ''}>${type}</option>`).join('')}</select><button type="button" class="config-remove" data-config-remove="${escape(row.remove)}" aria-label="Remove field ${i + 1}" title="Remove">×</button></div>`;
  }).join('')}<button type="button" class="config-add" data-config-insert="${escape(insert)}">+ Add field</button></div>`;
}

function grouped(fields, rowOf) {
  const sections = new Map([[null, []]]);
  for (const field of fields) {
    const group = field.group ?? null;
    if (!sections.has(group)) sections.set(group, []);
    sections.get(group).push(rowOf(field));
  }
  return [...sections].map(([group, rows]) => group === null ? rows.join('')
    : `<div class="config-section" role="group" aria-label="${escape(group)}"><div class="config-section-heading">${escape(group)}</div>${rows.join('')}</div>`).join('');
}

export function fieldsHTML(answer, config, raw, label, refused = null) {
  return grouped(answer.fields, ({ key, kind, code, rows, options, duration, hint, required, expression, ...field }) => {
    const error = refused?.slot === key ? refused : undefined;
    const value = raw?.[key] ?? config?.[key],
      attrs = `name="${escape(key)}" data-config-field="${escape(key)}" aria-label="${escape(fieldName({ ...field, key }))}"${required ? ' aria-required="true"' : ''}${error ? ' aria-invalid="true"' : ''}`;
    const control =
      kindSelect({ ...field, key }) +
      (kind === 'type-fields'
        ? typeFieldRows(rows, options, field.insert, raw)
        : kind === 'structured'
          ? structureHTML(rows, raw, label)
          : kind === 'grants'
            ? grantRows({ policies: field.policies }, raw, label)
            : kind === 'fixed'
              ? ''
              : kind === 'choice'
                ? choiceControl(attrs, value ?? '', options, required, field.absent)
                : kind === 'base-stream'
                  ? choiceControl(attrs, raw?.[key] ?? field.base, options, required, field.absent)
                  : unitControl(kind, attrs, value, duration, key, raw, fieldName({ ...field, key })));
    const tag = kind === 'structured' || kind === 'grants' || kind === 'type-fields' ? 'div' : 'label';
    const why = code ?? (kind === 'choice' ? heldReason(value ?? '', options, label) : undefined);
    const notes = [why?.label, hint].filter(Boolean).join(' · ');
    const marks = ` data-config-row="${escape(key)}"` + (why ? ` data-reason="${escape(why.code)}"` : '') + (expression ? ' data-config-expression' : '');
    return fieldRow({ ...field, key }, control, { tag, required, notes, attrs: marks, error });
  });
}

export function configFormHTML(answer, n, draft, raw, label, submission) {
  if (!answer.declared || (!answer.fields.length && !answer.code))
    return empty('CONFIG_SLOTS_UNDECLARED');
  if (!answer.fields.length)
    return `<p class="subtle-note" data-reason="${escape(answer.code.code)}" title="${escape(answer.code.label)}">${escape(answer.code.label)}</p>`;
  const refused = submission?.field === undefined ? n.issue?.detail ?? null
    : { slot: submission.field, code: submission.code, text: submission.message, said: submission.said };
  return fieldsHTML(answer, { ...n.config, ...draft }, raw, label, refused);
}
