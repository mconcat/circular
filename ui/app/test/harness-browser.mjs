import assert from 'node:assert/strict';
import { editDaemon } from './edit-peer.mjs';
import { fixture, healthPage } from './fixtures.mjs';

export const state = '/s/project';
export const search = (extra = {}) => '?' + new URLSearchParams({ state, recent: JSON.stringify([state]), theme: 'light', ...extra });
const requirement = name => ({ capability: 'agent_harness', decision: [2n, `No program is bound to ${name}.`], subject: [1n, name],
  authority: 1n, condition: 1n, rule_index: 0n });
export function daemonWith({ declared = [], bound = [], needs = [], candidates, reject, standing = true } = {}) {
  const f = fixture();
  f.health = healthPage();
  f.events = { anchor: null, items: [] };
  f.harnessCandidates = declared;
  f.harnesses = new Map(bound);
  if (!standing) f.system = null;
  const unobserved = { rejected: { code: 22n, message: 'This witness does not observe it' } };
  f.queries = { 'timeline.bins': unobserved, 'timeline.at': unobserved,
    'authoring.actor-access': { items: needs.length
      ? [{ actor: [1n, { scope: [[1n, 'desk']], local: 'a' }], requirements: needs.map(requirement) }] : [] },
    ...(candidates ? { 'agent.harness-candidates': candidates } : {}) };
  return editDaemon({ fixtureValue: f, reject, quietFeeds: true });
}
export const asked = (daemon, name) => daemon.queries.filter(query => query.name === name).length;
export const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

export function page(evaluate, call) {
  const q = JSON.stringify;
  const wait = async (expression, what = expression) => {
    for (let turn = 0; turn < 200; turn += 1) {
      const value = await evaluate(expression);
      if (value) return value;
      await pause(20);
    }
    assert.fail(`The page did not show: ${what}`);
  };
  const press = async selector => {
    await wait(`!!document.querySelector(${q(selector)})?.getClientRects().length`, selector);
    const at = await evaluate(`(() => { const el = document.querySelector(${q(selector)}); el.scrollIntoView({ block: 'center' });
      const r = el.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
    for (const [type, buttons] of [['mouseMoved', 0], ['mousePressed', 1], ['mouseReleased', 0]])
      await call('Input.dispatchMouseEvent', { type, ...at, button: 'left', buttons, clickCount: 1 });
  };
  return { wait, press,
    async type(selector, text) {
      await press(selector);
      await evaluate(`document.querySelector(${q(selector)}).select()`);
      await call('Input.insertText', { text });
    },
    drawn: (selector, what = selector) => wait(`!!document.querySelector(${q(selector)})`, what),
    value: selector => evaluate(`document.querySelector(${q(selector)})?.value ?? null`),
    text: selector => evaluate(`document.querySelector(${q(selector)})?.textContent ?? null`),
    rows: scope => evaluate(`[...document.querySelectorAll(${q(`${scope} [data-harness-row]`)})].map(tr => [...tr.cells].map(cell => cell.textContent))`),
    found: (scope, attribute) => evaluate(`Object.fromEntries([...document.querySelectorAll(${q(`${scope} form[${attribute}]`)})]
      .map(form => [form.getAttribute(${q(attribute)}), form.querySelector('[data-harness-found] code')?.textContent ?? null]))`),
    notices: scope => evaluate(`[...document.querySelectorAll(${q(`${scope} .inline-notice`)})].map(el => [el.textContent, el.dataset.reason ?? null])`),
    inspect: path => evaluate(`StudyApp.selectNode(StudyApp.graph().nodes.find(node => node.path === ${q(path)}).id)`),
  };
}
