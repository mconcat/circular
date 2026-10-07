import { CircularUInt } from '@circular/protocol';
import { reason } from './reasons.mjs';
import { identity } from './query.mjs';
import { addressPath } from './scene.mjs';
import { escape, kicker, empty, objectMembers, atDetail, places, ONE_LINE, ROW } from './view-registry.mjs';
import { DETAIL, NAMES } from './tier.mjs';
import { viewer } from './viewer.mjs';

export const submitCode = 'INLET_INJECTION_UNAVAILABLE';
const inputs = { bool: 'checkbox', int: 'integer', uint: 'integer', float: 'float', string: 'text' };

export function formFields(node) {
  const seen = new Set();
  return (node.out ?? []).flatMap(([, flow]) => objectMembers(flow)).filter(({ name }) => !seen.has(name) && seen.add(name))
    .map(({ name, shape }) => {
      const base = shape.kind === 'Base' ? shape.base : null;
      return { name, input: inputs[base] ?? 'text', base };
    });
}

export function requestMount(node, mounts = []) {
  const ports = new Set((node.out ?? []).map(([id]) => id));
  return mounts.find(m => m.declaration.roles.request
    && identity(m.declaration.roles.request.actor) === node.id && ports.has(m.declaration.roles.request.port))?.address;
}

export function formInput(node, { mounts }) {
  const mount = requestMount(node, mounts);
  return { formFields: formFields(node), ...(mount === undefined ? { formSubmitCode: submitCode } : { formMount: mount }) };
}

export function formPayload(fields, entered) {
  return Object.fromEntries(fields.filter(f => Object.hasOwn(entered, f.name)).map(({ name, base }) => {
    const value = entered[name];
    if (base === 'int') return [name, BigInt(value)];
    if (base === 'uint') return [name, new CircularUInt(BigInt(value))];
    if (base === 'float') return [name, Number(value)];
    return [name, value];
  }));
}

export function formSubmission(node, entered, { mounts }) {
  const mount = requestMount(node, mounts);
  if (mount === undefined) return { code: submitCode };
  try { return { mount, payload: formPayload(formFields(node), entered) }; }
  catch { return { code: 'CONFIG_VALUE_INVALID' }; }
}

export function answered(id, answer) {
  viewer.answer(id, 'form', answer);
}

export function updateFormView(card, node) {
  if (!node) return;
  const body = card?.querySelector('.node-viewer[data-viewer="form"]');
  if (!body) return;
  const output = body.querySelector('.typed-form output');
  if (node.formSubmitCode === undefined) {
    const answer = viewer.card(node.id).answers?.form;
    if (output && answer && output.textContent !== answer.text) output.textContent = answer.text;
    if (output && answer && output.title !== answer.title) output.title = answer.title;
    if (answer?.code) output?.setAttribute?.('data-reason', answer.code); else output?.removeAttribute?.('data-reason');
    return;
  }
  const code = reason(node.formSubmitCode), title = code.label;
  output?.setAttribute?.('data-reason', code.code);
  if (output && output.textContent !== code.label) output.textContent = code.label;
  if (output && output.title !== title) output.title = title;
}

export default {
  kind: 'form',
  render: (n, own, tier, accepts = true) => {
    const fields = n.formFields || [], detail = tier === DETAIL, names = tier === NAMES;
    if (!fields.length) return atDetail(tier, kicker(n.viewConfig?.heading)) + empty('FORM_FIELDS_UNDECLARED', '', tier);
    const field = f => `<label${detail ? '' : ' style="flex:none;height:var(--resize-handle);margin:0"'}>${detail ? escape(f.name) : `<span style="flex:0 1 auto;${ONE_LINE}">${escape(f.name)}</span>`}<input name="${escape(f.name)}" ${f.input === 'checkbox' ? 'type="checkbox"' : `type="${f.input === 'text' ? 'text' : 'number'}" required`} ${f.input === 'float' ? 'step="any"' : f.input === 'integer' ? 'step="1"' : ''}${detail || f.input === 'checkbox' ? '' : ' style="flex:1 1 50%;width:auto;min-width:0;height:100%;padding:0 4px"'}></label>`;
    const code = n.formSubmitCode && reason(n.formSubmitCode);
    const send = face => `<button class="viewer-action" type="submit"${code ? ` disabled data-reason="${escape(code.code)}" title="${escape(code.label)}"` : ` title="${escape(n.formMount ? addressPath(n.formMount) : '')}"`}${face}</button>`;
    const output = `<output${code ? ` data-reason="${escape(code.code)}" title="${escape(code.label)}"` : ''}>${escape(code ? code.label : '')}</output>`;
    return detail
      ? kicker(n.viewConfig?.heading) + `<form class="typed-form" data-typed-form="${n.id}">${fields.map(field).join('')}${send('>Send value ↑')}${output}</form>`
      : !accepts ? `<div class="typed-form" style="display:flex;flex-direction:column;flex:1 1 auto;min-height:0"><p class="form-reading" style="${ROW};${ONE_LINE}" title="${escape(fields.map(f => f.name).join(' · '))}">${escape(fields.map(f => f.name).join(' · '))}</p>${output}</div>`
      : `<form class="typed-form" data-typed-form="${n.id}" style="display:flex;flex-direction:column;flex:1 1 auto${names ? ';min-height:0' : ''}">${places(`<div style="display:flex;flex-direction:column${names ? ';overflow-y:auto' : ''}">${fields.map(field).join('')}</div>${send(' aria-label="Send value">↑')}`, true)}${output}</form>`;
  },
  defaultFor: ['form'],
  traits: { interactive: true, inlineSettings: false, injects: true },
  size: { height: 248, min: 174 },
  input: formInput,
  update: updateFormView,
  submit: formSubmission,
  answered,
};
