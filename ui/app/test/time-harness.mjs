import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import vm from 'node:vm';
import { initialize, mount, observation } from '../renderer/adapter.mjs';
import { UNMEASURED } from '../renderer/card-size.mjs';
import { healthPage } from './fixtures.mjs';
import { journalWriter } from './inspector-writer-harness.mjs';
import { fixtureSource } from '../fixture-source.mjs';

export const presentation = await fs.readFile(new URL('../time-machine.js', import.meta.url), 'utf8');
const field = await fs.readFile(new URL('../wire-field.js', import.meta.url), 'utf8');

function stubDocument() {
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) {
      let text = '';
      const el = {id, style:{}, dataset:{}, writes:[], disabled:false, title:'', listeners:{}, childNodes:[{nodeType:3, nodeValue:''}],
        get textContent() { return text; }, set textContent(v) { text = String(v); this.writes.push(text); },
        classList:{set:new Set(), toggle(name, on) { on ? this.set.add(name) : this.set.delete(name); }, contains(name) { return this.set.has(name); }},
        setAttribute(name, value) { this[name] = String(value); }, getAttribute(name) { return this[name] ?? null; }, removeAttribute(name) { delete this[name]; },
        addEventListener(type, fn) { (this.listeners[type] ??= []).push(fn); },
        click() { for (const fn of this.listeners.click ?? []) if (!this.disabled) fn({}); },
        getBoundingClientRect:() => ({width:0}), querySelector:selector => element(id.startsWith('<') ? `${id} ${selector}` : selector), focus() {},
        children:[], append(child) { this.children.push(child); child.parent = this; },
        remove() { if (this.parent) this.parent.children = this.parent.children.filter(c => c !== this); this.removed = (this.removed ?? 0) + 1; },
        bars:[], getContext() { const el = this; return {setTransform() {}, clearRect() { el.bars = []; }, beginPath() {}, moveTo() {},
          lineTo() {}, stroke() {}, fillRect(x, y, w, h) { el.bars.push({x, y, w, h}); }}; }};
      elements.set(id, el);
    }
    return elements.get(id);
  };
  let created = 0;
  const document = {querySelector:s => element(s), getElementById:id => id.startsWith('node-') ? null : element('#' + id),
    createElement:tag => { created += 1; return element(`<${tag}>${created}`); }, get created() { return created; },
    querySelectorAll:() => [], addEventListener() {}, hidden:false};
  return {document, element};
}

function frameQueue() {
  const queue = [];
  return Object.assign(queue, {run() { for (const fn of queue.splice(0)) fn(); }});
}

function context(window, document, clock, frames = frameQueue()) {
  return vm.createContext({window, document, StudySource:window.StudySource, STUDY_HISTORY:window.STUDY_HISTORY,
    structuredClone, performance:{now:() => clock.now}, setInterval:globalThis.setInterval, clearInterval:globalThis.clearInterval,
    requestAnimationFrame:fn => frames.push(fn), cancelAnimationFrame(id) { frames[id - 1] = () => {}; },
    matchMedia:() => ({matches:true, addEventListener() {}}), devicePixelRatio:1,
    ResizeObserver:class { constructor(fn) { window.resized = fn; } observe() {} }});
}

export function fixtureMachine(t, {observing = () => true, history = {start:0, duration:120, stages:[]}} = {}) {
  const {document, element} = stubDocument();
  const clock = {now:1000}, frames = frameQueue();
  t.mock.method(globalThis, 'setInterval', () => 0);
  const window = {STUDY:{}, STUDY_HISTORY:history, performance:{now:() => clock.now}};
  const port = fixtureSource(window);
  const ctx = context(window, document, clock, frames);
  vm.runInContext(field, ctx);
  vm.runInContext(presentation, ctx);
  const archive = new window.PresentationArchive({root:{nodes:[], edges:[]}});
  const views = [];
  const machine = new window.TimeMachine(archive, {transport:port.transport, head:() => port.head, scopes:() => ({}),
    visible:() => true, observing, pausedScopes:() => new Set(), actors:() => new Set(), onView:(...args) => views.push(args)});
  return {machine, el:element, clock, views, frames, document, window};
}

export async function productMachine(t, sdk, check) {
  const previous = {window:globalThis.window, document:globalThis.document, location:globalThis.location};
  const {document, element} = stubDocument();
  const clock = {now:1000}, codes = [], views = [];
  const intervals = new Map(); let timer = 0;
  t.mock.method(globalThis, 'setInterval', (fn, ms) => { intervals.set(++timer, {fn, ms}); return timer; });
  t.mock.method(globalThis, 'clearInterval', id => intervals.delete(id));
  const window = sdk ? {circularConnection:async () => ({connection:'connected'})} : {};
  Object.assign(globalThis, {window, document, location:{search:''}});
  try {
    await initialize({connect:sdk && (async () => sdk)});
    const frames = frameQueue();
    window.cancelAnimationFrame = id => { frames[id - 1] = () => {}; };
    const ctx = context(window, document, clock, frames);
    vm.runInContext(field, ctx);
    vm.runInContext(presentation, ctx);
    const source = window.StudySource;
    const archive = new window.PresentationArchive({root:{nodes:[], edges:[]}});
    const machine = new window.TimeMachine(archive, {transport:source.transport, bar:source.timeBar, head:() => source.head,
      scopes:() => ({}), visible:() => true, observing:() => source.observing(),
      pausedScopes:() => new Set(), actors:() => new Set(), onView:(...args) => views.push(args)});
    window.StudyApp = {archive, timeMachine:machine, state:{scope:'root'}, graph:() => window.STUDY.root,
      liveScopes:() => Object.fromEntries(Object.entries(window.STUDY).filter(([, v]) => v.nodes)),
      renderJournalHeader:journalWriter(element('.journal-follow'), source, () => machine.mode !== 'live'),
      renderGraph() {}, renderJournal() {}, toast:code => codes.push(code),
      renderActors() { return []; }};
    window.Product = {refreshEditTools() {}};
    await mount({ metrics: UNMEASURED });
    if (sdk) assert.equal(await observation, true);
    await check({source, archive, machine, el:element, clock, codes, views, intervals, frames});
    for (let i = 0; i < 20; i += 1) await new Promise(resolve => setImmediate(resolve));
  } finally { Object.assign(globalThis, previous); }
}

import { heldSession, ack, creditAck, arrivalFrame, uint } from './records-peer.mjs';
export const recordedActor = local => ({scope:[], local});
export const recordedArrival = (local, index, at) => ({kind:'actor_arrival', actor:recordedActor(local), index,
  observed_at_ms:at, port:'input', body:'recorded body'});
export function recordedArrivalSDK(first = [recordedArrival('one', 7n, 1_200n)],
  {terminal = 2n, cut, at, marks = [], incidentsAt = [], regressions = 0n} = {}) {
  const sent = [], frames = [], waiting = [];
  const asked = handle => sent.filter(request => request.handle === handle).at(-1).value.args;
  const answer = value => ({kind:{verb:'QueryResult'}, payload:[1n, value]});
  const page = (items, extra = {}) => ({anchor:1n, items, terminal, ...extra});
  const streams = {'subscription-actor.events':[ack, creditAck], 'subscription-records':[ack, creditAck]};
  const snapshot = {status:'accepted', value:{anchor:{scope:[]}, commands:[...new Set(first.map(r => r.actor.local))].map(local => ({
    kind:'UpsertActor', actor:{arm:'relative', value:recordedActor(local)},
    declaration:{actorType:'synthetic', config:{}, flags:{bypass:false, pause:false, mute:false}}})), terminal:'Complete'}};
  let reads = 0;
  const sdk = heldSession({authoringSnapshot:async () => snapshot,
    hold:async name => name,
    send:async (handle, partition, verb, value) => { sent.push({handle, partition, verb, value}); },
    declare:async command => { sent.push({verb:'declare', value:command}); return {status:'rejected'}; },
    exchange:async (partition, verb, payload) => {
      if (partition === 'Query') return answer({anchor:payload.name === 'actor.create-inputs' ? [] : null, items:[], terminal:2n});
      sent.push({verb:'exchange', value:{partition, verb, payload}}); return {status:'rejected'};
    },
    next:async handle => {
      if (handle === 'actor.events') { reads += 1; return answer(page(first, cut && reads === 1 ? {cut} : {})); }
      if (handle === 'daemon.health') return answer(healthPage());
      if (handle === 'runtime.approvals') return answer(page([], {anchor:{producer:1n, persistence:[3n]}}));
      if (handle === 'timeline.bins') {
        const {from_ms, to_ms, bins} = asked(handle), bin = (to_ms.value - from_ms.value) / bins.value;
        const within = i => at => at >= from_ms.value + BigInt(i) * bin && at < from_ms.value + BigInt(i + 1) * bin;
        return answer({anchor:{from_ms, to_ms, bin_ms:uint(bin), clock_regressions:uint(regressions),
          marks:marks.filter(([, at]) => at >= from_ms.value && at < to_ms.value).map(([kind, at_ms]) => ({kind, at_ms:uint(at_ms)})),
          bins:Array.from({length:Number(bins.value)}, (_, i) => ({incidents:uint(incidentsAt.filter(within(i)).length),
            count:uint(first.map(row => row.observed_at_ms).filter(within(i)).length)}))},
        items:[], terminal:2n});
      }
      if (handle === 'timeline.at') return at ? answer(at(asked(handle).at_ms.value)) : {kind:{verb:'QueryResult'}, payload:[2n, {code:2n}]};
      if (handle === 'dead.letters' || handle === 'instance.transitions') return answer(page([], {anchor:null}));
      const step = streams[handle]?.shift();
      if (step !== undefined) return step;
      if (handle !== 'subscription-actor.events') return new Promise(() => {});
      const arrival = frames.shift() ?? await new Promise(resolve => waiting.push(resolve));
      streams[handle].push(null, creditAck);
      return arrivalFrame(arrival);
    },
    release:async () => {},
  });
  const deliver = arrival => { const resolve = waiting.shift(); if (resolve) resolve(arrival); else frames.push(arrival); };
  return {sdk, sent, deliver};
}
