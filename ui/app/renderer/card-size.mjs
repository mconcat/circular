import { DETAIL, NAMES, TIERS, TITLE_LINES, leastZoom, wordSegments, symbolScale } from './tier.mjs';

const BODY_WIDTH = 244;

function linesIn(segments, room) {
  let lines = 1, used = 0;
  for (const { glyphs, width, hang } of segments) {
    if (used > 0 && used + width > room) { lines++; used = 0; }
    if (used + width <= room) used += width;
    else for (const advance of glyphs) {
      if (used > 0 && used + advance > room) { lines++; used = 0; }
      used += advance;
    }
    used += hang;
  }
  return lines;
}
const segmentsOf = (text, advance) => wordSegments(text).map(segment => {
  const glyphs = [...segment.trimEnd()].map(advance);
  return { glyphs, width: glyphs.reduce((sum, width) => sum + width, 0), hang: advance(segment) - advance(segment.trimEnd()) };
});
const textLines = (text, advance, room) => text.split('\n').reduce((lines, part) => lines + linesIn(segmentsOf(part, advance), room), 0);
const widestSegment = segments => Math.max(0, ...segments.map(segment => segment.width));
function leastRoom(segments, lines) {
  let low = widestSegment(segments), high = segments.reduce((sum, segment) => sum + segment.width + segment.hang, 0);
  if (linesIn(segments, low) <= lines) return low;
  for (let step = 0; step < 40; step++) {
    const middle = (low + high) / 2;
    if (linesIn(segments, middle) <= lines) high = middle; else low = middle;
  }
  return high;
}

export function size(spec, label, metrics, foot = 0, config) {
  const name = String(label ?? ''), text = metrics.label(name), segments = segmentsOf(name, metrics.label);
  const tiers = TIERS.filter(tier => TITLE_LINES[tier]).map(tier => ({ tier, lines: TITLE_LINES[tier], scale: symbolScale(leastZoom(tier)),
    chrome: metrics.chrome(tier), head: metrics.head(tier), row: metrics.row(tier) }));
  let w = BODY_WIDTH;
  for (const { lines, scale, chrome } of tiers)
    w = Math.max(w, scale * ((lines === 1 ? text : leastRoom(segments, lines)) + chrome));
  w = Math.ceil(w);
  const declared = config?.[spec.size.words], words = typeof declared === 'string' && declared.length ? metrics.words(spec.kind) : undefined;
  let h = spec.size.height;
  for (const { tier, lines, scale, chrome, head, row } of tiers) {
    const taken = scale * text <= w - scale * chrome ? 1 : lines;
    h = Math.max(h, spec.size.height + (taken - 1) * scale * metrics.line, scale * (taken * metrics.line + head + row));
    if (tier !== 'reduced' || !words) continue;
    const room = (w - 2 * metrics.inset) / scale - (words.chrome - 2 * metrics.inset);
    h = Math.max(h, scale * (taken * metrics.line + head + words.frame + (textLines(declared, words.advance, room) - 1) * words.line) + 2 * metrics.inset);
  }
  return { w, h: Math.ceil(Math.max(h, foot)) };
}

export function nameFloor(cards, metrics) {
  const reduced = leastZoom('reduced');
  let floor = 0;
  for (const card of cards) {
    const above = cards.filter(other => other !== card && other.y + other.height <= card.y
      && other.x < card.x + card.width && card.x < other.x + other.width);
    const clearance = Math.min(Infinity, ...above.map(other => card.y - other.y - other.height)) + metrics.inset;
    const segments = segmentsOf(card.name, metrics.label), room = card.width - 2 * metrics.inset;
    const widest = widestSegment(segments);
    let least = reduced;
    for (let k = 1; ; k++) {
      const need = k * metrics.line + metrics.nameFoot, tall = need && need / clearance, across = leastRoom(segments, k);
      if (tall >= least) break;
      least = Math.min(least, Math.max(tall, across / room));
      if (across <= widest) break;
    }
    floor = Math.max(floor, least);
  }
  return floor;
}

const NO_CHIP = Object.freeze({ w: 0, h: 0 });
export const UNMEASURED = Object.freeze({ label: () => 0, line: 0, chrome: () => 0, head: () => 0, row: () => 0, chip: () => NO_CHIP,
  port: () => null, caption: () => 0, inset: 0, portFoot: 0, nameFoot: 0, words: () => undefined });

export function chipRoom(metrics) {
  const room = { w: 0, h: 0 };
  for (const tier of TIERS) {
    const { w, h } = metrics.chip(tier);
    room.w = Math.max(room.w, w);
    room.h = Math.max(room.h, h);
  }
  return room;
}

const plane = flow => flow.kind === 'Unavailable' ? null : flow.flow.kind === 'Signal' ? 'signal' : 'event';
export function portCaption([id, flow, , label]) {
  const name = label ?? id;
  return name === plane(flow) ? '' : name;
}

export function captionRoom(metrics, side, caption) {
  if (!caption) return null;
  let room = null;
  for (const tier of TIERS) {
    const at = metrics.port(tier);
    if (!at) continue;
    const w = Math.min(at.frame + at.scale * metrics.caption(caption), at.max), edge = at[side];
    const box = { x: side === 'in' ? edge.edge - w : edge.edge, y: edge.top, w, h: edge.h };
    room = room === null ? box : union(room, box);
  }
  return room;
}
const union = (a, b) => {
  const x = Math.min(a.x, b.x), y = Math.min(a.y, b.y);
  return { x, y, w: Math.max(a.x + a.w, b.x + b.w) - x, h: Math.max(a.y + a.h, b.y + b.h) - y };
};

export function captionReach(side, captions, metrics) {
  let reach = 0;
  for (const caption of captions) {
    const box = captionRoom(metrics, side, caption);
    if (box) reach = Math.max(reach, side === 'in' ? -box.x : box.x + box.w);
  }
  return reach;
}

export function captionBoxes(card, metrics, at = card, y = row => row[2] + metrics.inset) {
  const boxes = [];
  for (const side of ['in', 'out']) for (const row of card[side] ?? []) {
    const [name, , , label = name, caption = label] = row, box = captionRoom(metrics, side, caption);
    if (box) boxes.push({ side, x: at.x + (side === 'in' ? 0 : Number(card.width)) + box.x, y: at.y + y(row) + box.y, w: box.w, h: box.h });
  }
  return boxes;
}

export function captionRise(metrics) {
  let head = 0, rise = 0;
  for (const tier of TIERS) {
    const at = metrics.port(tier);
    if (!at) continue;
    head = Math.max(head, at.head);
    rise = Math.max(rise, -at.in.top, -at.out.top);
  }
  return { head, rise };
}

const PORT_AT = 100, WIDEST = 'M'.repeat(400);
const PORT_PROBE = ['in', 'out'].map(side => `<button class="node-port ${side}" style="top:${PORT_AT}px">`
  + '<span class="port-jack"></span><span title="">M</span></button>').join('');
const CHIP_PROBE = '<span class="component-caption"></span><span class="component-terminal left"></span>'
  + '<span class="component-core"><svg class="icon"></svg><strong></strong></span><span class="component-terminal right"></span>';
const PROBE = '<header class="node-header"><span class="node-icon"><svg class="icon"></svg></span>'
  + '<div class="node-titles"><div class="node-title">M</div><div class="node-type"></div></div>'
  + '<span class="node-life tiny-dot"></span><button class="viewer-toggle"><svg class="icon"></svg></button></header>'
  + '<div class="node-viewer"><div class="viewer-places"><span></span><button></button></div></div>'
  + '<button class="node-resize"><span></span></button>';
const WIDE = 1000, TALL = 400;
const drawnAt = tier => symbolScale(leastZoom(tier) || leastZoom(TIERS[TIERS.indexOf(tier) + 1]));
function probed(doc) {
  const canvas = doc.getElementById('canvas'), host = doc.getElementById('nodes');
  if (!canvas || !host) return null;
  const probe = doc.createElement('article');
  probe.className = 'node';
  probe.setAttribute('aria-hidden', 'true');
  probe.style.cssText = `visibility:hidden;left:0;top:0;width:${WIDE}px;height:${TALL}px;min-width:0;--hf-09-symbol-scale:1`;
  probe.innerHTML = PROBE;
  host.append(probe);
  const box = selector => probe.querySelector(selector).getBoundingClientRect();
  const wide = width => { probe.style.width = `${width}px`; return probe.getBoundingClientRect(); };
  return { probe, remove: () => probe.remove(),
    at(tiers, scales, read) {
      const held = canvas.getAttribute('data-tier'), seen = {}, hidden = [];
      for (let view = canvas; view; view = view.parentElement)
        if (view.classList.contains('hidden')) { hidden.push(view); view.classList.remove('hidden'); }
      for (const tier of tiers) {
        canvas.setAttribute('data-tier', tier);
        seen[tier] = scales.map(scale => {
          probe.style.setProperty('--hf-09-symbol-scale', scale);
          const whole = wide(WIDE);
          return read(whole, box, WIDE / whole.width, wide, tier);
        });
      }
      probe.style.setProperty('--hf-09-symbol-scale', 1);
      wide(WIDE);
      if (held === null) canvas.removeAttribute('data-tier'); else canvas.setAttribute('data-tier', held);
      for (const view of hidden) view.classList.add('hidden');
      return seen;
    } };
}

export async function documentMetrics(doc, registry) {
  const stand = probed(doc);
  if (!stand) return UNMEASURED;
  stand.probe.insertAdjacentHTML('beforeend', PORT_PROBE);
  const chips = doc.getElementById('wire-components'), chipProbe = chips && doc.createElement('button');
  if (chipProbe) {
    chipProbe.className = 'wire-component';
    chipProbe.setAttribute('aria-hidden', 'true');
    chipProbe.style.cssText = 'visibility:hidden;left:0;top:0';
    chipProbe.innerHTML = CHIP_PROBE;
    chips.append(chipProbe);
  }
  try {
    const inset = parseFloat(doc.defaultView.getComputedStyle(stand.probe).borderTopWidth);
    const name = stand.probe.querySelector('.node-title'), style = doc.defaultView.getComputedStyle(name);
    const font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
    await doc.fonts.load(font);
    const [inlet, outlet] = stand.probe.querySelectorAll('.node-port'), plates = [inlet, outlet].map(port => port.lastElementChild);
    const plateStyle = doc.defaultView.getComputedStyle(plates[0]);
    const plateFont = `${plateStyle.fontStyle} ${plateStyle.fontWeight} ${plateStyle.fontSize} ${plateStyle.fontFamily}`;
    await doc.fonts.load(plateFont);
    const under = () => parseFloat(doc.defaultView.getComputedStyle(stand.probe.querySelector('.node-viewer')).marginBottom);
    const chipAt = (tier, unit) => {
      if (!chipProbe) return NO_CHIP;
      chipProbe.style.setProperty('--hf-09-symbol-scale', drawnAt(tier));
      const chip = chipProbe.getBoundingClientRect();
      return Object.freeze({ w: unit * chip.width, h: unit * chip.height });
    };
    const portAt = tier => {
      const scale = drawnAt(tier), was = stand.probe.style.getPropertyValue('--hf-09-symbol-scale');
      stand.probe.style.setProperty('--hf-09-symbol-scale', scale);
      try {
        if (!plates.every(plate => plate.getClientRects().length)) return null;
        const card = stand.probe.getBoundingClientRect(), k = WIDE / card.width, jack = inlet.getBoundingClientRect(), y = jack.top + jack.height / 2;
        const width = text => { plates[0].textContent = text; return plates[0].getBoundingClientRect().width * k; };
        const side = (rect, edge) => Object.freeze({ edge: edge * k, top: (rect.top - y) * k, h: rect.height * k });
        const [a, b] = plates.map(plate => plate.getBoundingClientRect());
        const at = Object.freeze({ scale, head: (stand.probe.querySelector('.node-header').getBoundingClientRect().bottom - card.top) * k,
          in: side(a, a.right - card.left), out: side(b, b.left - card.right), frame: width(''), max: width(WIDEST) });
        plates[0].textContent = 'M';
        return at;
      } finally {
        stand.probe.style.setProperty('--hf-09-symbol-scale', was);
      }
    };
    const resize = stand.probe.querySelector('.node-resize');
    const footAt = tier => {
      const scale = drawnAt(tier), was = stand.probe.style.getPropertyValue('--hf-09-symbol-scale');
      stand.probe.style.setProperty('--hf-09-symbol-scale', scale);
      try {
        if (!resize.getClientRects().length) return 0;
        const card = stand.probe.getBoundingClientRect(), k = WIDE / card.width;
        return (inlet.getBoundingClientRect().height / 2 + card.bottom - resize.getBoundingClientRect().top) * k;
      } finally {
        stand.probe.style.setProperty('--hf-09-symbol-scale', was);
      }
    };
    const seen = stand.at(TIERS, [1], (whole, box, unit, wide, tier) => ({
      chrome: unit * (whole.width - box('.node-titles').width),
      head: unit * (box('.node-header').height - box('.node-title').height),
      row: unit * box('.viewer-places').height + under(),
      chip: chipAt(tier, unit),
      port: portAt(tier),
      foot: footAt(tier),
      titles: unit * box('.node-titles').height }));
    const context = doc.createElement('canvas').getContext('2d');
    context.font = font;
    context.letterSpacing = style.letterSpacing;
    const words = await declaredWords(doc, stand, registry, under);
    const plateContext = doc.createElement('canvas').getContext('2d');
    plateContext.font = plateFont;
    plateContext.letterSpacing = plateStyle.letterSpacing;
    return Object.freeze({ label: text => context.measureText(text).width,
      line: parseFloat(style.lineHeight), chrome: tier => seen[tier][0].chrome, head: tier => seen[tier][0].head, row: tier => seen[tier][0].row,
      chip: tier => seen[tier]?.[0].chip ?? NO_CHIP, port: tier => seen[tier]?.[0].port ?? null,
      caption: text => plateContext.measureText(text).width, inset, portFoot: Math.max(...TIERS.map(tier => seen[tier][0].foot)),
      nameFoot: seen[NAMES][0].titles - parseFloat(style.lineHeight),
      words: kind => words.get(kind) });
  } finally {
    stand.remove();
    chipProbe?.remove();
  }
}

async function declaredWords(doc, stand, registry, under) {
  const declaring = registry ? registry.all().filter(entry => entry.size?.words) : [];
  const viewer = stand.probe.querySelector('.node-viewer'), measured = new Map();
  const read = measure => declaring.length ? stand.at(['reduced'], [1], (whole, box, unit) => declaring.map(entry => {
    const held = viewer.innerHTML;
    viewer.dataset.viewer = entry.kind;
    viewer.style.flex = 'none';
    viewer.innerHTML = entry.render({ id: 'probe', config: { [entry.size.words]: 'M' }, viewConfig: {} }, {}, 'reduced', false);
    try {
      const element = viewer.querySelector(`[data-words="${entry.size.words}"]`);
      return element && measure(entry, element, whole, unit);
    } finally {
      viewer.innerHTML = held;
      viewer.style.flex = '';
      delete viewer.dataset.viewer;
    }
  })).reduced[0] : [];
  const face = element => { const s = doc.defaultView.getComputedStyle(element); return { font: `${s.fontStyle} ${s.fontWeight} ${s.fontSize} ${s.fontFamily}`, spacing: s.letterSpacing }; };
  for (const typeset of read((entry, element) => face(element))) if (typeset) await doc.fonts.load(typeset.font);
  for (const body of read((entry, element, whole, unit) => {
    const box = element.getBoundingClientRect();
    return { kind: entry.kind, ...face(element), frame: unit * viewer.getBoundingClientRect().height + under(), line: unit * box.height,
      chrome: unit * (whole.width - box.width) };
  })) {
    if (!body) continue;
    const typeset = doc.createElement('canvas').getContext('2d');
    typeset.font = body.font;
    typeset.letterSpacing = body.spacing;
    measured.set(body.kind, Object.freeze({ frame: body.frame, line: body.line, chrome: body.chrome, advance: text => typeset.measureText(text).width }));
  }
  return measured;
}

export function controlRoom(doc) {
  const stand = probed(doc);
  if (!stand) return undefined;
  try {
    const seen = stand.at(TIERS.filter(tier => tier !== DETAIL), [1, 2], (whole, box, unit, wide) => {
      const beyond = whole.right - box('.viewer-places > button').right;
      const height = unit * (whole.height - box('.node-viewer').height + box('.viewer-places').height);
      const narrow = wide(0);
      return { width: unit * (box('.viewer-places > button').right - narrow.left + beyond), height };
    });
    const part = (one, two) => Object.freeze({
      symbol: Object.freeze({ width: two.width - one.width, height: two.height - one.height }),
      world: Object.freeze({ width: 2 * one.width - two.width, height: 2 * one.height - two.height }) });
    return Object.freeze(Object.fromEntries(Object.entries(seen).map(([tier, [one, two]]) => [tier, part(one, two)])));
  } finally {
    stand.remove();
  }
}
