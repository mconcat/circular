import { configFieldList } from './config-fields.mjs';
import { fieldName } from './config-form.mjs';
import { reasonText } from './reasons.mjs';
import { escape, kicker, formatReading, formatDuration, formatTime } from './view-registry.mjs';
import { isReceived, primaryOutletReading, stampSequence, viewRecords } from './arrivals.mjs';
import { rowText } from './value-text.mjs';
const numeric = ({ kind }) => kind === 'integer' || kind === 'number';
const recorded = row => typeof row.observed_at_ms === 'bigint';
const stamp = (row, count) => ({ at: Number(row.observed_at_ms) / 1000, ms: String(row.observed_at_ms), count });

export function timingInput(node, ctx) {
  const emitted = ctx.emitted(node), entry = ctx.configEntry(node), readCode = ctx.configCode;
  const config = node.declaration?.config;
  const list = configFieldList(config, true, entry, readCode);
  const durations = list.fields.filter(field => numeric(field) && field.duration);
  const field = durations.length === 1 ? durations[0] : null;
  const value = field ? formatReading(config?.[field.key])?.full ?? null : null;
  const state = node.health?.state ?? 'unobserved';
  const primary = primaryOutletReading(node, ctx.graph.edges);
  return { timing: {
    key: field?.key ?? null, name: field ? fieldName(field) : null, value,
    code: list.code ?? (durations.length > 1 ? 'READ_UNAVAILABLE' : value === null ? 'UNDECLARED' : null),
    received: viewRecords(node, 'arrivals', { sided: true }).filter(row => recorded(row) && isReceived(row))
      .map(row => ({ at: Number(row.observed_at_ms) / 1000, ms: String(row.observed_at_ms) })),
    emitted: {
      stamps: (emitted ?? []).filter(recorded).map(row => {
        const sequence = stampSequence(row);
        return stamp(row, sequence === null ? null : sequence + 1n);
      }),
      code: emitted === undefined ? 'READ_UNAVAILABLE' : 'EMISSION_UNOBSERVED',
    },
    ring: ctx.pause ? 'paused' : state === 'running' ? 'running' : state === 'waiting' ? 'idle' : 'unobserved',
    state, pause: ctx.pause?.label ?? null, pauseCode: ctx.pause?.code ?? null,
    port: primary?.port ?? null,
    emissions: primary === null ? [] : (emitted ?? [])
      .filter(row => row.port === primary.port && recorded(row))
      .map(row => ({ at: Number(row.observed_at_ms) / 1000, text: rowText(node, row, 'emitted').text })),
  } };
}

export function sideAt(side, t = Infinity) {
  let count = null, ms = null;
  for (const row of side?.stamps ?? []) {
    if (row.at > t) continue;
    if (row.count !== null && !(count >= row.count)) count = row.count;
    if (ms === null || BigInt(row.ms) > BigInt(ms)) ms = row.ms;
  }
  return count === null ? { code: side?.code ?? 'READ_UNAVAILABLE' } : { count, ms };
}

export function receivedLine(timing, t = Infinity) {
  const latest = (timing?.received ?? []).reduce((best, row) => row.at <= t && !(best?.at > row.at) ? row : best, null);
  return latest ? formatTime(latest.ms) : null;
}
const receivedLead = 'Last received';
const receivedHTML = time => `<span data-fact>${escape(receivedLead)}</span><span data-fact>${escape(time?.text ?? '')}</span>`;
const receivedTitle = time => time ? `${receivedLead} ${time.detail}` : '';

export function emittedLine(timing, t = Infinity) {
  const latest = (timing?.emissions ?? []).reduce((best, row) => row.at <= t && !(best?.at > row.at) ? row : best, null);
  return latest ? { port: timing.port, text: latest.text } : null;
}
const emittedLead = line => line ? `Last emitted on ${line.port}` : '';
const emittedHTML = line => `<span data-fact>${escape(emittedLead(line))}</span><span data-fact>${escape(line?.text ?? '')}</span>`;
const emittedTitle = line => line ? `${emittedLead(line)}: ${line.text}` : '';

const write = (element, text) => { if (element && element.textContent !== text) element.textContent = text; };
const titled = (element, text) => { if (element && element.title !== text) element.title = text; };
const attributed = (element, code) => {
  if (!element) return;
  if (code) element.dataset.reason = code;
  else delete element.dataset.reason;
};
const reasonAttribute = code => code ? ` data-reason="${escape(code)}"` : '';
const durationCode = timing => timing?.code ?? (formatDuration(timing?.value) ? null : 'READ_UNAVAILABLE');
const declared = timing => {
  const duration = formatDuration(timing?.value), code = durationCode(timing);
  return { key: timing?.key ? timing.name : reasonText(code), text: duration?.text ?? '—', code,
    keyCode: timing?.key ? null : code, title: duration ? `${timing.key} · ${duration.title}` : reasonText(code) };
};
const ringCode = timing => timing?.ring === 'paused' ? timing.pauseCode : !timing || timing.ring === 'unobserved' ? 'unobserved' : null;
const ringLabel = timing => !timing || timing.ring === 'unobserved' ? reasonText('unobserved')
  : timing.ring === 'paused' ? timing.pause : timing.state;
const plural = (count, one, many) => count === 1n || count === 1 ? one : many;
const head = (timing, t, label) => {
  const read = sideAt(timing?.emitted, t), time = read.code ? null : formatTime(read.ms);
  return { read, time, label: label ?? (read.code ? '' : 'emitted'),
    title: read.code ? reasonText(read.code)
      : `${read.count} ${plural(read.count, 'emission', 'emissions')} · newest recorded ${time?.detail ?? read.ms}` };
};
const sideHTML = (name, side) => `<p class="timing-side" data-timing-side="${name}" title="${escape(side.title)}"${reasonAttribute(side.read.code)}>` +
  `<strong data-timing-count${side.read.code ? ' hidden' : ''}>${escape(side.read.code ? '—' : String(side.read.count))}</strong> ` +
  `<span data-timing-label${side.read.code ? ' hidden' : ''}>${escape(side.label)}</span>` +
  ` <small data-timing-at title="${escape(side.time?.title ?? reasonText(side.read.code))}"${reasonAttribute(side.read.code)}>${escape(side.time?.text ?? reasonText(side.read.code))}</small></p>`;
const sideFact = side => side.read.code ? { reason: side.read.code, title: side.title }
  : { count: String(side.read.count), noun: side.label, title: side.title };
function updateSide(row, side) {
  if (!row) return;
  titled(row, side.title);
  attributed(row, side.read.code);
  const count = row.querySelector('[data-timing-count]'), label = row.querySelector('[data-timing-label]');
  write(count, side.read.code ? '—' : String(side.read.count));
  write(label, side.label);
  const at = row.querySelector('[data-timing-at]');
  for (const slot of [count, label]) if (slot) slot.hidden = Boolean(at) && Boolean(side.read.code);
  write(at, side.time?.text ?? reasonText(side.read.code));
  titled(at, side.time?.title ?? reasonText(side.read.code));
  attributed(at, side.read.code);
}

export function updateTimingView(card, node, t = globalThis.window?.StudyApp?.displayTime?.() ?? Infinity) {
  if (!node) return;
  const viewer = card?.querySelector('.node-viewer[data-viewer="timing"]');
  const timing = node.timing;
  if (!viewer || !timing) return;
  const ring = viewer.querySelector('.timing-ring');
  if (ring) {
    ring.classList.toggle('idle', timing.ring !== 'running');
    ring.dataset.timingState = timing.ring;
    ring.setAttribute('aria-label', ringLabel(timing));
    attributed(ring, ringCode(timing));
  }
  updateSide(viewer.querySelector('[data-timing-side="emitted"]'), head(timing, t, node.viewConfig?.count_label));
  const duration = declared(timing);
  if (timing.code) viewer.setAttribute?.('data-reason', timing.code); else viewer.removeAttribute?.('data-reason');
  const declaredLine = viewer.querySelector('[data-timing-window]');
  titled(declaredLine, duration.title);
  attributed(declaredLine, duration.code);
  write(viewer.querySelector('[data-timing-key]'), duration.key);
  attributed(viewer.querySelector('[data-timing-key]'), duration.keyCode);
  write(viewer.querySelector('[data-timing-value]'), duration.text);
  titled(viewer.querySelector('[data-timing-value]'), duration.title);
  attributed(viewer.querySelector('[data-timing-value]'), duration.code);
  const received = viewer.querySelector('[data-timing-received]');
  if (received) {
    const time = receivedLine(timing, t), [lead, at] = [...(received.querySelectorAll?.('[data-fact]') ?? [])];
    write(lead, receivedLead);
    write(at, time?.text ?? '');
    titled(received, receivedTitle(time));
    received.hidden = !time;
  }
  const value = viewer.querySelector('[data-timing-emitted]');
  if (value) {
    const line = emittedLine(timing, t), [port, text] = [...(value.querySelectorAll?.('[data-fact]') ?? [])];
    write(port, emittedLead(line));
    write(text, line?.text ?? '');
    titled(value, emittedTitle(line));
    value.hidden = !line;
  }
}

export default {
  kind: 'timing',
  render: n => {
    const timing = n.timing, duration = declared(timing), emitted = emittedLine(timing), received = receivedLine(timing);
    const ring = `<span class="timing-ring ${timing?.ring === 'running' ? '' : 'idle'}" data-timing-state="${escape(timing?.ring ?? 'unobserved')}" aria-label="${escape(ringLabel(timing))}"${reasonAttribute(ringCode(timing))}></span>`;
    return kicker(n.viewConfig?.heading) +
      `<div class="timing-content"><div class="timing-head" data-timing-head>${sideHTML('emitted', head(timing, Infinity, n.viewConfig?.count_label))}</div>` +
      `<p class="facts" data-timing-received title="${escape(receivedTitle(received))}"${received ? '' : ' hidden'}>${receivedHTML(received)}</p>` +
      `<p class="timing-declared" data-timing-window title="${escape(duration.title)}"${reasonAttribute(duration.code)}><span data-timing-key${reasonAttribute(duration.keyCode)}>${escape(duration.key)}</span> <span data-timing-value title="${escape(duration.title)}"${reasonAttribute(duration.code)}>${escape(duration.text)}</span>${ring}</p>` +
      `<p class="facts" data-timing-emitted title="${escape(emittedTitle(emitted))}"${emitted ? '' : ' hidden'}>${emittedHTML(emitted)}</p></div>`;
  },
  glance: {
    cells: (n, t = Infinity) => ({ emitted: sideFact(head(n.timing, t, n.viewConfig?.count_label)) }),
    reduced: ['emitted'],
    names: ['emitted'],
  },
  defaultFor: ['timer', 'debounce'],
  size: { height: 248, min: 174 },
  input: timingInput,
  update: updateTimingView,
  tick: 'replace',
};
