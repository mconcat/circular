import fs from 'node:fs';
import vm from 'node:vm';
import { initialize, mount, observation } from '../renderer/adapter.mjs';
import { UNMEASURED } from '../renderer/card-size.mjs';
import { originalUI } from './original-ui.mjs';

const source = fs.readFileSync(new URL('../app.js', import.meta.url), 'utf8');
const classes = () => { const names = new Set();
  return { add: n => names.add(n), remove: n => names.delete(n), contains: n => names.has(n), toggle: (n, on) => on ? names.add(n) : names.delete(n) }; };

export async function product(peer, action) {
  const previous = { window: globalThis.window, location: globalThis.location, document: globalThis.document };
  try {
    globalThis.window = { circularConnection: async () => ({ connection: 'connected' }) };
    globalThis.location = { search: '' };
    globalThis.document = { querySelector: () => ({}), querySelectorAll: () => [], getElementById: () => null };
    await initialize({ connect: async () => peer.session });
    const notes = [];
    window.StudyApp = { state: { scope: 'root', selected: null, zoom: 1 }, archive: {},
      graph: () => window.STUDY.root,
      liveScopes: () => Object.fromEntries(Object.entries(window.STUDY).filter(([, v]) => v.nodes)),
      renderGraph() {}, renderJournal() {}, timeMachine: { refresh() {} }, toast: (...args) => notes.push(args), historical: () => false };
    window.Product = { refreshEditTools() {}, checkpoint() {} };
    await mount({ metrics: UNMEASURED }); if (await observation !== true) throw new Error('fixture product not observed');
    await new Promise(resolve => setImmediate(resolve));
    await action(notes);
  } finally {
    await peer.session.close(); for (let i = 0; i < 10; i++) await new Promise(resolve => setImmediate(resolve));
    Object.assign(globalThis, previous);
  }
}

const productSource = fs.readFileSync(new URL('../product.js', import.meta.url), 'utf8');
function productConnect(clearConnection) {
  const P = {}, context = vm.createContext({ window: globalThis.window, source: globalThis.window.StudySource, P, A: { clearConnection } });
  vm.runInContext(productSource.slice(productSource.indexOf('  P.connect = (outlet, inlet) => {'), productSource.indexOf('  function rename() {')), context);
  return P.connect;
}

export function canvasGestures({ nodes, edges = () => [], selected = [], zoom = 1, historical = false, pointAt = () => null }) {
  const handlers = new Map(), elements = new Map(), sent = [], selections = [];
  const element = id => elements.get(id) ?? elements.set(id, { style: { setProperty() {} }, classList: classes(), dataset: {},
    querySelector: s => s === '.resize-dimensions' ? { textContent: '' } : null, querySelectorAll: () => [] }).get(id);
  const canvas = { classList: classes(), setPointerCapture() {}, focus() {}, addEventListener: (type, fn) => handlers.set(type, fn) };
  const state = { zoom, selectedSet: new Set(selected), tool: 'select', space: false, x: 0, y: 0 };
  const context = vm.createContext({
    window: globalThis.window, source: globalThis.window.StudySource, performance,
    document: { addEventListener: (type, fn) => handlers.set(type, fn), elementFromPoint: (...at) => pointAt(...at) },
    $: s => s === '#canvas' ? canvas : s.startsWith('#node-') ? element(s.slice(6)) : { classList: classes(), setAttribute() {} },
    $$: () => [], state, findNode: id => nodes().find(n => n.id === id), allNode: id => nodes().find(n => n.id === id),
    graph: () => ({ nodes: nodes(), edges: edges() }),
    historical: () => historical, selectNode() {}, selectEdge: (...args) => selections.push(args),
    hideProbe() {}, showProbe() {}, scheduleWires() {}, renderWires() {},
    travel: { stop() {}, moving: false },
    refreshWireSelection() {}, transformWorld() {}, toWorld: (x, y) => ({ x: x / zoom, y: y / zoom }), archive: null, suppressClickUntil: 0,
    openPalette() { state.paletteOpened = true; }, WireGeometry: { rounded: () => '' },
    LiveViewers: originalUI().LiveViewers,
  });
  vm.runInContext(source.slice(source.indexOf('  const size ='), source.indexOf('  const toWorld =')) +
    source.slice(source.indexOf('  const viewTraits ='), source.indexOf('  const interactive =')) +
    source.slice(source.indexOf('  function clearConnection() {'), source.indexOf('  $("#canvas").addEventListener(\n    "wheel"')), context);
  context.window = { ...globalThis.window, Product: { ...globalThis.window.Product, connect: productConnect(context.clearConnection), clearCompatibility() {}, showCompatibility() {} } };
  vm.runInContext('var Product = window.Product;', context);
  const target = (kind, id, port = {}) => {
    const self = { dataset: kind === 'header' ? { drag: id } : kind === 'port' ? { node: id, ...port } : { resize: id },
      closest: s => (kind === 'header' && s === '[data-drag]') || (kind === 'resize' && s === '[data-resize]')
        || (kind === 'port' && s === '.node-port') ? self
        : s.split(',').includes('.node') ? element(id) : null };
    return self;
  };
  const chip = (edge, id) => {
    const self = { dataset: { comb: id, edge },
      closest: s => s.split(',').some(one => ['[data-comb]', '.wire-component', '[data-edge]', 'button'].includes(one)) ? self : null };
    return self;
  };
  return { state, handlers, target, chip, selections,
    event(type, extra = {}) {
      return handlers.get(type)({ type, button: 0, pointerId: 1, pointerType: 'mouse', clientX: 0, clientY: 0, shiftKey: false,
        target: { closest: () => null }, preventDefault() {}, ...extra });
    } };
}

export function disconnectButton({ edges, state, renderGraph = () => {} }) {
  const from = source.indexOf('      case "disconnect-wire": {'), to = source.indexOf('  document.addEventListener("input", (e) => {', from);
  const block = source.slice(from, source.lastIndexOf('    }\n  });', to));
  const context = vm.createContext({ window: globalThis.window, source: globalThis.window.StudySource, state,
    graph: () => ({ edges: edges() }), renderGraph });
  vm.runInContext(`function press(target) { switch (target.id) {\n${block}\n} }`, context);
  return { press: () => context.press({ id: 'disconnect-wire' }) };
}

export function renameDialog() {
  const from = productSource.indexOf('    if (form.dataset.rename) {'), to = productSource.indexOf('    if (form.hasAttribute("data-group")) {', from);
  const dialog = { closed: 0, close() { this.closed++; } };
  const context = vm.createContext({ window: globalThis.window, source: globalThis.window.StudySource, $: () => dialog, A: {} });
  vm.runInContext(`function submit(ev) { const form = ev.target;\n${productSource.slice(from, to)}\n}`, context);
  return { dialog, submit: (id, value) => context.submit({ target: { dataset: { rename: id }, elements: { name: { value } } }, preventDefault() {} }) };
}

export function flagButton(selected) {
  const from = productSource.indexOf('      if (b.dataset.flag)\n'), to = productSource.indexOf('      if (b.dataset.reorder) {', from);
  const context = vm.createContext({ window: globalThis.window, source: globalThis.window.StudySource, S: { selected }, A: {} });
  vm.runInContext(`function press(b) {\n${productSource.slice(from, to)}\n}`, context);
  return flag => context.press({ dataset: { flag } });
}

export function pauseControl({ historical = false } = {}) {
  const from = source.indexOf('  function togglePause(force = false) {'), to = source.indexOf('  function sendPrompt(button) {', from);
  const menu = { classList: classes() }; menu.classList.add('open');
  const context = vm.createContext({ window: globalThis.window, source: globalThis.window.StudySource, historical: typeof historical === 'function' ? historical : () => historical,
    $: () => menu, state: { paused: new Set(), scope: 'root' }, timeMachine: { resume() { throw new Error('replay is not run control'); } },
    toast() {}, renderGraph() {}, paused: () => false, graph: () => ({ nodes: [] }) });
  vm.runInContext(source.slice(from, to), context);
  return { menu, press: force => context.togglePause(force) };
}
