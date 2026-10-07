import { performer } from './renderer/gesture.mjs';
import { escape, declaredCapacity } from './renderer/view-registry.mjs';
import { reasonText } from './renderer/reasons.mjs';
import { viewer } from './renderer/viewer.mjs';
import { flagWords } from './renderer/node-state.mjs';
import { outletReading } from './renderer/arrivals.mjs';
import { WIRE_RATE_SECONDS } from './renderer/activity.mjs';
import { key } from './renderer/scene.mjs';
import { place } from './renderer/placement.mjs';
import { draftChipPlace } from './renderer/combinators.mjs';
import { chipRoom, UNMEASURED } from './renderer/card-size.mjs';
import { outputsFromScene } from './renderer/outputs.mjs';
import { renderSurface, updateSurface } from './renderer/surface.mjs';

const clone = value => structuredClone(value);
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));

const configs = {
  route: { at: ['kind'], cases: { message: 'message', task: 'task' } },
  pipeline_actor: null,
  debounce: { quiet_window: 300 },
  alert: { firing_delay: 1000, predicate: 'event.value > 80', recovery_delay: 3000 },
  tap: null,
  input: { label: 'Input' },
  output: { label: 'Output' },
  replicator: { at: ['project'], ttl: 60000, capacity: 8 },
  agent: { harness: 'local-research', result: 'bytes', queue_capacity: 16, tools: [], approval: 'none' },
  counter: null,
  ema: { half_life: 10, time_basis: 'samples' },
  windowed_reduce: { window_length: 10000, emission_period: 1000, seed: 0, reduce: 'acc + event' },
  timer: { every: 1000 },
  tool_executor: {
    tools: {
      read_file: { effect: 'file_read', path: './sources/context.md' },
      write_file: { effect: 'file_write', mode: 'replace', path: './notes/research.md', approval: 'required' },
    },
    capabilities: {},
  },
  notify: { channel: 'local', minimum_interval: 5000, during_interval: 'latest' },
  peer: { name: 'research-peer', realm: 'local', adapter: 'codex', inbox_capacity: 32, inbound_policy: { any_known_peer: true } },
  listener: { source: { kind: 'file_tail', value: { glob: './logs/*.log', poll: 1000 } }, capabilities: {} },
  keyed_reduce: { at: ['project'], value: ['cost'] },
  request: { method: 'get', url: 'https://example.invalid/notes', headers: [], capabilities: {} },
  file: { path: './notes/untitled.md', capabilities: {} },
  json: { initial: { topic: 'Local research', enabled: true } },
  otlp: { listen: '127.0.0.1:4318' },
  match: null,
  assemble: { at: ['request'], capacity: 12, inactivity_timeout: 1500, max_window: 10000 },
  join: { at: ['project'] },
  form: { fields: [1, [4, [{ name: 'question', shape: [2, 'string'] }, { name: 'include_sources', shape: [2, 'bool'] }], false]] },
};
const info = {
  route: ['GitBranch', 'Routing'], pipeline_actor: ['Layers', 'Structure'], debounce: ['Timer', 'Timing'],
  alert: ['CircleAlert', 'Signals'], tap: ['Radio', 'Observe'], input: ['ArrowDownLeft', 'Inputs'],
  output: ['ArrowUpRight', 'Outputs'], replicator: ['Layers', 'Structure'], agent: ['Sparkles', 'Agents'],
  counter: ['Activity', 'Measures'], ema: ['Activity', 'Measures'], windowed_reduce: ['Activity', 'Measures'],
  timer: ['Timer', 'Timing'], tool_executor: ['Terminal', 'Actions'], notify: ['MessageSquare', 'Actions'],
  peer: ['Network', 'Connections'], listener: ['Radio', 'Inputs'], keyed_reduce: ['List', 'Measures'],
  request: ['ArrowUpRight', 'Actions'], file: ['FileText', 'Files'], json: ['Braces', 'Inputs'],
  otlp: ['Activity', 'Inputs'], match: ['GitBranch', 'Routing'], assemble: ['Layers', 'Data'],
  join: ['List', 'Data'], form: ['SlidersHorizontal', 'Inputs'],
};
const shapes = {
  route: { event: 'T', unmatched: 'T' },
  debounce: { event: 'T' },
  alert: { event: 'T', transition: 'Object' },
  tap: { event: 'T' },
  agent: { turn: 'Any', tool_result: 'Object', record: 'Object', tool_request: 'Object', result: 'Bytes' },
  counter: { event: 'Any', count: 'Int' },
  ema: { sample: 'Float', ema: 'Object' },
  windowed_reduce: { sample: 'Float', aggregate: 'Inferred' },
  timer: { bang: 'Any', tick: 'Object' },
  tool_executor: { call: 'Object', result: 'Object' },
  notify: { notification: 'Any' },
  peer: { send: 'Object', refresh: 'Any', message: 'Object', peers: 'Object', delivery: 'Object', binding: 'Object' },
  listener: { control: 'Control', line: 'Object' },
  keyed_reduce: { event: 'Object', remove: 'Object', map: 'Object', total: 'Float', count: 'Int' },
  request: { event: 'Any', response: 'Object' },
  file: { write: 'Any', read: 'Any', content: 'Bytes', written: 'Int' },
  json: { set: 'Any', bang: 'Any', value: 'Any' },
  otlp: { logs: 'Object', metrics: 'Object' },
  match: { event: 'T', ok: 'T', err: 'Reason' },
  assemble: { event: 'Object' },
  join: { event: 'Any', state: 'Any', remove: 'Any' },
};
const optionValues = {
  approval: ['none', 'required'],
  result: ['bytes', 'json'],
  time_basis: ['samples', 'wallclock'],
  during_interval: ['latest', 'queue', 'suppress'],
};
const labels = {
  at: 'Payload key path', value: 'Value path', queue_capacity: 'Mailbox capacity', quiet_window: 'Quiet window · ms',
  half_life: 'Half-life', time_basis: 'Time basis', emission_period: 'Emit every · ms', window_length: 'Window length · ms',
  every: 'Interval · ms', ttl: 'Idle lifetime · ms', capacity: 'Capacity', inactivity_timeout: 'Close after idle · ms',
  max_window: 'Maximum window · ms', minimum_interval: 'Minimum interval · ms', during_interval: 'While waiting',
  inbox_capacity: 'Peer inbox capacity', initial: 'Initial value', fields: 'Input fields', over: 'Template scope',
  harness: 'Local harness', result: 'Result format', tools: 'Tools', capabilities: 'Declared capability overrides',
  approval: 'Require approval', cases: 'Route cases', predicate: 'Condition', reduce: 'Reducer', path: 'File path',
  source: 'Source', url: 'URL', method: 'Method', label: 'Label', listen: 'Listen address',
};
const steps = {
  flatten: { expression: '[]', cue: 'object array' },
  map: { expression: 'event * 2', cue: 'identity' },
  filter: { expression: 'event > 3', cue: 'non-null' },
  parse: { expression: 'json', cue: 'JSON' },
  bang: { expression: 'null', cue: 'trigger' },
};

const sampleOutputs = {
  actors: [
    { address: { scope: [], local: 'notebook' }, title: 'Research notes', type: 'file', view: 'notebook',
      config: { path: './notes/research.md' }, port: 'content',
      body: ['Small systems, useful context.',
        'Keep the research loop open. A local source can update the agent’s context without restarting the rest of the program.',
        'Three useful observations from the current sources:', '1. Ownership remains close to each actor.',
        '2. Quiet time is a normal part of an asynchronous program.', '3. A notebook makes the result available beyond the canvas.'].join('\n') },
    { address: { scope: [{ name: 'sources' }], local: 'source-file' }, title: 'Project context', type: 'file', view: 'table',
      config: { path: './sources/context.md' }, port: 'content', label: 'Sources',
      viewConfig: { columns: [{ label: 'Source', path: ['source'] }, { label: 'What it contributes', path: ['contributes'] },
        { label: 'Updated', path: ['updated'] }] },
      body: [{ source: 'Project context', contributes: 'Purpose and working constraints', updated: 'Just now' },
        { source: 'Research notes', contributes: 'Open questions and findings', updated: '2 min ago' },
        { source: 'Reference index', contributes: 'Links used by the research desk', updated: '5 min ago' }] },
    { address: { scope: [], local: 'prompt' }, title: 'A place to begin', type: 'input', view: 'prompt', port: 'event' },
  ],
  mounts: [['working-notebook', 'result', 'notebook'], ['source-library', 'result', 'source-file'],
    ['ask-the-research-desk', 'request', 'prompt']],
};

function catalogRows(published, viewOf) {
  return published.map(row => ({
    ...row,
    type: row.actor_type,
    title: row.label,
    icon: info[row.actor_type][0],
    group: info[row.actor_type][1],
    get view() { return viewOf(row.actor_type); },
    config: configs[row.actor_type],
  }));
}

export function fixtureSource(win = globalThis.window) {
  const STUDY = win.STUDY;
  for (const [id, text] of Object.entries(win.STUDY_MESSAGES ?? {})) viewer.message(id, text);
  let rows;
  const byType = () => new Map(source.catalog().map(row => [row.actor_type, row]));
  let canvas = {};
  let metrics;
  const app = () => win.StudyApp;
  const cardBox = n => ({ x: n.x, y: n.y, w: n.width, h: app().nodeHeight(n) });
  const sampleScene = (() => {
    const actorOf = local => sampleOutputs.actors.find(actor => actor.address.local === local);
    const nodes = sampleOutputs.actors.map(actor => ({ id: key(actor.address), address: actor.address,
      title: actor.title, type: actor.type, viewKind: { kind: actor.view, code: null },
      ...(actor.viewConfig ? { viewConfig: actor.viewConfig } : {}), declaration: { config: actor.config ?? null },
      in: [], out: [[actor.port, undefined, 0, actor.label ?? actor.port]], arrivals: [],
      emitted: actor.body === undefined ? []
        : [{ kind: 'actor_emission', actor: actor.address, port: actor.port, body: actor.body, observed_at_ms: 1200n }] }));
    const exportMounts = sampleOutputs.mounts.map(([local, role, actor]) => ({ address: { scope: [], local },
      declaration: { roles: { [role]: { actor: actorOf(actor).address, port: actorOf(actor).port } } } }));
    const ids = new Map([...nodes.map(node => [node.id, node.address.local]),
      ...exportMounts.map(mount => [key(mount.address), mount.address.local])]);
    return { graph: { nodes, edges: [], exportMounts, anchor: { scope: [] }, snapshotPage: { terminal: 'Complete' } },
      domId: id => ids.get(id) };
  })();

  let history = [], historyAt = -1, checkpointQueued = false, restoring = false;
  const snapshot = () => clone({
    scopes: app().liveScopes(), selected: app().state.selected,
    selectedSet: [...app().state.selectedSet], scope: app().state.scope,
  });
  function checkpoint() {
    const A = app();
    if (restoring || A.historical()) return;
    const frame = snapshot(), last = history[historyAt];
    if (last && JSON.stringify(last.scopes) === JSON.stringify(frame.scopes)) return;
    history.splice(historyAt + 1);
    history.push(frame);
    historyAt = history.length - 1;
    win.Product.refreshEditTools();
  }
  function queueCheckpoint() {
    if (checkpointQueued) return;
    checkpointQueued = true;
    queueMicrotask(() => { checkpointQueued = false; checkpoint(); });
  }
  function restore(at) {
    const A = app(), S = A.state;
    if (A.historical() || at < 0 || at >= history.length) return;
    restoring = true;
    const f = clone(history[at]);
    for (const k of Object.keys(A.liveScopes())) delete STUDY[k];
    Object.assign(STUDY, f.scopes);
    if (S.scope !== f.scope && S.cameras.has(f.scope)) {
      Object.assign(S, S.cameras.get(f.scope));
      A.transformWorld();
    }
    S.scope = f.scope;
    S.selected = f.selected;
    S.selectedSet = new Set(f.selectedSet);
    S.edge = null;
    historyAt = at;
    viewer.keep(id => Boolean(A.allNode(id)));
    A.renderGraph();
    win.StudyFixture?.capture(A.archive, A.timeMachine.head, A.liveScopes());
    restoring = false;
    win.Product.refreshEditTools();
  }

  function record(actor, event, detail, value) {
    const A = app();
    const r = A.archive.append({ actor, event, detail, value, at: A.timeMachine.head });
    requestAnimationFrame(() => win.StudyFixture?.capture(A.archive, A.timeMachine.head, A.liveScopes()));
    A.renderJournal();
    queueCheckpoint();
    return r;
  }
  function commit(label, fn) {
    const A = app();
    if (A.historical()) return A.gestureCode('EDIT_UNAVAILABLE');
    fn();
    viewer.keep(id => Boolean(A.allNode(id)));
    record(A.state.selected || 'scope', 'Presentation', label, {});
    A.renderGraph();
    checkpoint();
  }
  const live = () => !app().historical();
  const captureNow = () => { const A = app(); if (!A.historical()) win.StudyFixture?.capture(A.archive, A.timeMachine.head, A.liveScopes()); };
  const banner = (...args) => win.Product.banner(...args);

  function portsFor(node, config = node.config) {
    const type = node.type, spec = byType().get(type);
    if (!spec) return { in: node.in || [], out: node.out || [] };
    const make = rows => rows.map((p, i) => [p.id, p.id === '_error' ? 'Reason' : shapes[type]?.[p.id] || 'Any', 92 + i * 25, p.id]);
    let input = make(spec.in_ports), output = make(spec.out_ports);
    if (type === 'input')
      output = [[node.boundaryPort || 'event', 'Any', node.view === 'prompt' ? 176 : 92, config.label || 'Input']];
    if (type === 'output') input = [[node.boundaryPort || 'event', 'Any', 92, config.label || 'Output']];
    if (type === 'pipeline_actor') return { in: node.in || [], out: node.out || [] };
    if (type === 'replicator')
      output = (node.interfaceOut || []).map((id, i) => [id, 'T', 92 + i * 25, node.out?.find(port => port[0] === id)?.[3] || id]);
    if (type === 'route')
      output = [
        ...Object.keys(config.cases || {}).map((key, i) => ['route_' + key, 'T', 92 + i * 25, key]),
        ['unmatched', 'T', 92 + Object.keys(config.cases || {}).length * 25, 'unmatched'],
      ];
    if (type === 'windowed_reduce' && config.reduce) output = [['aggregate', 'Inferred', 92, 'aggregate']];
    if (type === 'agent')
      output = output.map(p => p[0] === 'result' ? [p[0], config.result === 'bytes' ? 'Bytes' : 'Inferred', p[2], p[3]] : p);
    if (type === 'join') output = output.map(p => p[0] === 'event' ? [p[0], 'Object', p[2], p[3]] : p);
    if (type === 'form') {
      const fields = config.fields?.[1];
      const shape = fields?.[0] === 4 ? 'Object'
        : fields?.[0] === 2 ? { string: 'String', float: 'Float', int: 'Int', bool: 'Bool' }[fields[1]] || 'Any' : 'Any';
      output = [['event', shape, 92, 'form value']];
    }
    return { in: input, out: output };
  }
  function make(type, id, x = 150, y = 150) {
    const s = byType().get(type), config = clone(s.config);
    const n = {
      id, type, title: s.title, icon: s.icon, view: s.view, viewer: s.view === 'agent' ? 'task' : s.view,
      x, y, width: 244, height: 260, health: 'running', activity: 'Idle', config, preview: {},
      flags: { pause: false, mute: false, bypass: false },
    };
    Object.assign(n, portsFor(n));
    if (['pipeline_actor', 'replicator'].includes(type)) n.scope = id;
    if (type === 'agent') { n.height = 310; n.preview = { text: 'Ready for your next question.' }; }
    if (type === 'json') n.preview = { value: config.initial };
    if (type === 'input') n.height = 240;
    if (type === 'file') n.preview = { heading: 'No content yet', lines: ['Write or read to observe this file.'], bytes: 0 };
    if (type === 'tool_executor') n.preview = { rows: Object.keys(config.tools).map(k => [k, 'idle']), bytes: 0 };
    if (type === 'tap') n.preview = { rows: [['—', 'Waiting for arrivals']], kinds: ['arrival'] };
    if (type === 'counter') n.preview = { value: 0, label: 'accepted arrivals' };
    if (type === 'ema') n.preview = { value: 0, label: 'weighted mean', samples: 0 };
    if (type === 'timer') n.preview = { value: 0, label: 'ticks emitted' };
    n.height = Math.max(n.height, 128 + Math.max(n.in.length, n.out.length) * 25);
    return n;
  }
  function configForm(n, draft = n.config) {
    const type = n.type, sampleConfig = configs[type] || {}, keys = Object.keys(sampleConfig);
    if (!keys.length) return '<p class="subtle-note">This actor has no runtime configuration.</p>';
    return keys
      .filter(k => k !== 'capabilities')
      .map(k => {
        const value = draft?.[k], label = labels[k] || k;
        const undeclared = value == null, valueType = typeof (value ?? sampleConfig[k]);
        const attrs = `name="${k}" data-config-field="${k}" aria-label="${label}"${undeclared ? ` data-reason="UNDECLARED" title="${escape(reasonText('UNDECLARED'))}"` : ''}`;
        const emptyChoice = undeclared ? `<option value="" selected>${escape(reasonText('UNDECLARED'))}</option>` : '';
        let control;
        if (k === 'approval')
          control = `<select ${attrs}>${emptyChoice}${optionValues[k].map(v => `<option value="${v}" ${value === v ? 'selected' : ''}>${v === 'none' ? 'No approval required' : 'Approval required'}</option>`).join('')}</select>`;
        else if (optionValues[k])
          control = `<select ${attrs}>${emptyChoice}${optionValues[k].map(v => `<option ${value === v ? 'selected' : ''}>${v}</option>`).join('')}</select>`;
        else if (k === 'fields')
          control = `<div class="form-field-editor" data-fields-editor>${(value?.[1]?.[0] === 4 ? value[1][1] : []).map((f, i) => `<div><input data-field-name="${i}" aria-label="Field ${i + 1} name" value="${escape(f.name)}"><select data-field-shape="${i}" aria-label="Field ${i + 1} type">${['string', 'bool', 'int', 'float'].map(t => `<option ${f.shape?.[1] === t ? 'selected' : ''}>${t}</option>`).join('')}</select><button type="button" data-remove-field="${i}" aria-label="Remove field ${i + 1}">×</button></div>`).join('')}<button type="button" class="quiet-button" data-add-field>+ Add field</button></div>`;
        else if (valueType === 'object' || k === 'seed')
          control = `<textarea ${attrs} data-json="true" rows="${k === 'tools' ? 6 : 3}" spellcheck="false">${undeclared ? '' : escape(JSON.stringify(value, null, 2))}</textarea>`;
        else if (valueType === 'number')
          control = `<input ${attrs} type="number" step="1" value="${value ?? ''}" required>`;
        else control = `<input ${attrs} value="${escape(value)}" required>`;
        const hint = k === 'queue_capacity' ? 'Actor mailbox; separate from receiving inlet capacity.'
          : k === 'tools' && type === 'tool_executor' ? 'Approval is configured per tool, never for the executor as a whole.'
            : k === 'fields' ? 'These fields also determine the type of the form output.'
              : k === 'over' ? 'A declared template scope, not an actor to run.' : '';
        return `<label class="config-field"><span class="config-label">${escape(label)}</span>${control}${undeclared ? `<small data-reason="UNDECLARED" title="${escape(reasonText('UNDECLARED'))}">${escape(reasonText('UNDECLARED'))}</small>` : ''}${hint ? `<small>${hint}</small>` : ''}</label>`;
      })
      .join('');
  }
  function readDraft(form, n) {
    const next = clone(n.config);
    for (const input of form.querySelectorAll('[name]')) {
      if (input.type === 'hidden') continue;
      next[input.name] = input.dataset.json ? JSON.parse(input.value) : input.type === 'number' ? Number(input.value) : input.value;
    }
    const fieldEditor = form.querySelector('[data-fields-editor]');
    if (fieldEditor)
      next.fields = [1, [4, [...fieldEditor.querySelectorAll('[data-field-name]')].map(el => ({
        name: el.value,
        shape: [2, fieldEditor.querySelector(`[data-field-shape="${el.dataset.fieldName}"]`).value],
      })), false]];
    return next;
  }
  function validate(n, c) {
    for (const [key, value] of Object.entries(c))
      if (key !== 'seed' && typeof configs[n.type]?.[key] === 'number'
        && (!Number.isInteger(value) || value < (['quiet_window', 'window_length', 'minimum_interval'].includes(key) ? 0 : 1)))
        return {
          code: 'ConfigRejected',
          message: `${labels[key] || key} must be ${['quiet_window', 'window_length', 'minimum_interval'].includes(key) ? 'non-negative' : 'positive'}.`,
          field: key,
        };
    if (n.type === 'route' && new Set(Object.values(c.cases).map(v => JSON.stringify(v))).size !== Object.keys(c.cases).length)
      return { code: 'ConfigRejected', message: 'Each route case needs a distinct value.', field: 'cases' };
    if (n.type === 'form' && !c.fields?.[1]?.[1]?.length) return null;
    if (n.type === 'file' && !c.path.trim()) return { code: 'ConfigRejected', message: 'Enter a file path.', field: 'path' };
    return null;
  }

  function initializeActor(n) {
    if (n.type !== 'replicator') return;
    const S = app().state;
    const input = make('input', n.id + '-input', 65, 150), tap = make('tap', n.id + '-tap', 395, 150),
      output = make('output', n.id + '-output', 725, 150);
    input.title = 'Instance input';
    input.config.label = 'event';
    input.boundaryPort = n.id + '-input-port';
    Object.assign(input, portsFor(input));
    output.title = 'Instance result';
    output.config.label = 'Result';
    output.boundaryPort = n.id + '-result-port';
    Object.assign(output, portsFor(output));
    n.interfaceOut = [output.boundaryPort];
    Object.assign(n, portsFor(n));
    n.out[0][3] = 'Result';
    n.preview.disposition = 'Pass-through template · waiting for keyed arrivals';
    STUDY[n.scope] = {
      name: n.title + ' template', parent: S.scope, template: true, nodes: [input, tap, output],
      edges: [
        { id: n.id + '-relay-in', from: input.id, out: input.out[0][0], to: tap.id, in: 'event', combinators: [], demoRate: 0 },
        { id: n.id + '-relay-out', from: tap.id, out: 'event', to: output.id, in: output.in[0][0], combinators: [], demoRate: 0 },
      ],
      notes: [],
    };
  }
  async function promptSent(actor, text) {
    const A = app(), S = A.state, scope = S.scope;
    if (!actor) return;
    actor.preview.task = text;
    actor.preview.text = 'Following this question through the available context.';
    actor.activity = 'Working';
    const update = () => {
      if (S.scope !== scope || A.historical()) return;
      A.renderActors([actor.id]);
    };
    update();
    await wait(850);
    actor.preview.result = 'The question is recorded. A simulated response is now available in this actor.';
    actor.activity = 'Idle';
    update();
    record(actor.id, 'actor_arrival', 'Simulated response observed', { question: text, response: actor.preview.result });
  }
  function countApprovals() {
    const P = win.Product;
    for (const scope of Object.values(app().liveScopes()))
      for (const n of scope.nodes)
        n.approvalCount = P.approvals.filter(r => r.actor === n.id && ['requested', 'submitting', 'failed'].includes(r.state)).length;
  }
  function refreshApprovalBadge() {
    const count = win.Product.approvals.filter(r => ['requested', 'submitting', 'failed'].includes(r.state)).length;
    document.querySelector('#approval-count').textContent = count;
    document.querySelector('#approval-count').hidden = false;
  }
  function newProject() {
    canvas.dialog('New local project',
      `<p>No cloud account or API key is needed to make a local canvas.</p><form data-new-project><label>Name<input name="name" placeholder="A name for your program" required autofocus></label><label>Description<input name="description" placeholder="What will it help you do?"></label><div class="dialog-actions"><button class="dark-button">Create project</button></div></form>`);
  }
  function connectionDialog(openId) {
    const P = win.Product, connected = P.connection === 'connected';
    canvas.dialog('Local connection',
      `<div class="connection-detail">${canvas.notice(connected ? 'Your local workspace is connected.' : 'Observation is disconnected. Last values remain visible; they are not live.', connected ? 'accepted' : 'pending')}<button class="dark-button" data-reconnect="${openId || ''}">${connected ? 'Check connection' : 'Reconnect locally'}</button><details style="margin-top:22px"><summary>Harnesses</summary><p>Choose an installed local harness when configuring an agent. Credentials remain with that harness.</p><div class="detail-row"><span>Local research</span><span>Available · simulated</span></div><button class="quiet-button" data-setup-harness>Set up a local harness…</button></details></div>`);
  }
  async function openProject(id) {
    const P = win.Product, A = app(), p = P.projects.find(p => p.id === id);
    if (!p) return;
    if (P.connection !== 'connected') return connectionDialog(id);
    P.project = id;
    document.querySelector('#projects-view').innerHTML = canvas.projectHeader()
      + `<div class="product-empty"><span class="loading-orbit"></span><h2>Opening ${escape(p.name)}</h2><p>Connecting to the local program…</p></div>`;
    await wait(450);
    A.setScope(p.scope);
    document.querySelector('.project-path').innerHTML = `<span class="project-mark">${escape(p.name[0])}</span><span>${escape(p.name)}</span>`;
    canvas.show('canvas');
  }
  async function reconnect(openId) {
    const P = win.Product, A = app(), box = document.querySelector('.connection-detail');
    if (box) box.innerHTML = canvas.notice('Reconnecting to your local workspace…');
    await wait(800);
    P.connection = 'connected';
    document.querySelector('#connection-strip').hidden = true;
    A.field.wake();
    A.renderGraph();
    if (box)
      box.innerHTML = canvas.notice('Connection restored. Observations are live again.', 'accepted')
        + '<button class="quiet-button" data-product-close>Continue</button>';
    if (openId) {
      document.querySelector('#product-dialog').close();
      canvas.show('projects');
      openProject(openId);
    }
    canvas.renderProjects();
    if (P.screen === 'outputs') canvas.renderOutputs();
  }
  function bindDialogs() {
    document.addEventListener('click', ev => {
      const b = ev.target.closest('button');
      if (!b) return;
      if (b.hasAttribute('data-reconnect')) return reconnect(b.dataset.reconnect);
      if (b.hasAttribute('data-setup-harness'))
        return canvas.dialog('Set up a local harness',
          '<p>Use an installed harness on this machine. This prototype simulates discovery and keeps credentials with the harness.</p><form data-harness-setup><label>Installed harness<select name="harness"><option>Local research</option></select></label><div class="dialog-actions"><button class="dark-button">Connect installed harness</button></div></form>');
      if (b.hasAttribute('data-add-field') || b.hasAttribute('data-remove-field')) {
        const A = app(), S = A.state, n = A.selectedNode(), form = b.closest('form');
        win.Product.captureDraft(form, n);
        const draft = clone(viewer.card(n.id).draft?.shown || n.config);
        draft.fields ??= [1, [4, [], false]];
        const fields = draft.fields[1][1];
        if (b.hasAttribute('data-add-field')) fields.push({ name: 'field_' + (fields.length + 1), shape: [2, 'string'] });
        else fields.splice(Number(b.dataset.removeField), 1);
        viewer.draft(n.id, { shown: draft });
        A.renderActors([n.id]);
      }
    }, true);
    document.addEventListener('submit', ev => {
      const form = ev.target;
      if (form.hasAttribute('data-new-project')) {
        ev.preventDefault();
        const S = app().state, id = 'project-' + ++S.generation, name = form.elements.name.value.trim();
        STUDY[id] = { name, nodes: [], edges: [], notes: [] };
        win.Product.projects.push({ id, name, description: form.elements.description.value, scope: id });
        document.querySelector('#product-dialog').close();
        canvas.show('projects');
      }
      if (form.hasAttribute('data-harness-setup')) {
        ev.preventDefault();
        canvas.dialog('Harness connected',
          canvas.notice('Local research is available for agent configuration.', 'accepted') + '<button class="quiet-button" data-product-close>Done</button>');
      }
    });
  }

  for (const g of Object.values(STUDY).filter(g => g.nodes))
    for (const [i, e] of g.edges.entries()) {
      e.combinators = e.label
        ? [{ id: e.id + '-0', kind: e.label, expression: e.map || steps[e.label]?.expression || '', cue: steps[e.label]?.cue || e.label, x: e.labelX || 350, y: e.labelY || 330 }]
        : [];
      e.demoRate = [1.4, 2.6, 3.2, 0.9, 1.7, 6.4][i % 6];
    }

  let anchor = 0;
  const capture = (machine, t) => win.StudyFixture?.capture(machine.archive, t, machine.options.scopes());
  const transport = {
    attach(machine) { win.StudyFixture?.start(machine.archive, machine, machine.options.scopes()); },
    position(m) {
      return m.mode === 'live' ? m.head
        : m.mode === 'replay' ? Math.min(m.horizon, m.at + ((win.performance.now() - anchor) / 1000) * m.speed) : m.at;
    },
    seek(m, t) {
      if (m.mode === 'live') capture(m, m.head);
      m.show({ mode: 'history', at: Math.max(0, Math.min(t, m.head)) });
    },
    resume(m) {
      if (m.mode === 'live') { m.ended = false; return; }
      if (m.mode === 'replay') return m.show({ mode: 'history', at: m.position });
      anchor = win.performance.now();
      m.show({ mode: 'replay', horizon: m.head });
    },
    live: m => m.show({ mode: 'live' }),
    speedChanged(m) {
      anchor = win.performance.now();
      m.refresh();
    },
  };

  const source = {
    get projection() { return STUDY; },
    outletReading,
    product: {
      projects: [{ id: 'fieldnotes', name: 'Fieldnotes', description: 'A research desk that reads, connects ideas, and keeps a working notebook.', scope: 'root' }],
      project: 'fieldnotes',
      approvals: [],
    },
    catalog() {
      return rows ??= catalogRows(win.PUBLISHED_ACTORS, type => win.LiveViewers.select({ type }).kind);
    },
    choices: n => win.LiveViewers.choices(n, Object.keys(byType().get(n.type)?.config ?? {})),
    stepKinds: () => ({ kinds: Object.keys(steps) }),
    transport,
    get head() { return win.StudyFixture?.head ?? win.STUDY_HISTORY.duration; },
    rate(e, t) {
      if (win.StudyFixture) return win.StudyFixture.rate(e, t);
      const rate = e.demoRate ?? 2;
      return rate * (0.91 + 0.09 * Math.sin(t * 1.2 + e.from.length));
    },
    rateSeconds: WIRE_RATE_SECONDS,
    edgeValue(e) {
      const r = app().visibleRecords().find(r => r.actor === e.to && r.event === 'actor_arrival');
      const v = r?.value ?? (e.out === 'context' ? 'Project context, 2.1 kB' : { kind: 'message', text: 'Reading project context' });
      return typeof v === 'string' ? v : JSON.stringify(v);
    },
    observation(n, t, rate) {
      const seed = n.id.length * 0.8;
      const value = rate * (0.54 + 0.32 * Math.sin(t * 1.9 + seed) ** 2 + 0.2 * Math.sin(t * 4.1 + seed) ** 2);
      const capacity = declaredCapacity(n);
      const sample = 1 + Math.floor((0.5 + 0.5 * Math.sin(t * 0.72 + seed)) * 3);
      const queue = n.preview ? capacity == null ? sample : Math.min(capacity, sample) : 0;
      return { value, queue, activeTool: Math.floor(t / 2) % 3 };
    },
    paintsOnly: true,
    observing: () => !win.Product || win.Product.connection === 'connected',
    scopeHealth(scope) {
      const paused = app().state.paused.has(scope);
      return { state: paused ? 'paused' : 'live', text: paused ? 'Paused' : 'Live' };
    },
    updateStatusbar() {
      const A = app();
      return import('./renderer/statusbar.mjs').then(({ updateStatusbar }) => {
        const failed = A.graph().nodes.filter(n => n.health === 'failed').length;
        const paused = A.state.paused.has(A.state.scope);
        const text = A.historical()
          ? 'Recorded state · ' + win.studyTimeFormat(A.displayTime())
          : win.Product && win.Product.connection !== 'connected'
            ? 'Disconnected · last observations'
            : paused ? 'Scope paused'
              : `${A.graph().nodes.length - failed} ${A.graph().nodes.length - failed === 1 ? 'actor' : 'actors'} alive${failed ? ` · ${failed} failed` : ''}`;
        updateStatusbar(document, text, undefined, A.historical() ? 'history'
          : win.Product && win.Product.connection !== 'connected' ? 'unobserved' : paused ? 'paused' : failed ? 'dead' : 'alive');
      });
    },
    refuse: refusal => app().toast(refusal.code),
    notice: code => app().toast(code),
    codeLabel: code => ({ code, label: reasonText(code) }),
    editHistory: () => ({ undo: historyAt > 0, redo: historyAt < history.length - 1 }),
    outputs() {
      const P = win.Product, project = P?.projects?.find(p => p.id === P.project);
      if (project?.scope !== 'root') return { surfaces: [], count: 0, terminal: 'Complete', code: null };
      return outputsFromScene(sampleScene.graph, sampleScene.domId, null, null);
    },
    renderSurface: (surface, page) => renderSurface(surface, { ...page, viewer }),
    updateSurface,
    bindSurfaces() {},
    configForm,
    readDraft,
    permissions(n) {
      const c = n.config || {}, tools = c.tools && !Array.isArray(c.tools) ? Object.entries(c.tools) : null,
        approval = Object.hasOwn(c, 'approval'),
        grants = !tools && !approval && c.capabilities ? Object.entries(c.capabilities).filter(([, g]) => g && Object.hasOwn(g, 'approval')) : [];
      return {
        note: `Observed decisions are read-only. ${tools ? 'Approval requirements belong to each tool.' : approval || grants.length ? 'Approval requirements are editable in Configure.' : 'This actor has no configurable approval requirement.'}`,
        rows: tools ? tools.map(([name, t]) => [name, t.approval || 'none']) : approval ? [['Configured approval', c.approval || 'none']]
          : grants.map(([name, g]) => [name, g.approval]),
        empty: { text: 'No approval decisions observed for this actor.' },
      };
    },
    sdkText() { return ''; },
    sdkProgram() { return { text: '', spans: [], diagnostics: [{ code: 'FIXTURE_HAS_NO_AUTHORING_FOLD', message: 'Connect to a project to read its SDK program.' }] }; },
    mount({ metrics: measured } = {}) { metrics = measured; },
    get metrics() { return metrics ?? UNMEASURED; },
    landing(type, at) {
      const A = app(), sample = make(type, type, at.x, at.y);
      return place([{ x: at.x, y: at.y, w: sample.width, h: sample.height }], A.graph().nodes.map(cardBox))[0];
    },
    hold(kind, id, value) {
      const shown = app().allNode(id);
      if (shown) Object.assign(shown, value);
      return shown;
    },
    openState: openProject,
    actions: { 'new-project': newProject, connection: connectionDialog, harnesses: connectionDialog },
    evidenceTarget: r => ['event-at', r.at],
    actorRecords: id => app().visibleRecords().filter(r => r.actor === id),
    rawObservation(id) {
      const n = app().findNode(id);
      return n && { id: n.id, health: n.health, activity: n.activity, preview: n.preview };
    },
    selectionRecords: () => undefined,
    recordOf: () => undefined,
    endedText: () => undefined,
    journalLine: past => ({ label: past ? 'At selected time' : 'Following arrivals', title: '' }),
    settingsDeclared: () => true,
    ...mockupInspect,
    healthBanner: (n, past) => mockupInspect.healthBanner(n, past, past ? win.studyTimeFormat(app().displayTime()) : null),
    afterGraph() {
      const connection = document.querySelector('#connection-strip > span');
      if (connection) connection.textContent = 'Disconnected · showing last observations';
    },
    afterPalette() {},
    afterProjects() {},
    face(n) {
      const A = app();
      return mockupFace(n, !A?.historical?.() && Boolean(A?.state?.paused?.has(A.state.scope)));
    },
    afterInspector() {},
    afterEdgeInspector() {},
    updateViewer() {},
    refreshApprovalBadge,
    attach(handles) {
      canvas = handles;
      bindDialogs();
      const { A, C } = handles;
      for (const g of Object.values(A.liveScopes()))
        for (const n of g.nodes) {
          n.flags ??= {};
          if (n.view === 'agent') n.viewer = 'task';
          if (n.type === 'tool_executor') n.config = clone(C.byType.get('tool_executor').config);
          const ports = portsFor(n);
          if (n.type !== 'pipeline_actor')
            for (const side of ['in', 'out'])
              for (const p of ports[side])
                if (!n[side].some(old => old[0] === p[0])) n[side].push(p);
        }
      win.StudyPrototype?.install({ ...handles, appendRecord: record, initializeActor, make,
        refreshApprovalBadge, countApprovals, resetHistory() { history = []; historyAt = -1; checkpoint(); } });
      checkpoint();
      captureNow();
    },
    perform: performer({
      createActor({ type, at, connectFrom, preprocess = [] }) {
        const A = app(), S = A.state;
        const sample = source.catalog().find(x => x.type === type);
        if (!sample) return;
        const id = type + '-' + ++S.generation;
        const put = at && source.landing(type, at);
        const n = make(type, id, put?.x ?? 235, put?.y ?? 150);
        if (n.scope) STUDY[n.scope] = { name: n.title, parent: S.scope, nodes: [], edges: [], notes: [] };
        initializeActor(n);
        A.graph().nodes.push(n);
        S.selected = id;
        S.selectedSet = new Set([id]);
        S.edge = null;
        S.component = null;
        S.tab = 'configure';
        record(id, 'UpsertActor', 'New actor · preview', sample.config);
        A.renderGraph();
        if (connectFrom) {
          const side = connectFrom.side === 'out' ? 'in' : 'out', port = n[side][0];
          if (port) {
            const made = { node: n.id, name: port[0] };
            source.perform({ kind: 'connect', outlet: side === 'in' ? connectFrom : made,
              inlet: side === 'in' ? made : connectFrom, preprocess });
            A.clearConnection();
          }
        }
        A.toast(`${sample.title} added to the canvas.`);
      },
      retireActors({ actors }) {
        if (!live()) return;
        const A = app(), S = A.state, ids = new Set(actors);
        const removeScope = scope => {
          for (const n of STUDY[scope]?.nodes || []) if (n.scope) removeScope(n.scope);
          delete STUDY[scope];
        };
        commit('Actors removed', () => {
          const g = A.graph();
          for (const n of g.nodes.filter(n => ids.has(n.id))) if (n.scope) removeScope(n.scope);
          g.nodes = g.nodes.filter(n => !ids.has(n.id));
          g.edges = g.edges.filter(w => !ids.has(w.from) && !ids.has(w.to));
          S.selected = null;
          S.selectedSet.clear();
          S.edge = null;
        });
      },
      connect({ outlet, inlet, preprocess = [] }) {
        const A = app(), S = A.state, P = win.Product, scope = S.scope;
        const edge = { id: 'preview-edge-' + ++S.generation, from: outlet.node, out: outlet.name, to: inlet.node, in: inlet.name,
          combinators: preprocess.map((c, i) => ({ ...c, id: 'pending-comb-' + ++S.generation,
            x: A.findNode(inlet.node).x - 80 - (preprocess.length - 1 - i) * 96,
            y: A.findNode(inlet.node).y + 100 })), demoRate: 2.8, pending: true };
        STUDY[scope].edges.push(edge);
        A.renderGraph();
        A.selectEdge(edge.id);
        captureNow();
        banner('Checking the unresolved port types…');
        const outcome = P.nextConnection;
        P.nextConnection = null;
        wait(700).then(() => {
          if (!STUDY[scope]?.edges.includes(edge)) return;
          if (outcome === 'reject') {
            STUDY[scope].edges = STUDY[scope].edges.filter(w => w !== edge);
            if (S.edge === edge.id) S.edge = null;
            banner('The inferred output does not match this inlet. The connection was rejected.', 'rejected', 'IncompatibleConnection');
          } else {
            edge.pending = false;
            banner('Connection accepted.', 'accepted');
            record(inlet.node, 'UpsertEdge', 'Connection accepted after validation', edge);
          }
          A.renderGraph();
          checkpoint();
        });
        return true;
      },
      retireEdge({ edge: id }) {
        const A = app(), S = A.state, g = A.graph(), edge = g.edges.find(w => w.id === id);
        g.edges = g.edges.filter(w => w.id !== id);
        record(edge.to, 'RetireEdge', 'Connection removed · preview', { edge: edge.id });
        A.toast('Connection removed from the preview.');
        return true;
      },
      moveActors: () => { checkpoint(); captureNow(); },
      resize: () => { checkpoint(); captureNow(); },
      alignTops({ actors }) {
        const A = app(), nodes = actors.map(A.findNode), top = Math.min(...nodes.map(n => n.y));
        commit('Aligned selected actors', () => nodes.forEach(n => (n.y = top)));
      },
      group({ actors, into: target, name }) {
        const A = app(), S = A.state, parent = A.graph(), ids = new Set(actors),
          nodes = parent.nodes.filter(n => ids.has(n.id));
        if (!nodes.length) return;
        if (nodes.some(n => ['input', 'output', 'pipeline_actor', 'replicator'].includes(n.type)))
          return canvas.dialog('These actors cannot be moved together',
            `<p>Boundary actors stay at their boundary. Containers cannot be moved inside another scope.</p>${canvas.notice('Choose ordinary actors for this move.', 'rejected', 'boundary_actor_immovable / would_nest_into_self')}`);
        let container = parent.nodes.find(n => n.id === target);
        if (!container) {
          const id = 'pipeline-' + ++S.generation;
          container = make('pipeline_actor', id, Math.min(...nodes.map(n => n.x)), Math.min(...nodes.map(n => n.y)));
          container.title = name || 'Working group';
          container.scope = id;
          parent.nodes.push(container);
          STUDY[id] = { name: container.title, parent: S.scope, nodes: [], edges: [], notes: [] };
        }
        const child = STUDY[container.scope];
        if (nodes.some(n => child.nodes.some(c => c.id === n.id)))
          return banner('A local name already exists in the destination.', 'rejected', 'local_collision_in_target');
        const minX = Math.min(...nodes.map(n => n.x)), minY = Math.min(...nodes.map(n => n.y));
        nodes.forEach(n => { n.x = n.x - minX + 260; n.y = n.y - minY + 100; child.nodes.push(n); });
        const boundary = new Map(), parentEdges = [];
        for (const w of parent.edges) {
          const f = ids.has(w.from), t = ids.has(w.to);
          if (f && t) { child.edges.push(w); continue; }
          if (!f && !t) { parentEdges.push(w); continue; }
          if ((f && w.to === container.id) || (t && w.from === container.id)) {
            const into = f, boundaryPort = into ? w.in : w.out;
            const boundaryActor = child.nodes.find(n => n.type === (into ? 'input' : 'output') && n[into ? 'out' : 'in'].some(p => p[0] === boundaryPort));
            const relay = child.edges.find(r => into ? r.from === boundaryActor?.id && r.out === boundaryPort : r.to === boundaryActor?.id && r.in === boundaryPort);
            if (relay) {
              child.edges = child.edges.filter(r => r !== relay);
              child.edges.push({ ...w, from: into ? w.from : relay.from, out: into ? w.out : relay.out, to: into ? relay.to : w.to, in: into ? relay.in : w.in });
              continue;
            }
          }
          const side = t ? 'in' : 'out', inner = t ? w.to : w.from, port = t ? w.in : w.out, key = `${side}:${inner}:${port}`;
          let b = boundary.get(key);
          if (!b) {
            const id = 'boundary-' + ++S.generation, kind = t ? 'input' : 'output', actor = child.nodes.find(n => n.id === inner),
              shape = actor[t ? 'in' : 'out'].find(p => p[0] === port)?.[1] || 'Any';
            b = make(kind, id, t ? 40 : Math.max(...nodes.map(n => n.x + n.width)) + 140, 100 + boundary.size * 260);
            b.title = `${actor.title} · ${port}`;
            b.config.label = b.title;
            b.boundaryPort = 'boundary-' + id;
            Object.assign(b, portsFor(b));
            b[t ? 'out' : 'in'][0][1] = shape;
            child.nodes.push(b);
            container[side].push([b.boundaryPort, shape, 92 + container[side].length * 25, b.title]);
            child.edges.push({ id: 'relay-' + ++S.generation, from: t ? id : inner, out: t ? b.boundaryPort : port,
              to: t ? inner : id, in: t ? port : b.boundaryPort, combinators: [], demoRate: w.demoRate });
            boundary.set(key, b);
          }
          parentEdges.push({ ...w, from: t ? w.from : container.id, out: t ? w.out : b.boundaryPort,
            to: t ? container.id : w.to, in: t ? b.boundaryPort : w.in });
        }
        parent.edges = parentEdges;
        parent.nodes = parent.nodes.filter(n => !ids.has(n.id));
        container.height = Math.max(260, 140 + Math.max(container.in.length, container.out.length) * 25);
        S.selected = container.id;
        S.selectedSet = new Set([container.id]);
        record(container.id, 'MoveToScope', `${nodes.length} actors moved into ${container.title}`, { actors: [...ids] });
        A.renderGraph();
        checkpoint();
        banner(`${nodes.length} actors moved into ${container.title}. Open the container to continue.`, 'accepted');
      },
      async configure({ actor, form }) {
        const A = app(), S = A.state, P = win.Product, n = A.findNode(actor);
        if (!live() || viewer.card(n.id).submission?.status === 'submitting') return;
        P.captureDraft(form, n);
        let draft, error;
        try {
          draft = readDraft(form, { ...n, config: viewer.card(n.id).draft?.shown || n.config });
          error = validate(n, draft);
        } catch {
          error = { code: 'ConfigRejected', message: 'A JSON field could not be read. Correct the draft and try again.' };
        }
        if (error) {
          viewer.change(n.id, { submission: { ...error, status: 'rejected' } });
          A.renderActors([n.id]);
          return;
        }
        viewer.draft(n.id, { shown: draft });
        viewer.change(n.id, { submission: { status: 'submitting', message: 'Submitting this actor’s configuration…' } });
        A.renderActors([n.id]);
        const applyScope = S.scope, outcome = P.nextApply;
        P.nextApply = null;
        await wait(650);
        if (outcome === 'reject') {
          viewer.change(n.id, { submission: { status: 'rejected', code: 'ConfigRejected',
            message: 'The selected harness is unavailable. Choose an installed harness and try again.' } });
          A.renderActors([n.id]);
          return;
        }
        n.config = clone(draft);
        Object.assign(n, portsFor(n));
        n.height = Math.max(A.nodeHeight(n), Math.max(n.in.length, n.out.length) * 25 + 135);
        for (const edge of STUDY[applyScope]?.edges || []) {
          if ((edge.from === n.id && !n.out.some(p => p[0] === edge.out)) || (edge.to === n.id && !n.in.some(p => p[0] === edge.in))) {
            edge.issue = { message: 'A connected port is no longer declared. Review this connection.', code: 'DestinationGone' };
            banner(`A port changed on ${n.title}. Affected connections need attention.`, 'pending');
          }
        }
        if (n.type === 'json') n.preview.value = clone(draft.initial);
        if (n.type === 'tool_executor') n.preview.rows = Object.keys(draft.tools).map(k => [k, 'idle']);
        viewer.change(n.id, { draft: undefined, submission: { status: 'accepted', message: 'Configuration accepted.' } });
        record(n.id, 'UpsertActor', 'Configuration accepted · simulation', draft);
        A.renderGraph();
        checkpoint();
      },
      setFlag({ actor, flag }) {
        const n = app().findNode(actor);
        commit('Actor flag changed', () => { n.flags ??= {}; n.flags[flag] = !n.flags[flag]; });
      },
      rename({ actor, label }) {
        commit('Actor renamed', () => { app().findNode(actor).title = label; });
        return true;
      },
      setView({ actor, view }) {
        const A = app(), n = A.findNode(actor);
        n.viewer = view;
        A.renderGraph();
      },
      addStep({ edge: id, step: kind, at }) {
        const A = app(), S = A.state, e = A.graph().edges.find(e => e.id === id);
        if (!e) return;
        const base = steps[kind];
        const put = draftChipPlace(at, A.graph().nodes.map(n => ({ ...n, height: A.nodeHeight(n) })), chipRoom(metrics ?? UNMEASURED), metrics ?? UNMEASURED);
        const c = { id: 'comb-' + ++S.generation, kind, expression: base.expression, cue: base.cue, x: put.x, y: put.y };
        e.combinators.push(c);
        S.component = c.id;
        A.renderWires();
        A.renderInspector();
        record(e.to, 'UpsertEdge', `${kind} added at ${e.in} · preview`, { kind });
        A.toast(`${kind} inserted on the wire.`);
      },
      applyStep({ step: c, config }) {
        const taken = live();
        commit('Inlet combinator configured', () => {
          c.config = config;
          c.expression = c.kind === 'parse' ? config.decoder : c.kind === 'flatten' ? JSON.stringify(config.at)
            : c.kind === 'bang' ? 'null' : config[c.kind === 'map' ? 'transform' : 'predicate'];
        });
        return taken;
      },
      moveStep({ edge: id, index: i, direction }) {
        const w = app().graph().edges.find(w => w.id === id), j = i + direction;
        if (j < 0 || j >= w.combinators.length) return;
        commit('Inlet processing reordered', () => {
          [w.combinators[i], w.combinators[j]] = [w.combinators[j], w.combinators[i]];
          const positions = w.combinators.map(c => ({ x: c.x, y: c.y })).sort((a, b) => a.x - b.x);
          w.combinators.forEach((c, k) => Object.assign(c, positions[k]));
        });
      },
      removeStep({ edge: id, index }) {
        const A = app(), e = A.graph().edges.find(w => w.id === id);
        e.combinators = e.combinators.filter((_, i) => i !== index);
        A.state.component = null;
        A.renderWires();
        A.renderInspector();
        queueMicrotask(checkpoint);
      },
      inletSettings({ edge: id, values }) {
        const w = app().graph().edges.find(w => w.id === id);
        commit('Receiving inlet settings updated', () => {
          w.declaredDelay = Number(values.delay);
          w.delivery = values.delivery;
          w.capacity = values.capacity ? Number(values.capacity) : undefined;
          if (w.pressure)
            w.issue = {
              message: w.delivery.startsWith('BestEffort')
                ? `Inlet full · ${w.delivery === 'BestEffortDropNewest' ? 'incoming values shed' : 'oldest values replaced'}.`
                : 'Inlet full · reliable delivery is waiting.',
              code: w.delivery.startsWith('BestEffort')
                ? w.delivery === 'BestEffortDropNewest' ? 'CapacityDecision::ShedIncoming' : 'CapacityDecision::ReplaceOldest'
                : 'CapacityDecision::BlockReliable',
            };
        });
      },
      createNote() {
        const A = app(), S = A.state, rect = document.querySelector('#canvas').getBoundingClientRect(),
          left = (18 - S.x) / S.zoom, top = (18 - S.y) / S.zoom,
          width = (rect.width - 36) / S.zoom, height = (rect.height - 36) / S.zoom;
        let point = { x: left, y: top };
        outer: for (let y = top; y < top + height - 100; y += 120)
          for (let x = left; x < left + width - 230; x += 250) {
            if (!A.graph().nodes.some(n => x + 230 > n.x - 5 && x < n.x + n.width + 5 && y + 100 > n.y - 5 && y < n.y + A.nodeHeight(n) + 5)) {
              point = { x, y };
              break outer;
            }
          }
        commit('Note added', () => {
          A.graph().notes ??= [];
          A.graph().notes.push({ id: 'note-' + ++S.generation, ...point, text: 'Add context for the next person.' });
        });
      },
      noteBody({ note, text }) {
        const n = app().graph().notes.find(n => n.id === note);
        if (n) { n.text = text; checkpoint(); }
      },
      moveNote({ note, x, y, cancelled }) {
        if (cancelled) { app().render(); return; }
        commit('Note moved', () => Object.assign(app().graph().notes.find(n => n.id === note), { x, y }));
      },
      resizeNote({ note, width, height, cancelled }) {
        if (cancelled) { app().render(); return; }
        commit('Note resized', () => Object.assign(app().graph().notes.find(n => n.id === note), { width, height }));
      },
      retireNote({ note }) {
        const A = app();
        commit('Note removed', () => { A.graph().notes = A.graph().notes.filter(n => n.id !== note); });
      },
      togglePause({ force }) {
        const A = app(), S = A.state;
        if (A.historical()) return A.timeMachine.resume();
        const paused = () => S.paused.has(S.scope);
        if (paused() && !force) {
          S.paused.delete(S.scope);
          A.toast('Scope resumed in the demo.');
        } else {
          S.paused.add(S.scope);
          A.toast(force ? 'Force pause previewed.' : 'Scope paused in the demo.');
        }
        record(A.graph().nodes[0]?.id || 'scope', paused() ? 'Pause' : 'Resume',
          force ? 'Manual force pause · preview' : 'Manual scope control · preview', { scope: S.scope });
        A.renderGraph();
      },
      inject({ actor, entered, line }) {
        const A = app(), S = A.state;
        const form = line?.form;
        if (form) {
          const n = A.findNode(actor);
          n.preview.sent = 'Value sent · ' + new Date().toLocaleTimeString();
          const value = Object.fromEntries([...form.elements].filter(el => el.name)
            .map(el => [el.name, el.type === 'checkbox' ? el.checked : el.type === 'number' ? Number(el.value) : el.value]));
          record(n.id, 'actor_arrival', 'Form value submitted', value);
          line.textContent = n.preview.sent;
          return;
        }
        const text = entered.text.trim();
        if (S.paused.has(S.scope)) { A.toast('Resume this scope before sending a demo prompt.'); return; }
        const target = A.graph().edges.find(e => e.from === actor);
        if (!target) { A.toast('Connect this output before sending a demo prompt.'); return; }
        record(target.to, 'actor_arrival', `turn ← ${actor}.event · demo`, text);
        const agent = A.findNode(target.to);
        if (agent) {
          agent.activity = 'Ready';
          agent.preview = { ...agent.preview, text: 'Prompt received. Ready for the next turn.' };
        }
        A.renderActors([target.to]);
        promptSent(agent, text);
        A.toast('Demo prompt recorded at the receiving actor.');
        return true;
      },
      async decide({ request }) {
        const A = app(), P = win.Product, r = P.approvals.find(row => row.id === request.id), decision = request.decision;
        if (!r) return;
        r.state = 'submitting';
        r.decision = decision;
        P.refreshApprovals?.();
        refreshApprovalBadge();
        await wait(650);
        if (r.outcome === 'fail') {
          r.state = 'failed';
          r.reason = 'The decision could not reach the local worker. Your request is still pending.';
          r.code = 'ApprovalWorkerSubmitError::Disconnected';
          r.outcome = 'accept';
        } else if (r.outcome === 'stale') {
          r.state = 'stale';
          r.reason = 'This request is no longer pending. No decision was applied.';
          r.code = 'RuntimeApprovalDecision::Unknown';
        } else {
          r.state = 'accepted';
          r.reason = decision === 'approve' ? 'Your approval was accepted.' : 'Your denial was accepted. The action will not proceed.';
          r.code = 'Applied';
          const n = A.allNode(r.actor);
          if (n) {
            n.permissionDecisions ??= [];
            n.permissionDecisions.unshift({ outcome: decision === 'approve' ? 'Allowed' : 'Denied', action: r.tool || r.action,
              reason: 'User decision for this request' });
          }
          record(r.actor, 'ApprovalDecision', r.reason, { decision });
        }
        countApprovals();
      },
      undo: () => restore(historyAt - 1),
      redo: () => restore(historyAt + 1),
    }),
  };
  return source;
}

export const mockupInspect = Object.freeze({
  inspectorObservations: () => ({ label: 'Simulated observations' }),
  identityLine: n => `${n.type} · /${n.id}`,
  actorAddress: n => n.id,
  portTitle: p => p[3] || p[0],
  healthBanner(n, past, at) {
    const status = n.health === 'failed' ? 'Actor failed' : n.flags?.pause ? 'Paused by you' : n.activity || 'Idle';
    return { strong: `${past ? 'Recorded state' : n.health === 'failed' ? 'Failed' : 'Alive'} · ${status}`,
      small: past ? at : n.health === 'failed' ? 'This actor stopped. Other actors continue.'
        : n.activity === 'Idle' ? 'No input waiting. The actor is available.' : 'Current observation' };
  },
});

export function mockupFace(n, paused = false) {
  const flags = flagWords(n.flags).join(' · ');
  return { life: n.health, flags, dot: { color: 'green', title: n.health === 'failed' ? 'Actor failed' : 'Actor alive' },
    row: { code: String(flags || (paused ? 'Paused' : n.activity)), color: 'green' } };
}
