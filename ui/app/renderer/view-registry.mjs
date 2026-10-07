
import { observeActors } from './scene.mjs';
import { codeText, reasonText } from './reasons.mjs';
import { recordValue } from './record-text.mjs';
import { DETAIL, NAMES, wordsHTML } from './tier.mjs';
import { RATE_SECONDS } from './activity.mjs';

export const escape = value => String(value ?? '').replace(/[&<>"']/g, c =>
  ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
export const spark = '<canvas class="viewer-spark" width="480" height="130" aria-label="Recent arrival density"></canvas>';
export const kicker = (heading, live) => heading == null && live === undefined ? ''
  : `<div class="viewer-kicker">${escape(heading)}<span>${escape(live)}</span></div>`;
export const atDetail = (tier, html) => tier === DETAIL ? html : '';
export const formAt = tier => tier === DETAIL ? '' : ' reduced';
export const styleAt = (tier, reduced, names = '') => {
  const declarations = tier === DETAIL ? '' : tier === NAMES ? [reduced, names].filter(Boolean).join(';') : reduced;
  return declarations ? ` style="${declarations}"` : '';
};
export const ROW = 'margin:0;padding:0;font-size:inherit;line-height:var(--resize-handle);letter-spacing:0';
export const places = (html, grow = false) => `<div class="viewer-places"${grow ? ' style="flex:1 1 auto"' : ''}>${html}</div>`;
export const ONE_LINE = 'min-width:0;max-width:100%;white-space:nowrap;overflow:hidden;text-overflow:ellipsis';
export const WORDS = 'flex:1 1 auto;width:auto;min-width:0;white-space:normal;overflow:hidden;text-overflow:ellipsis';
export const words = text => wordsHTML(text, escape);
export const writeWords = (element, text) => {
  if (element.textContent !== String(text ?? '')) element.innerHTML = words(text);
};
export const PROSE = 'white-space:pre-wrap;overflow-wrap:anywhere';

const factAttributes = (name, fact) => ` data-cell="${escape(name)}"${fact.title ? ` title="${escape(fact.title)}"` : ''}`
  + (fact.code ? ` data-reason="${escape(codeText(fact.code))}"` : '') + (fact.declared ? ` data-words="${escape(fact.declared)}"` : '');
const liveAttribute = fact => fact.live ? ` data-live="${escape(fact.live)}"` : '';
export function factHTML(name, fact, tier) {
  if ('reason' in fact) {
    const { reason, ...rest } = fact;
    return factHTML(name, tier === NAMES ? { ...rest, reading: '—', title: rest.title ?? reasonText(reason), code: reason }
      : { ...rest, words: reasonText(reason), title: rest.title ?? reasonText(reason), code: reason }, tier);
  }
  const at = factAttributes(name, fact);
  if ('rows' in fact) return `<div class="glance-rows"${at}>${fact.rows.map(([label, value, code, title]) =>
    `<p${code ? ` data-reason="${escape(codeText(code))}"` : ''}${code || title ? ` title="${escape(title ?? reasonText(code))}"` : ''}><span>${escape(label)}</span>${value ? `<b>${escape(value)}</b>` : ''}</p>`).join('')}</div>`;
  if ('spark' in fact) return `<canvas class="viewer-spark glance-spark"${at} width="480" height="130" aria-label="${escape(fact.spark)}"></canvas>`;
  if ('count' in fact) return `<p class="glance-words"${at}><b${liveAttribute(fact)}>${escape(fact.count)}</b>${fact.unit !== undefined ? escape(fact.unit)
    : ` ${tier === NAMES ? escape(fact.noun) : words(fact.noun)}`}</p>`;
  if ('words' in fact) return `<p class="glance-words"${at}>${tier === NAMES ? escape(fact.words) : words(fact.words)}</p>`;
  return `<p class="glance-reading${fact.lead ? ' lead' : ''}"${at}>${fact.caption ? `<small>${escape(fact.caption)}</small> ` : ''}<span${liveAttribute(fact)}>${escape(fact.reading)}</span></p>`;
}
const glanceCells = (glance, node, tier, t, source) => {
  const cells = glance.cells(node, t, source);
  return glance[tier].filter(name => cells[name]).map(name => [name, factHTML(name, cells[name], tier)]);
};
export const glanceHTML = (glance, node, tier, t = Infinity, source) =>
  `<div class="glance">${glanceCells(glance, node, tier, t, source).map(([, html]) => html).join('')}</div>`;
const drawnFacts = new WeakMap();
export function writeGlance(host, glance, node, t, source, tier) {
  const box = host?.querySelector?.('.node-viewer > .glance');
  if (!box) return;
  const cells = glanceCells(glance, node, tier, t, source), standing = [...box.children];
  if (cells.map(([name]) => name).join(' ') !== standing.map(element => element.dataset.cell).join(' ')) {
    box.innerHTML = cells.map(([, html]) => html).join('');
    for (const [i, element] of [...box.children].entries()) drawnFacts.set(element, cells[i][1]);
    return;
  }
  for (const [i, [, html]] of cells.entries()) {
    const element = standing[i];
    if (drawnFacts.get(element) === html) continue;
    const frame = box.ownerDocument.createElement('template');
    frame.innerHTML = html;
    const drawn = frame.content.firstElementChild;
    if (drawn.isEqualNode(element)) { drawnFacts.set(element, html); continue; }
    element.replaceWith(drawn);
    drawnFacts.set(drawn, html);
  }
}
export const readingAt = (tier, row = tier === NAMES) => tier === DETAIL ? { block: '', value: '', label: '' } : {
  block: ` style="flex-direction:row;${tier === NAMES ? '' : 'flex-wrap:wrap;'}align-items:baseline;gap:${tier === NAMES ? '6px' : '0 6px'}${row ? ';padding:0' : ''}"`,
  value: ` style="${row ? `flex:0 1 auto;${ONE_LINE};${ROW};color:var(--ink)` : `display:block;flex:0 1 auto;max-width:100%;${ONE_LINE}`}"`,
  label: ` style="${tier === NAMES ? `flex:1 1 0;${ONE_LINE};${ROW}` : WORDS}"`,
};
export const table = (columns, rows, cls = '') =>
  `<div class="viewer-table ${cls}"><div class="viewer-table-head">${columns.map(c => `<span>${c}</span>`).join('')}</div>${rows.map(r => `<div class="viewer-table-row">${r.map(c => cell(c)).join('')}</div>`).join('')}</div>`;
export const reasonCell = code => Object.freeze({ reason: code });
export function readViewPath(body, path) {
  let value = body;
  for (const segment of path) {
    const container = typeof segment === 'bigint'
      ? Array.isArray(value) && segment >= 0n && segment < BigInt(value.length)
      : typeof segment === 'string' && value !== null && typeof value === 'object' && !Array.isArray(value);
    if (!container || !Object.hasOwn(value, segment)) return { code: 'READ_UNAVAILABLE' };
    value = value[segment];
  }
  return value === undefined ? { code: 'READ_UNAVAILABLE' } : { value };
}
const cell = v => v && typeof v === 'object' && 'reason' in v
  ? `<span data-reason="${escape(codeText(v.reason))}" title="${escape(reasonText(v.reason))}">${escape(reasonText(v.reason))}</span>`
  : `<span>${escape(v)}</span>`;
export const resultRows = (headers, data, form = '', style = {}) => {
  const row = style.row ? ` style="${style.row}"` : '';
  return `<div class="result-table${form}"${style.table ? ` style="${style.table}"` : ''}>${headers ? `<div${row}>${headers.map(v => `<b>${escape(v)}</b>`).join('')}</div>` : ''}${data.map(r => `<div${row}>${r.map(cell).join('')}</div>`).join('')}</div>`;
};
export function writeFacts(line, facts) {
  const slots = [...(line.querySelectorAll?.('[data-fact]') ?? [])];
  const shown = facts.map(fact => fact?.[0] ? fact : null);
  for (const fact of shown.slice(slots.length)) {
    const free = shown.findIndex((own, i) => i < slots.length && !own);
    if (fact?.[1] && free >= 0) shown[free] = fact;
  }
  for (const [i, slot] of slots.entries()) {
    const [text, code] = shown[i] ?? ['', null];
    if (slot.textContent !== text) slot.textContent = text;
    if (code) slot.setAttribute('data-reason', code); else slot.removeAttribute?.('data-reason');
  }
}
export const empty = (code, action = '', tier = DETAIL, mark = '') =>
  `<div class="empty-state" data-reason="${escape(codeText(code))}"${styleAt(tier, '', 'gap:0')}>${mark}<span${styleAt(tier, '', `${ROW};${ONE_LINE}`)}>${escape(reasonText(code))}</span>${action}</div>`;
export const EMPTY_MARKS = Object.freeze({ projects: 'Layers', outputs: 'ArrowUpRight', inspector: 'MousePointer2' });
const iconMarkup = shape => shape ? `<svg class="icon" aria-hidden="true" viewBox="0 0 24 24">${shape.map(([tag, attrs]) =>
  `<${tag} ${Object.entries(attrs).map(([k, v]) => `${k}="${escape(v)}"`).join(' ')}></${tag}>`).join('')}</svg>` : '';
export const objectMembers = availability => availability?.kind === 'Known'
  && availability.flow.item.kind === 'Object' ? availability.flow.item.fields : [];
export const declaredCapacity = n => n?.config?.queue_capacity ?? null;
export const capacityText = n => String(declaredCapacity(n) ?? 'undeclared');
export const bytesText = bytes => bytes == null ? '—' : bytes >= 1024 ? (bytes / 1024).toFixed(1) + ' kB' : bytes + ' B';

export function formatReading(recorded, { fixed = false, label = '' } = {}) {
  const value = typeof recorded?.value === 'bigint' && Object.keys(recorded).length === 1 ? recorded.value : recorded;
  const full = typeof value === 'bigint' || typeof value === 'number' || typeof value === 'string'
    ? String(value) : null;
  if (full === null || !/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(full)) return null;
  let text;
  if (/^[+-]?\d+$/.test(full)) {
    const integer = BigInt(full);
    text = fixed && integer >= -9007199254740991n && integer <= 9007199254740991n ? `${full}.0` : full;
  } else {
    const numeric = Number(full);
    if (!Number.isFinite(numeric) || Math.abs(numeric) > Number.MAX_SAFE_INTEGER
      || numeric === 0 && /[1-9]/.test(full.split(/e/i)[0])) text = full;
    else {
      text = numeric.toFixed(1);
      if (numeric !== 0 && Number(text) === 0) text = numeric.toPrecision(2);
      else if (!fixed) text = text.replace(/\.0$/, '');
    }
  }
  return { full, text, title: [full, label].filter(Boolean).join(' · ') };
}

const durationUnits = [['h', 3600000], ['min', 60000], ['s', 1000], ['ms', 1]];
const recordedDurationUnit = durationUnits.at(-1)[0];
export function formatDuration(recorded) {
  const reading = formatReading(recorded);
  if (!reading) return null;
  const ms = Number(reading.full), magnitude = Math.abs(ms), title = `${reading.full} ${recordedDurationUnit}`;
  const [unit, size] = durationUnits.find(([, size]) => Math.round(magnitude / size * 100) >= 100) ?? durationUnits.at(-1);
  const printed = (Math.round(magnitude / size * 100) / 100).toFixed(2).replace(/\.?0+$/, '');
  const text = `${ms < 0 ? '-' : ''}${printed} ${unit}`;
  return { full: reading.full, text, title, detail: `${text} (${title})` };
}
export const DURATION_UNITS = Object.freeze([['ms', 1n], ['s', 1000n], ['min', 60000n]]);
export const durationUnitName = name => `${name}:unit`;
export function durationShown(text) {
  const typed = String(text ?? '').trim();
  if (!/^\d+$/.test(typed) || BigInt(typed) === 0n) return { unit: 'ms', text: typed };
  const ms = BigInt(typed), [unit, size] = DURATION_UNITS.findLast(([, size]) => ms % size === 0n);
  return { unit, text: String(ms / size) };
}
export function durationMs(text, unit = 'ms') {
  const typed = String(text ?? '').trim(), size = DURATION_UNITS.find(([name]) => name === unit)?.[1];
  if (unit === 'ms' || typed === '') return typed;
  return size !== undefined && /^\d+$/.test(typed) ? String(BigInt(typed) * size) : null;
}
export const durationUnitSelect = (name, unit, label) => `<select class="config-unit" name="${escape(name)}" data-duration-unit aria-label="${escape(label)} unit">`
  + DURATION_UNITS.map(([u]) => `<option value="${u}"${u === unit ? ' selected' : ''}>${u}</option>`).join('') + '</select>';

export function formatTime(recorded, spell = globalThis.window?.studyTimeFormat, describe = globalThis.window?.studyTimeTitle) {
  const reading = formatReading(recorded);
  if (!reading || !/^\d+$/.test(reading.full)) return null;
  const seconds = Number(reading.full) / 1000, ms = `${reading.full} ms`;
  const text = typeof spell === 'function' ? spell(seconds) : `${seconds.toFixed(3)} s`;
  return { full: reading.full, text, title: [ms, typeof describe === 'function' ? describe(seconds) : null].filter(Boolean).join(' · '),
    detail: `${text} (${ms})` };
}

export function formatRate(counted, code = null) {
  const count = formatReading(counted?.count);
  if (code || !count || !counted?.whole) {
    code ??= 'DENSITY_UNREAD';
    return { text: '—', title: reasonText(code), code };
  }
  return { ...count, title: `${count.full} ${count.full === '1' ? 'arrival' : 'arrivals'} recorded in the last ${RATE_SECONDS} s of recorded time`, code: null };
}

export function draw(canvas, values, paint = globalThis.window?.StudyPaint) {
  const ctx = canvas.getContext('2d'), w = canvas.width, h = canvas.height,
    max = Math.max(4, ...values.filter(v => v != null)) * 1.15;
  ctx.clearRect(0, 0, w, h);
  ctx.strokeStyle = paint?.grid || '#dbe3d2';
  ctx.lineWidth = 1;
  for (const y of [h - 9, h * 0.5, 10]) {
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
  }
  ctx.beginPath();
  let connected = false;
  values.forEach((v, i) => {
    if (v == null) { connected = false; return; }
    const x = (i * w) / (values.length - 1), y = h - 9 - (v / max) * (h - 18);
    if (connected) ctx.lineTo(x, y);
    else ctx.moveTo(x, y);
    connected = true;
  });
  ctx.lineJoin = 'round';
  ctx.strokeStyle = paint?.emissionHalo;
  ctx.lineWidth = 8;
  ctx.stroke();
  ctx.strokeStyle = paint?.emissionCore;
  ctx.lineWidth = 2.4;
  ctx.stroke();
}

const refusal = (code, detail) => Object.assign(new Error(code), { code, ...detail });

export function viewRegistry(descriptors) {
  const entries = new Map();
  for (const descriptor of descriptors) {
    if (entries.has(descriptor.kind)) throw refusal('VIEW_KIND_DUPLICATE', { kind: descriptor.kind });
    entries.set(descriptor.kind, Object.freeze({ ...descriptor, defaultFor: Object.freeze([...(descriptor.defaultFor ?? [])]),
      render: (node, own, tier = DETAIL, accepts = true) => tier !== DETAIL && descriptor.glance
        ? glanceHTML(descriptor.glance, node, tier) : descriptor.render(node, own, tier, accepts),
      input: (node, ctx) => ({ viewConfig: recordValue(node.viewConfig), ...descriptor.input?.(node, ctx) }),
      reads: descriptor.reads && Object.freeze([...descriptor.reads]),
      traits: Object.freeze({ ...(descriptor.traits ?? {}) }) }));
  }
  const registry = {
    has: kind => typeof kind === 'string' && entries.has(kind),
    of: kind => entries.get(kind) ?? entries.get('basic'),
    defaultFor(actorType) {
      for (const [kind, entry] of entries) if (entry.defaultFor.includes(actorType)) return { kind, code: null };
      return undefined;
    },
    kinds: () => [...entries.keys()],
    all: () => [...entries.values()],
    choices(type, declared, shown) {
      return [...entries.values()].filter(entry => entry.kind === shown || entry.defaultFor.includes(type)
        || entry.reads?.every(key => declared?.includes(key))).map(entry => entry.kind);
    },
    page(graph, page, only) {
      if (!page) return graph;
      return observeActors(graph, node => registry.of(node.viewKind?.kind).page?.(node, page), only);
    },
    async read(session, graph, page) {
      for (const entry of entries.values())
        if (entry.read) graph = await entry.read(session, graph, page, node => node.viewKind?.kind === entry.kind);
      return graph;
    },
    rows: () => Math.max(0, ...[...entries.values()].map(entry => entry.rows ?? 0)),
    viewOf: (declared, actorType) => viewOf(declared, actorType, registry),
  };
  return Object.freeze(registry);
}

export const viewOf = (declared, actorType, registry) =>
  declared ? (registry.has(declared) ? { kind: declared, code: null } : { kind: 'basic', code: 'VIEW_KIND_UNREGISTERED' })
    : registry.defaultFor(actorType) ?? { kind: 'basic', code: 'VIEW_ACTOR_TYPE_UNREGISTERED' };

export function renderView(views, n, own = {}, selected = n.viewKind, tier = DETAIL, accepts = true) {
  const { kind, code } = selected;
  const body = views.of(kind).render(n, own, tier, accepts);
  return `<div class="node-viewer" data-viewer="${escape(kind)}"${code ? ` data-reason="${escape(code)}"` : ''}>${body}${code ? `<div class="viewer-message">${escape(reasonText(code))}</div>` : ''}</div>`;
}

const WHOLE = Object.freeze({ tier: DETAIL, scale: 1 });

export function liveViewers(views, win = globalThis.window) {
  const select = node => {
    if (node.viewKind) return node.viewKind;
    const declared = node.viewer || node.view;
    return viewOf(declared, declared ? undefined : node.type, views);
  };
  const set = (el, key, value) => {
    const target = el.querySelector(`[data-live="${key}"]`);
    if (target && target.textContent !== value) target.textContent = value;
  };
  const observation = (n, t, rate) => win.StudySource.observation(n, t, rate);
  const hosts = n => win.document.querySelectorAll(`[data-view-host="${n.id}"]`);
  const projection = (el, node) => el.viewProjection?.() ?? { node, source: win.StudySource };
  const drawnAt = (el, drawn) => el.viewProjection ? WHOLE : drawn;
  function paint(el, n, t, rate, records, source) {
    const obs = source.observation(n, t, rate(n.id, t));
    const rateReading = source.arrivalsIn
      ? formatRate(source.arrivalsIn(n.id, t - RATE_SECONDS, t))
      : formatReading(obs.value, { fixed: true, label: 'events / s' });
    set(el, 'rate', rateReading?.text ?? '—');
    const rateSlot = el.querySelector('[data-live="rate"]');
    if (rateSlot) rateSlot.title = rateReading?.title ?? reasonText('READ_UNAVAILABLE');
    set(el, 'bytes', bytesText(n.preview?.bytes));
    const canvas = el.querySelector('.viewer-spark');
    if (canvas) {
      const history = Array.from({ length: 40 }, (_, i) => {
        const at = Math.max(0, t - (39 - i) * 0.2);
        return source.observation(n, at, rate(n.id, at)).value ?? null;
      });
      draw(canvas, history, win.StudyPaint);
    }
    const rows = el.querySelectorAll('.tools-table .viewer-table-row');
    if (rows.length && obs.activeTool !== undefined)
      rows.forEach((r, i) => {
        r.classList.toggle('current', i === obs.activeTool % rows.length);
        r.lastElementChild.textContent = i === obs.activeTool % rows.length ? 'active' : 'idle';
      });
    const recordRows = el.querySelectorAll('.records-table .viewer-table-row');
    if (recordRows.length) {
      const arrivals = records(n.id).filter(r => r.event === 'actor_arrival');
      recordRows.forEach((r, i) => {
        const arrival = arrivals[i];
        r.firstElementChild.textContent = arrival ? '#' + arrival.index : '—';
        r.lastElementChild.textContent = arrival ? arrival.value?.kind || 'arrival' : '';
      });
    }
  }
  const writer = (view, drawn) => drawn.tier !== DETAIL && view.glance
    ? (el, node, t, source) => writeGlance(el, view.glance, node, t, source, drawn.tier) : view.update;
  function update(el, n, t = win.StudySource.head ?? 0, { rate = () => 0, records = () => [], drawn = WHOLE } = {}) {
    const { node, source } = projection(el, n);
    const view = views.of(select(node).kind), paintsOnly = source.paintsOnly;
    const at = drawnAt(el, drawn), write = writer(view, at);
    if (!paintsOnly && at.tier !== DETAIL && view.glance) write?.(el, node, t, source, at);
    if (paintsOnly || view.tick !== 'replace') paint(el, node, t, rate, records, source);
    if (!paintsOnly && (at.tier === DETAIL || !view.glance)) write?.(el, node, t, source, at);
  }
  let last = -1;
  return {
    render(n, own = {}, tier = DETAIL, accepts = true) {
      return renderView(views, n, own, select(n), tier, accepts);
    },
    select,
    choices: (n, declared) => views.choices(n.type, declared, select(n).kind).map(kind => [kind, kind]),
    capacity: capacityText,
    declaredCapacity,
    formatReading,
    formatDuration,
    formatTime,
    durationShown,
    durationMs,
    durationUnitName,
    durationUnitSelect,
    drawSpark: draw,
    empty: (code, action = '', frame) => empty(code, action, DETAIL, iconMarkup(win.ICONS?.[EMPTY_MARKS[frame]])),
    traits: kind => views.has(kind) ? views.of(kind).traits : Object.freeze({}),
    size: n => views.of(select(n).kind).size,
    kinds: () => views.kinds(),
    update,
    tick(nodes, t, rate, force = false, records = () => [], drawn = WHOLE) {
      const due = force || !(t >= last && t - last < 0.2);
      if (due) last = t;
      for (const n of nodes) {
        for (const el of hosts(n)) {
          const { node, source } = projection(el, n);
          const view = views.of(select(node).kind), policy = source.paintsOnly ? 'painter' : view.tick ?? 'painter';
          const at = drawnAt(el, drawn), write = writer(view, at), first = at.tier !== DETAIL && Boolean(view.glance);
          const writes = !(policy === 'painter' || !write || (policy === 'paced' && !due));
          if (writes && first) write(el, node, t, source, at);
          if (due && policy !== 'replace') paint(el, node, t, rate, records, source);
          if (writes && !first) write(el, node, t, source, at);
        }
      }
    },
    draw(nodes, t, rate, records = () => [], drawn = WHOLE) {
      for (const n of nodes) for (const el of hosts(n)) update(el, n, t, { rate, records, drawn });
    },
    reset: () => { last = -1; },
    observation,
  };
}
