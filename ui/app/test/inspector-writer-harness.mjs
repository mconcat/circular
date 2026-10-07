import fs from 'node:fs';
import vm from 'node:vm';
import { originalUI } from './original-ui.mjs';
import { unit, units } from './source-unit.mjs';

const app = fs.readFileSync(new URL('../app.js', import.meta.url), 'utf8');
const product = fs.readFileSync(new URL('../product.js', import.meta.url), 'utf8');
const part = (text, from, to) => text.slice(text.indexOf(from), text.indexOf(to, text.indexOf(from)));
const decode = text => text.replaceAll('&quot;', '"').replaceAll('&#39;', "'")
  .replaceAll('&lt;', '<').replaceAll('&gt;', '>').replaceAll('&amp;', '&');

const VOID = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr']);
function tree(html, root) {
  const all = [root], open = [root];
  root.children = [];
  for (const [, close, tag, rest] of html.matchAll(/<(\/?)([a-zA-Z][\w-]*)((?:[^>"']|"[^"]*"|'[^']*')*)>/g)) {
    const name = tag.toLowerCase();
    if (close) { const at = open.findLastIndex(el => el.tag === name); if (at > 0) open.length = at; continue; }
    const el = {tag:name, parent:open.at(-1), children:[],
      attrs:new Map([...rest.matchAll(/([^\s=/>"']+)(?:="([^"]*)")?/g)].map(([, key, value]) => [key, decode(value ?? '')]))};
    el.parent.children.push(el);
    all.push(el);
    if (!VOID.has(name) && !rest.trim().endsWith('/')) open.push(el);
  }
  return all;
}
const hasClass = (el, name) => (el.attrs.get('class') ?? '').split(/\s+/).includes(name);
function compound(text) {
  const tests = [], step = /([a-zA-Z][\w-]*|\*)|#([\w-]+)|\.([\w-]+)|\[([\w-]+)(?:="([^"]*)"|='([^']*)'|=([\w-]+))?\]|:not\(([^()]*)\)|:(first-child|last-child)/y;
  for (let at = 0; at < text.length;) {
    step.lastIndex = at;
    const m = step.exec(text);
    if (!m) throw new Error(`inspector-writer harness cannot read selector part "${text.slice(at)}"`);
    at = step.lastIndex;
    const [, tag, id, cls, attr, v1, v2, v3, not, child] = m, value = v1 ?? v2 ?? v3;
    if (tag) tests.push(el => tag === '*' || el.tag === tag.toLowerCase());
    else if (id) tests.push(el => el.attrs.get('id') === id);
    else if (cls) tests.push(el => hasClass(el, cls));
    else if (attr) tests.push(el => el.attrs.has(attr) && (value === undefined || el.attrs.get(attr) === value));
    else if (not) { const inner = compound(not); tests.push(el => !inner(el)); }
    else tests.push(el => el.parent != null && (child === 'first-child' ? el.parent.children[0] : el.parent.children.at(-1)) === el);
  }
  return el => tests.every(test => test(el));
}
function steps(group) {
  const out = [{comb:null, text:''}];
  let depth = 0, quote = null, space = false;
  for (const ch of group.trim()) {
    if (quote) { out.at(-1).text += ch; if (ch === quote) quote = null; continue; }
    if (depth === 0 && /[\s>+~]/.test(ch)) {
      if (/\s/.test(ch)) space = out.at(-1).text !== '';
      else { out.push({comb:ch, text:''}); space = false; }
      continue;
    }
    if (space) { out.push({comb:' ', text:''}); space = false; }
    if (ch === '"' || ch === "'") quote = ch;
    if (ch === '[' || ch === '(') depth++;
    if (ch === ']' || ch === ')') depth--;
    out.at(-1).text += ch;
  }
  return out.map(({comb, text}) => ({comb, test:compound(text)}));
}
function chain(el, parts, i, root) {
  if (!parts[i].test(el)) return false;
  if (i === 0) return true;
  const outside = () => parts.slice(0, i).every(part => !part.test(root)), comb = parts[i].comb;
  if (el.parent == null) return outside();
  if (comb === '>') return chain(el.parent, parts, i - 1, root);
  if (comb === ' ') {
    for (let up = el.parent; up; up = up.parent) if (chain(up, parts, i - 1, root)) return true;
    return outside();
  }
  const siblings = el.parent.children, at = siblings.indexOf(el);
  if (comb === '+') return at > 0 && chain(siblings[at - 1], parts, i - 1, root);
  return siblings.slice(0, at).some(sibling => chain(sibling, parts, i - 1, root));
}
const matches = (el, selector, root) => selector.split(',').some(group => {
  const parts = steps(group);
  return chain(el, parts, parts.length - 1, root);
});
const scoped = (query, prefix) => String(query).split(',').map(group => group.trim())
  .map(group => prefix == null ? group : group.startsWith(':scope') ? prefix + group.slice(6) : `${prefix} ${group}`).join(', ');

export function inspectorWriter(win = window) {
  const ui = originalUI(), A = win.StudyApp, source = win.StudySource;
  let contentHTML = '', panelHTML = '', generation = 0, quiet = false;
  const writes = [];
  const neutralProps = new Set(['classList', 'className', 'style', 'scrollTop', 'scrollLeft']);
  const made = new WeakMap();
  const summary = node => made.has(node) ? `<${made.get(node).tag}>` : String(node);
  function watched(target, selector, {modeled = [], connected = () => true, lookup, el} = {}) {
    const where = () => typeof selector === 'function' ? selector() : selector;
    const record = (write, value) => { if (connected()) writes.push({name:'lookup', selector:where(), write, value}); };
    const kebab = key => `data-${key.replace(/[A-Z]/g, c => '-' + c.toLowerCase())}`;
    const dataset = new Proxy({}, {
      get:(t, key) => typeof key === 'string' ? t[key] ?? el?.attrs.get(kebab(key)) : undefined,
      set(t, key, value) { t[key] = value; record(kebab(key), value); return true; },
    });
    const insert = verb => (...nodes) => {
      record(verb, nodes.map(summary));
      for (const node of nodes) made.get(node)?.attach(`${where()} ${verb}`);
    };
    const defaults = {
      classList:{add() {}, remove() {}, toggle() {}, contains:() => false}, style:{setProperty() {}, removeProperty() {}},
      dataset, get isConnected() { return connected(); },
      setAttribute:(key, value) => record(key, value), removeAttribute:key => record(key, null),
      toggleAttribute:(key, force) => record(key, force),
      getAttribute:key => el?.attrs.get(key) ?? null, hasAttribute:key => Boolean(el?.attrs.has(key)),
      querySelector:query => lookup(query)[0] ?? null, querySelectorAll:query => lookup(query),
      insertAdjacentHTML:(where, html) => record('insertAdjacentHTML', html),
      insertAdjacentText:(where, text) => record('insertAdjacentText', text),
      addEventListener() {}, removeEventListener() {}, focus() {}, blur() {}, scrollIntoView() {},
      ...Object.fromEntries(['before', 'after', 'prepend', 'append', 'appendChild', 'insertBefore', 'replaceWith',
        'replaceChildren', 'remove'].map(verb => [verb, insert(verb)])),
    };
    for (const [key, value] of Object.entries(Object.getOwnPropertyDescriptors(defaults)))
      if (!(key in target)) Object.defineProperty(target, key, value);
    return new Proxy(target, {set(t, key, value, receiver) {
      if (!modeled.includes(key) && !neutralProps.has(key)) record(key, value);
      return Reflect.set(t, key, value, receiver);
    }});
  }
  function lookUp(query, prefix = null) {
    const selector = scoped(query, prefix), at = generation;
    const root = {tag:'aside', attrs:new Map([['id', 'inspector'], ['class', 'inspector']]), parent:null};
    const drawn = tree(panelHTML.replace(/(<div class="inspector-content">)[\s\S]*(<\/div><footer class="inspector-footer">)/,
      (_, open, close) => open + contentHTML + close), root);
    return drawn.filter(el => matches(el, selector, root)).map(el =>
      el === root ? box
        : el.parent === root && el.tag === 'div' && hasClass(el, 'inspector-content') ? content
          : el.parent.parent === root && hasClass(el.parent, 'inspector-footer') && el.parent.children.find(c => c.tag === 'span') === el ? footerSpan
            : watched({}, selector, {el, connected:() => generation === at, lookup:inner => lookUp(inner, selector)}));
  }
  function make(tag) {
    let where = null, at = null;
    const connected = () => where != null && generation === at;
    const own = inner => {
      const root = {tag, attrs:new Map(), parent:null};
      return tree(String(element.innerHTML ?? ''), root).slice(1).filter(el => matches(el, inner, root))
        .map(el => watched({}, `${where} <${tag}> ${inner}`, {el, connected, lookup:() => []}));
    };
    const element = watched({}, () => `${where ?? '(detached)'} <${tag}>`, {connected, lookup:own});
    made.set(element, {tag, attach(place) { where = place; at = generation; }});
    return element;
  }
  const box = watched({classList:{remove() {}}, insertAdjacentHTML() {}}, '#inspector',
    {modeled:['innerHTML'], lookup:query => lookUp(query, '#inspector')});
  const content = watched({insertAdjacentHTML() {}}, '#inspector .inspector-content',
    {modeled:['innerHTML'], lookup:query => lookUp(query, '#inspector .inspector-content')});
  let journalHTML = '';
  const journal = {title:''};
  Object.defineProperty(journal, 'innerHTML', {get:() => journalHTML, set(html) { journalHTML = html; writes.push({name:'journal', html}); }});
  const neutral = {classList:{add() {}, remove() {}, toggle() {}}, insertAdjacentHTML() {}};
  const footerSpan = watched({set outerHTML(html) {
    quiet = true;
    box.innerHTML = box.innerHTML.replace(/(<footer class="inspector-footer">[\s\S]*?)<span[^>]*>[\s\S]*?<\/span>/, (_, head) => head + html);
    quiet = false;
    writes.push({name:'footer', html});
  }}, '#inspector .inspector-footer > span', {modeled:['outerHTML'], lookup:query => lookUp(query, '#inspector .inspector-footer > span')});
  Object.defineProperty(content, 'innerHTML', {get:() => contentHTML, set(html) {
    contentHTML = html; generation++; writes.push({name:'body', html});
  }});
  Object.defineProperty(box, 'innerHTML', {get:() => panelHTML, set(html) {
    panelHTML = html;
    contentHTML = html.match(/<div class="inspector-content">([\s\S]*)<\/div><footer class="inspector-footer">/)?.[1] ?? '';
    if (!quiet) { generation++; writes.push({name:'inspector', html}); }
  }});
  const find = selector => selector === '#inspector' ? [box]
    : selector === '#inspector .inspector-content' ? [content]
      : selector === '#inspector .inspector-footer > span' ? (box.innerHTML.includes('inspector-footer') ? [footerSpan] : [])
        : selector === '.journal-follow' ? [journal] : lookUp(selector);
  globalThis.document = {
    querySelector:selector => find(selector)[0] ?? null, querySelectorAll:selector => find(selector),
    getElementById:id => find(`#${id}`)[0] ?? null, createElement:make,
  };
  const within = scope => typeof scope?.querySelector === 'function' && scope !== globalThis.document;
  const $ = (selector, scope) => within(scope) ? scope.querySelector(selector) ?? neutral
    : find(selector)[0] ?? (selector === '#inspector .inspector-footer > span' ? null : neutral);
  const $$ = (selector, scope) => within(scope) ? [...scope.querySelectorAll(selector)] : find(selector);
  const state = A.state;
  Object.assign(state, {tab:'inspect', selectedSet:new Set(), selectedRecord:null});
  ui.document = globalThis.document;
  Object.assign(ui, {source, StudySource:source, P:win.Product, S:state, state, A,
    $: $, $$, C:ui.ProductCatalog, historical:() => A.historical(),
    graph:A.graph, portY:() => 0, selectedNode:() => A.graph().nodes.find(n => n.id === state.selected),
    icon:() => '', ic:() => '', esc:ui.ProductCatalog.escape, e:ui.ProductCatalog.escape,
    studyTimeFormat:String, panelWidth:() => 310, notice:() => '',
    configContent:() => '', sdkContent:() => '',
    syncConfigActions() {}, drawSelection() {}, refreshWireSelection() {}, renderWireActions() {}, renderJournal() {}, renderComponentEditor() {},
  });
  ui.Product = win.Product;
  ui.Product.afterSelection = () => {};
  vm.runInContext(fs.readFileSync(new URL('../product-catalog.js', import.meta.url), 'utf8'), ui);
  ui.C = ui.ProductCatalog;
  vm.runInContext(part(product, '  function permissions(', '  P.afterSelection =')
    + unit(app, '  function portHTML(') + '\n'
    + part(app, '  function drawInspector()', '  function configContent(')
    + part(app, '  function syncHistoryControls()', '  let historyFrame =')
    + part(app, '  function selectNode(', '  function selectEdge(')
    + '\nfunction renderInspector() { drawInspector(); }', ui);
  A.renderInspector = ui.renderInspector;
  A.renderInspectorFooter = ui.renderInspectorFooter;
  A.renderJournalHeader = ui.renderJournalHeader;
  const slot = (html, pattern) => {
    const match = html.match(pattern);
    if (!match) return null;
    return {text:decode(match[2]), reason:match[1].match(/data-reason="([^"]*)"/)?.[1],
      title:match[1].match(/title="([^"]*)"/)?.[1], disabled:match[1].match(/aria-disabled="([^"]*)"/)?.[1]};
  };
  return {
    writes, box, content, journal,
    view: node => ui.LiveViewers.render(node),
    port(node, name, side = 'out') { return ui.portHTML(node, node[side].find(p => p[0] === name), side); },
    open(id) { ui.selectNode(id); },
    draw() { ui.renderInspector(); },
    footer: (html = box.innerHTML) => slot(html, /<footer class="inspector-footer">[\s\S]*?<span([^>]*)>([\s\S]*?)<\/span>/),
    payload: (html = content.innerHTML) => slot(html, /<pre class="code-block" data-selected-payload([^>]*)>([\s\S]*?)<\/pre>/),
  };
}

export function journalWriter(header, source, historical = () => false) {
  const ui = originalUI();
  Object.assign(ui, {$:() => header, source, historical, icon:() => '', esc:ui.ProductCatalog.escape});
  vm.runInContext(part(app, '  function renderJournalHeader()', '  let historyFrame ='), ui);
  return ui.renderJournalHeader;
}

export function canvasProjection(source) {
  const ui = vm.createContext({window:{StudySource:source}, document:{}});
  return vm.runInContext(units(app, '  const source =', '  const state =', '  let archive, timeMachine;', '  const graph =',
    '  const liveScopes =', '  const displayTime =', '  const visibleRecords =', '  const allNode =')
    + '\n({state, graph, liveScopes, allNode, visibleRecords})', ui);
}

export function projectsWriter() {
  const writes = [];
  let html = '', generation = 0;
  const neutralProps = new Set(['classList', 'className', 'style', 'scrollTop', 'scrollLeft']);
  const drawn = () => {
    const root = {tag:'section', attrs:new Map([['id', 'projects-view'], ['class', 'product-page']]), parent:null};
    return tree(html, root);
  };
  const inside = (el, of) => { for (let up = el.parent; up; up = up.parent) if (up === of) return true; return false; };
  function element(el, selector, all, at) {
    const connected = () => generation === at;
    const record = (write, value) => { if (connected()) writes.push({name:'lookup', selector, write, value}); };
    const find = query => all.filter(d => inside(d, el) && matches(d, query, all[0])).map(d => element(d, `${selector} ${query}`, all, at));
    const kebab = key => `data-${key.replace(/[A-Z]/g, c => '-' + c.toLowerCase())}`;
    const target = {
      dataset:new Proxy({}, {get:(_, key) => typeof key === 'string' ? el.attrs.get(kebab(key)) : undefined,
        set(_, key, value) { record(kebab(key), value); return true; }}),
      classList:{add() {}, remove() {}, toggle() {}, contains:() => false}, style:{setProperty() {}, removeProperty() {}},
      get isConnected() { return connected(); },
      getAttribute:key => el.attrs.get(key) ?? null, hasAttribute:key => el.attrs.has(key),
      setAttribute:(key, value) => record(key, value), removeAttribute:key => record(key, null),
      toggleAttribute:(key, force) => record(key, force),
      querySelector:query => find(query)[0] ?? null, querySelectorAll:query => find(query),
      elements:{namedItem:name => find(`[name="${name}"]`)[0] ?? null},
      closest:() => null, addEventListener() {}, removeEventListener() {}, focus() {}, blur() {}, scrollIntoView() {},
      insertAdjacentHTML:(where, value) => record('insertAdjacentHTML', value),
      insertAdjacentText:(where, value) => record('insertAdjacentText', value),
      ...Object.fromEntries(['before', 'after', 'prepend', 'append', 'appendChild', 'insertBefore', 'replaceWith',
        'replaceChildren', 'remove'].map(verb => [verb, (...nodes) => record(verb, nodes.map(String))])),
    };
    return new Proxy(target, {set(t, key, value) {
      if (!neutralProps.has(key)) record(key, value);
      return true;
    }});
  }
  const pageLookup = query => { const all = drawn(); return element(all[0], '#projects-view', all, generation)[query]; };
  const page = new Proxy({}, {
    get(_, key) {
      if (key === 'innerHTML') return html;
      if (key === 'id') return 'projects-view';
      return pageLookup(key);
    },
    set(_, key, value) {
      if (key === 'innerHTML') { html = value; generation++; writes.push({name:'projects', html:value}); }
      else if (!neutralProps.has(key)) writes.push({name:'lookup', selector:'#projects-view', write:key, value});
      return true;
    },
  });
  const lookUp = selector => {
    const all = drawn();
    return all.slice(1).filter(el => matches(el, selector, all[0])).map(el => element(el, selector, all, generation));
  };
  const inPage = selector => /#projects-view\b/.test(selector);
  globalThis.document = {
    querySelector:selector => selector === '#projects-view' ? page : inPage(selector) ? lookUp(selector)[0] ?? null : ({}),
    querySelectorAll:selector => inPage(selector) ? lookUp(selector) : [],
    getElementById:id => id === 'projects-view' ? page : null,
  };
  return {writes, page, get html() { return html; }};
}
