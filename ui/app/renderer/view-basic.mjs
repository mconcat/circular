import { journalRows, readFirstPage } from './query.mjs';
import { draw, escape, spark, formatReading, formatRate, kicker } from './view-registry.mjs';
import { distribute } from './arrivals.mjs';
import { observeActors } from './scene.mjs';
import { codeText, reason, reasonText } from './reasons.mjs';
import { tallyCell } from './journal.mjs';
import { RATE_SECONDS, RATE_UNIT } from './activity.mjs';

const CELLS = 40;
export function basicReading(node, t, source) {
  if (node?.basicCode) return {code: node.basicCode};
  if (!Number.isFinite(t)) t = source?.head ?? 0;
  const series = source?.arrivalSeries?.(node.id, t, CELLS);
  if (!series) return {code: 'READ_UNAVAILABLE'};
  const counted = source.arrivalsIn?.(node.id, t - RATE_SECONDS, t);
  return {recorded: node.recordedArrivals ?? null, count: counted?.count ?? 0, seconds: counted?.seconds ?? 0, whole: Boolean(counted?.whole),
    points: series.map(cell => cell == null ? null : cell / tallyCell), unread: series.some(cell => cell == null), at: t};
}

export function basicInput(node) {
  return {basicCode: node.basicDiagnostic ?? null};
}

export async function readBasicArrivals(session, graph, observedPage, mine) {
  const drawn = new Set(graph.nodes.filter(mine).map(node => node.id));
  if (!drawn.size) return graph;
  const mark = (read, basicDiagnostic) => observeActors(read, () => ({basicDiagnostic}), drawn);
  if (observedPage) return mark(graph, null);
  try { return mark(distribute(graph, await readFirstPage(session, 'actor.events', null, journalRows), drawn), null); }
  catch (error) { return mark(distribute(graph, {items: []}, drawn), codeText(error.code ?? 'READ_UNAVAILABLE')); }
}

const arrivalsRecorded = count => count == null ? '—' : `${count} ${String(count) === '1' ? 'arrival' : 'arrivals'} recorded`;
const displayTime = () => globalThis.window?.StudyApp?.displayTime?.() ?? globalThis.window?.StudySource?.head ?? 0;
const spell = seconds => globalThis.window?.studyTimeFormat?.(seconds) ?? `${seconds.toFixed(3)} s`;

export function updateBasicView(card, node, t = displayTime(), source = globalThis.window?.StudySource) {
  const viewer = card?.querySelector('.node-viewer[data-viewer="basic"]');
  if (!viewer) return;
  const reading = basicReading(node, t, source);
  const code = reading.code ?? null;
  const unread = reason('DENSITY_UNREAD');
  const detail = code ? reasonText(code) : [arrivalsRecorded(reading.recorded),
    reading.whole ? `${formatReading(reading.count).full} in the last ${RATE_SECONDS} s of recorded time up to ${spell(reading.at)}`
      : unread.label,
    reading.seconds && reading.unread ? `earlier cells: ${unread.label}` : null].filter(Boolean).join(' · ');
  viewer.setAttribute('aria-disabled', String(Boolean(code)));
  viewer.title = detail;
  const diagnostic = code ?? (!reading.whole || reading.unread ? unread.code : null);
  const rate = viewer.querySelector('[data-live="rate"]');
  if (rate) {
    const formatted = formatRate(reading, code);
    rate.textContent = formatted.text;
    rate.title = formatted.title;
  }
  const unit = viewer.querySelector('.viewer-reading > span');
  if (unit) unit.textContent = code ? reasonText(code) : RATE_UNIT;
  const recorded = viewer.querySelector('.viewer-reading > .viewer-count');
  if (recorded) recorded.textContent = code ? '' : arrivalsRecorded(reading.recorded);
  const strip = viewer.querySelector('.hf-08-arrival-strip');
  if (strip) {
    strip.setAttribute('data-observed', String(!code));
    strip.setAttribute('aria-label', detail);
    strip.title = detail;
    const maximum = Math.max(0, ...(reading.points ?? []).filter(value => value != null));
    for (const [i, mark] of [...strip.children].entries()) {
      const value = code ? null : reading.points[i];
      mark.setAttribute('data-arrival', String(value > 0));
      mark.style.visibility = value == null ? 'hidden' : '';
      if (value == null) {
        mark.setAttribute('data-reason', codeText(code ?? unread.code));
        mark.style.removeProperty('--hf-08-density');
      } else {
        mark.removeAttribute('data-reason');
        mark.style.setProperty('--hf-08-density', maximum ? String(value / maximum) : '0');
      }
    }
  }
  const canvas = viewer.querySelector('.viewer-spark');
  if (canvas) {
    canvas.setAttribute('aria-disabled', String(Boolean(code)));
    canvas.setAttribute('aria-label', detail);
    canvas.title = detail;
    if (code) canvas.getContext('2d')?.clearRect(0, 0, canvas.width, canvas.height);
    else draw(canvas, reading.points);
  }
  for (const slot of [rate, unit, strip, canvas]) {
    if (diagnostic) slot?.setAttribute?.('data-reason', codeText(diagnostic)); else slot?.removeAttribute?.('data-reason');
  }
}

export default {
  kind: 'basic',
  render: n => {
    const rate = formatRate(null);
    return `${kicker(n.viewConfig?.heading)}<div class="viewer-reading"><strong data-live="rate" title="${escape(rate.title)}">${rate.text}</strong><span>${RATE_UNIT}</span><span class="viewer-count"></span></div>${spark}<div class="hf-08-arrival-strip" role="img" aria-label="Arrival observation unavailable">${Array.from({length:40}, () => '<i aria-hidden="true"></i>').join('')}</div>`;
  },
  glance: {
    cells: (n, t, source) => {
      const reading = basicReading(n, t, source), code = reading.code ?? null, rate = formatRate(reading, code);
      return { rate: code ? { reason: code } : { count: rate.text, unit: RATE_UNIT, title: rate.title, code: rate.code },
        recorded: code ? null : { words: arrivalsRecorded(reading.recorded) } };
    },
    reduced: ['rate', 'recorded'],
    names: ['rate'],
  },
  defaultFor: [],
  reads: [],
  size: { height: 206, min: 174 },
  input: basicInput,
  update: updateBasicView,
  tick: 'replace',
  read: readBasicArrivals,
  page: node => node.basicDiagnostic ? { basicDiagnostic: null } : undefined,
};
