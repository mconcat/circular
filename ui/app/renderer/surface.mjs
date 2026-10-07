import { views } from './views.mjs';
import { escape as e, renderView, liveViewers, formatTime } from './view-registry.mjs';
import { latestRow, viewRecords } from './arrivals.mjs';
import { recordValue } from './record-text.mjs';
import { codeText, reasonText } from './reasons.mjs';

const unavailable = (code, detail = '') => `<p class="surface-reason" data-reason="${e(codeText(code))}" role="status">${e(reasonText(code))}${detail ? ` · ${e(detail)}` : ''}</p>`;
const staticValue = (value, definition) => value?.$circular === 'parameter'
  ? definition.params?.[value.name]?.default : value;
export const surfaceTitle = definition => definition?.surfaces?.map(mark => staticValue(mark.spec?.title, definition))
  .filter(value => typeof value === 'string').join(' · ') || undefined;
const bound = (surface, ref) => surface.views.find(view => view.role === ref?.role);
const directionOf = binding => binding?.node?.in?.some(([port]) => port === binding.port)
  && !binding.node.out?.some(([port]) => port === binding.port) ? 'arrivals' : 'emitted';
const rowsOf = binding => binding?.node
  ? viewRecords(binding.node, directionOf(binding)).filter(row => row.port === binding.port) : [];
const inputKey = (surface, path) => `${surface.id}/${path}`;
const ownInput = (viewer, binding, surface, path) => viewer?.card(binding?.actor).surfaceInputs?.[inputKey(surface, path)] ?? {};
const markAt = (surface, path) => path.split('.').reduce((parent, index) => parent?.children?.[Number(index)],
  { children: surface.declaration.surface?.surfaces });
const time = row => row?.observed_at_ms == null ? null : formatTime(row.observed_at_ms);
const portHost = binding => `data-view-host="${e(binding.actor)}" data-view-port="${e(binding.port)}"`;

function cellStyle(mark) {
  const cell = mark.modifiers?.cell;
  if (!cell) return '';
  const { col, row, width, height } = cell;
  if (![col, row, width, height].every(Number.isSafeInteger) || col < 0 || row < 0 || width < 1 || height < 1) return null;
  return `--surface-column:${col + 1} / span ${width};--surface-row:${row + 1} / span ${height}`;
}

export function surfacePortView(binding, kind, registry = views) {
  if (!binding?.node) return { code: 'READ_UNAVAILABLE' };
  const rows = rowsOf(binding), direction = directionOf(binding);
  const observation = { id: binding.node.id, declaration: binding.node.declaration, config: binding.node.config,
    viewConfig: binding.node.viewConfig,
    in: (binding.node.in ?? []).filter(([port]) => direction === 'arrivals' && port === binding.port),
    out: (binding.node.out ?? []).filter(([port]) => direction === 'emitted' && port === binding.port),
    portRecords: { port: binding.port, direction, rows } };
  const descriptor = registry.of(kind);
  Object.assign(observation, descriptor.page?.(observation, { items: viewRecords(observation) }));
  const input = descriptor.input?.(observation, binding.context);
  return { node: { id: binding.actor, viewKind: { kind, code: null }, ...input },
    code: rows.length ? null : direction === 'arrivals' ? 'ARRIVAL_UNOBSERVED' : 'EMISSION_UNOBSERVED' };
}

function observedMark(mark, surface, path, options, kind) {
  return [mark.spec?.role, mark.spec?.own].filter(Boolean).map((ref, index) => {
    const binding = bound(surface, ref), reading = surfacePortView(binding, kind, options.registry);
    if (reading.code) return unavailable(reading.code, ref.role);
    const render = options.renderView ?? ((node, own) => renderView(options.registry, node, own));
    return `<div class="output-content" data-output-mark="${e(path)}" data-output-stream="${index}" data-output-node="${e(binding.actor)}" ${portHost(binding)}>${render(reading.node, {})}</div>`;
  }).join('');
}

function control(mark, surface, path, options) {
  const binding = bound(surface, mark.spec?.role), spec = mark.spec ?? {}, definition = surface.declaration.surface;
  const own = ownInput(options.viewer, binding, surface, path);
  const value = name => staticValue(spec[name], definition);
  const unresolved = [...Object.values(spec), mark.modifiers?.placeholder]
    .some(value => value?.$circular === 'parameter' && staticValue(value, definition) === undefined);
  if (unresolved) return unavailable('READ_UNAVAILABLE');
  const code = surface.submitCode ?? (!binding?.node || binding.role !== 'request' ? 'INLET_INJECTION_UNAVAILABLE' : null);
  const disabled = code ? ` disabled title="${e(reasonText(code))}" data-reason="${e(codeText(code))}"` : '';
  const label = value('label');
  const attrs = `data-surface-input="${e(path)}" data-surface-id="${e(surface.id)}"`;
  const answer = `<output data-surface-answer>${own.answer ? unavailable(own.answer.code) : ''}</output>`;
  const field = `<label>${e(label ?? 'Message')}<${mark.mark === 'composer' ? 'textarea' : 'input'} name="value" aria-label="${e(label ?? 'Message')}" placeholder="${e(value('placeholder') ?? staticValue(mark.modifiers?.placeholder, definition) ?? '')}"${disabled}${mark.mark === 'composer' ? `>${e(own.value ?? '')}</textarea>` : ` type="text" value="${e(own.value ?? '')}">`}</label>`;
  let body;
  switch (mark.mark) {
    case 'composer': body = `${field}<button class="dark-button" type="submit"${disabled}>${e(label ?? 'Send')} ↗</button>`; break;
    case 'textInput': body = field; break;
    case 'button': {
      const payloadCode = value('send') === undefined ? 'READ_UNAVAILABLE' : null;
      body = `<button class="dark-button" type="submit"${disabled || (payloadCode ? ` disabled title="${e(reasonText(payloadCode))}" data-reason="${payloadCode}"` : '')}>${e(label ?? 'Send')}</button>${payloadCode ? unavailable(payloadCode, 'Button payload not declared') : ''}`;
      break;
    }
    case 'toggle': {
      const recorded = latestRow(rowsOf(binding))?.body;
      const checked = own.value ?? (spec.bind === undefined ? recorded : recorded?.[spec.bind]);
      body = `<label><input name="value" type="checkbox"${checked === true ? ' checked' : ''}${typeof checked !== 'boolean' ? ' data-unobserved="true"' : ''}${disabled}>${e(label ?? 'Toggle')}</label>`;
      if (typeof checked !== 'boolean') body += unavailable(checked === undefined ? 'EMISSION_UNOBSERVED' : 'CONFIG_VALUE_INVALID');
      break;
    }
    case 'select': body = `<label>${e(label ?? 'Select')}<select name="value"${disabled}><option value="" disabled${own.value === undefined ? ' selected' : ''}>Select…</option>${(spec.options ?? []).map(option => `<option value="${e(option)}"${own.value === option ? ' selected' : ''}>${e(option)}</option>`).join('')}</select></label>`; break;
  }
  return `<form class="surface-control surface-${e(mark.mark)}" ${attrs}>${body}${code ? unavailable(code) : ''}${answer}</form>`;
}

const marks = new Map([
  ['tab', container], ['window', container],
  ['grid', (mark, surface, path, options) => {
    const spec = mark.spec ?? {}, definition = surface.declaration.surface;
    const cols = staticValue(spec.cols, definition), height = staticValue(spec.row_h, definition);
    if (cols !== undefined && (!Number.isSafeInteger(cols) || cols < 1)
      || height !== undefined && (typeof height !== 'number' || !Number.isFinite(height) || height <= 0)) return unavailable('CONFIG_VALUE_INVALID');
    if (spec.cols !== undefined && cols === undefined || spec.row_h !== undefined && height === undefined) return unavailable('READ_UNAVAILABLE');
    const style = [cols === undefined ? '' : `--surface-columns:repeat(${cols}, minmax(0, 1fr))`,
      height === undefined ? '' : `--surface-row-height:minmax(${height}px, auto)`].filter(Boolean).join(';');
    return `<div class="surface-layout" style="${style}">${children(mark, surface, path, options)}</div>`;
  }],
  ['label', (mark, surface) => {
    const value = staticValue(mark.spec?.value, surface.declaration.surface);
    return value === undefined ? unavailable('READ_UNAVAILABLE') : `<p class="surface-label">${e(value)}</p>`;
  }],
  ['messages', (m, s, p, o) => observedMark(m, s, p, o, 'feed')],
  ['terminal', (m, s, p, o) => observedMark(m, s, p, o, 'transcript')],
  ['transcript', (m, s, p, o) => observedMark(m, s, p, o, 'transcript')],
  ...['textInput', 'button', 'toggle', 'select', 'composer'].map(kind => [kind, control]),
]);
function children(mark, surface, path, options) {
  return (mark.children ?? []).map((child, index) => renderMark(child, surface, `${path}.${index}`, options)).join('');
}
function container(mark, surface, path, options) {
  const title = staticValue(mark.spec?.title, surface.declaration.surface);
  return `<section class="surface-section"><header>${title === undefined ? unavailable('READ_UNAVAILABLE') : `<h2>${e(title)}</h2>`}</header>${children(mark, surface, path, options)}</section>`;
}
function renderMark(mark, surface, path, options) {
  const render = marks.get(mark?.mark), placement = cellStyle(mark);
  if (!render) return unavailable('VIEW_KIND_UNREGISTERED', mark?.mark);
  if (placement === null) return unavailable('CONFIG_VALUE_INVALID');
  return `<div class="surface-cell" data-surface-mark="${e(mark.mark)}" style="${placement}">${render(mark, surface, path, options)}</div>`;
}

function provenance(surface) {
  const seen = surface.views.filter(binding => binding.role !== 'request').map(binding => {
    const at = time(latestRow(rowsOf(binding)));
    return `<p><span>Updated from ${e(binding.node?.title ?? binding.role)}</span> ${at ? `<span title="${e(at.title)}">${e(at.text)}</span>` : `<span data-reason="EMISSION_UNOBSERVED">${e(reasonText('EMISSION_UNOBSERVED'))}</span>`}</p>`;
  }).join('');
  return `<footer class="surface-provenance">${seen}</footer>`;
}
function details(surface) {
  const rows = surface.views.map(binding => `<li>${e(binding.role)} · <code>${e(binding.port)}</code> <button class="quiet-button" data-locate="${e(binding.actor)}"${binding.node ? '' : ` disabled title="${e(reasonText('READ_UNAVAILABLE'))}" data-reason="READ_UNAVAILABLE"`}>Open ${e(binding.node?.title ?? binding.role)} on canvas</button></li>`).join('');
  const identifiers = { mount: surface.address, roles: surface.declaration.roles };
  return `<details class="surface-details"><summary>Source & details</summary><ul>${rows}</ul><pre>${e(JSON.stringify(recordValue(identifiers), null, 2))}</pre></details>`;
}

const injectingKind = (node, registry) => [node?.viewKind?.kind, registry.defaultFor(node?.type)?.kind]
  .find(kind => registry.has(kind) && registry.of(kind).traits.injects);
function requestBody(binding, opts, render) {
  const kind = binding.node && injectingKind(binding.node, opts.registry);
  if (!kind) return unavailable(binding.node ? 'INLET_INJECTION_UNAVAILABLE' : 'READ_UNAVAILABLE', binding.role);
  const { node } = surfacePortView(binding, kind, opts.registry);
  return `<div class="output-content" data-output-role="${e(binding.role)}" data-output-node="${e(binding.actor)}" ${portHost(binding)}>${render(node, opts.viewer?.card?.(binding.actor) ?? {})}</div>`;
}
const roleWord = role => role.charAt(0).toUpperCase() + role.slice(1);

function rolesBody(surface, opts) {
  const render = opts.renderView ?? ((node, own) => renderView(opts.registry, node, own));
  const roles = surface.views.map(binding => `<span class="status-pill" data-role="${e(binding.role)}">${e(roleWord(binding.role))}</span>`).join('');
  return `<header><h2>${e(surface.name)}</h2><div class="surface-roles">${roles}</div></header>` + surface.views.map(binding => {
    if (binding.role === 'request') return requestBody(binding, opts, render);
    const reading = surfacePortView(binding, binding.node?.viewKind?.kind ?? 'basic', opts.registry);
    if (reading.code) return unavailable(reading.code, binding.role);
    return `<div class="output-content" data-output-role="${e(binding.role)}" data-output-node="${e(binding.actor)}" ${portHost(binding)}>${render(reading.node, {})}</div>`;
  }).join('');
}

const pin = (surface, pinned, refused) => pinned === undefined ? ''
  : `<button type="button" class="surface-pin" data-pin-surface="${e(surface.id)}" aria-pressed="${pinned ? 'true' : 'false'}"${refused ? ` data-reason="${e(codeText(refused))}"` : ''} title="${e(refused ? reasonText(refused) : pinned ? 'Pinned first on this device. Press to unpin.' : 'Keep this card first on this device.')}">${pinned ? 'Pinned' : 'Pin'}</button>${refused ? unavailable(refused) : ''}`;

export function renderSurface(surface, options = {}) {
  const opts = { registry: views, ...options }, definition = surface.declaration.surface;
  const declared = definition?.$circular === 'export-definition' && Array.isArray(definition.surfaces);
  const body = declared
    ? definition.surfaces.map((mark, index) => renderMark(mark, surface, String(index), opts)).join('')
    : rolesBody(surface, opts);
  const ended = surface.ended ? ` data-reason="${e(codeText(surface.ended.code))}"` : '';
  return `<article class="surface-card"${declared ? ' data-surface-layout="declared"' : ''} data-export-surface="${e(surface.id)}"${ended}>${pin(surface, opts.pinned, opts.pinRefused)}${body}${provenance(surface)}${details(surface)}</article>`;
}

export function updateSurface(element, surface, { registry = views } = {}) {
  const source = { observation: (node, t) => ({ value: null, ...registry.of(node.viewKind.kind).sample?.(node, t) }) };
  const viewers = liveViewers(registry, { StudySource: source, StudyPaint: globalThis.window?.StudyPaint });
  for (const host of element.querySelectorAll('[data-output-mark],[data-output-role]')) {
    const mark = host.dataset.outputRole ? null : markAt(surface, host.dataset.outputMark);
    const ref = mark ? (Number(host.dataset.outputStream) ? mark.spec.own : mark.spec.role) : { role: host.dataset.outputRole };
    const kind = host.querySelector('[data-viewer]')?.dataset.viewer;
    if (!kind) continue;
    const binding = bound(surface, ref);
    host.viewProjection = () => ({ node: surfacePortView(binding, kind, registry).node, source });
    viewers.update(host, undefined, globalThis.window?.StudyApp?.displayTime?.() ?? globalThis.window?.StudySource?.head ?? Infinity);
  }
  for (const input of element.querySelectorAll('[data-surface-input] input[type="checkbox"]')) input.indeterminate = input.hasAttribute('data-unobserved');
}

export function bindSurfaceInputs(root, { surfaces, viewer, inject }) {
  const read = target => {
    const form = target.closest('[data-surface-input]');
    if (!form) return null;
    const surface = surfaces().find(item => item.id === form.dataset.surfaceId);
    if (!surface) return null;
    const path = form.dataset.surfaceInput, mark = markAt(surface, path), binding = bound(surface, mark?.spec?.role);
    if (!binding) return null;
    const field = form.elements.namedItem('value');
    const value = mark.mark === 'button' ? staticValue(mark.spec.send, surface.declaration.surface)
      : field?.type === 'checkbox' ? field.checked : field?.value;
    return { form, surface, path, mark, binding, value };
  };
  const remember = (entry, parts) => {
    const held = viewer.card(entry.binding.actor).surfaceInputs ?? {}, key = inputKey(entry.surface, entry.path);
    viewer.change(entry.binding.actor, { surfaceInputs: { ...held, [key]: { ...held[key], ...parts } } });
  };
  const submit = async entry => {
    if (!entry) return;
    remember(entry, { value: entry.value });
    const code = entry.surface.submitCode ?? (entry.value === undefined ? 'READ_UNAVAILABLE' : null);
    const answer = code ? { code } : await inject(entry.surface.id, entry.value);
    remember(entry, { answer });
    const line = entry.form.querySelector('[data-surface-answer]');
    if (line) line.innerHTML = unavailable(answer.code);
  };
  root.oninput = event => { const entry = read(event.target); if (entry) remember(entry, { value: entry.value }); };
  root.onsubmit = event => { const entry = read(event.target); if (entry) { event.preventDefault(); return submit(entry); } };
  root.onchange = event => { const entry = read(event.target); if (entry && ['toggle', 'select'].includes(entry.mark.mark)) return submit(entry); };
}
