import { reason } from './reasons.mjs';
import { escape, kicker, places, ROW, ONE_LINE } from './view-registry.mjs';
import { DETAIL } from './tier.mjs';
import { requestMount, submitCode } from './view-form.mjs';
import { addressPath } from './scene.mjs';
import { viewer } from './viewer.mjs';

export function promptInput(node, { mounts }) {
  const mount = requestMount(node, mounts);
  return mount === undefined ? { promptSubmitCode: submitCode } : { promptMount: mount };
}

export function promptSubmission(node, entered, { mounts }) {
  const mount = requestMount(node, mounts);
  return mount === undefined ? { code: submitCode } : { mount, payload: String(entered.text ?? '') };
}

export function answered(id, answer) {
  viewer.answer(id, 'prompt', answer);
}

const titled = (element, text) => { if (element && element.title !== text) element.title = text; };
const unsendable = node => node.promptSubmitCode ?? (node.promptMount === undefined ? 'READ_UNAVAILABLE' : null);
function controlAttributes(node) {
  const code = unsendable(node), why = code && reason(code);
  return code ? ` disabled data-reason="${escape(why.code)}" title="${escape(why.label)}"` : ` title="${escape(node.promptMount ? addressPath(node.promptMount) : '')}"`;
}
export function updatePromptView(card, node) {
  const body = card?.querySelector('.node-viewer[data-viewer="prompt"]');
  if (!body || !node) return;
  const code = unsendable(node), why = code && reason(code);
  const answer = code ? { text: why.label, title: why.label, code: why.code } : viewer.card(node.id).answers?.prompt;
  const line = body.querySelector('.viewer-kicker > span');
  if (line && answer && line.textContent !== answer.text) line.textContent = answer.text;
  titled(line, answer?.title ?? '');
  if (answer?.code) line?.setAttribute?.('data-reason', answer.code); else line?.removeAttribute?.('data-reason');
}

export default {
  kind: 'prompt',
  render: (n, own = {}, tier, accepts = true) => {
    const box = form => `<textarea class="prompt-input" data-prompt="${escape(n.id)}" aria-label="Message" spellcheck="false"${form}${controlAttributes(n)}>${escape(own.message ?? '')}</textarea>`;
    const send = face => `<button class="prompt-send" data-send="${n.id}"${controlAttributes(n)}${face}</button>`;
    return tier === DETAIL ? kicker(n.viewConfig?.heading, '') + box('') + send('>↑ <span>Send</span>')
      : !accepts ? `<p class="prompt-reading" style="${ROW};${ONE_LINE}${own.message ? ';color:var(--ink)' : ''}" title="${escape(own.message ?? '')}">${escape(own.message || 'Message')}</p>`
      : places(box(' rows="1" style="padding:0 4px;line-height:1.4;scrollbar-width:none"') + send(' aria-label="Send">↑'), true);
  },
  defaultFor: ['input'],
  traits: { interactive: true, portsAtFoot: true, inlineSettings: false, injects: true },
  size: { height: 215, min: 196 },
  input: promptInput,
  update: updatePromptView,
  tick: 'replace',
  submit: promptSubmission,
  answered,
};
