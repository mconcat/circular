import { tagsFor } from '../../protocol/src/wire.js';
import { runWithExecutionContext } from '@circular/core/internal';
import { compactTemplateCommands } from '@circular/generator/internal';
import ts from 'typescript';
import vm from 'node:vm';
import { createExportCollector } from './export-surfaces.js';
import { generateProgramDeclarations } from './program-declarations.js';
import { isDeepStrictEqual } from 'node:util';
import * as core from '@circular/core';
import * as exportsRuntime from '@circular/exports';
import { deriveBoundaryPortId } from '@circular/protocol';
import { SCOPE_ROLES } from '@circular/core';
import { emitDeclaration, createCurrentActorHandle, metadataOf } from '@circular/core/internal';
import { adaptAuthoringSession, actorCatalog, actorCreateAdmission } from '@circular/client';
import { DEFAULT_EDGE_ATTRS, declarationAddressValue, edgeKeyFromDeclaration } from '@circular/protocol/declaration';
import { semanticPrepass, isPreparedProgram, authoringError, diagnostic } from './prepass.js';
import { hydrateCurrentProject } from './current-module.js';
import { exceptionDiagnostic } from './exception-diagnostic.js';
import { thrownSourceSpan } from './source-span.js';
import { declarationCommandFromValue, declarationPayloadValue } from '@circular/protocol/declaration';
import { standingPresentations } from './standing-presentations.js';

const fallbackSpan = { source: '<authoring>', startLine: 1, startColumn: 1, endLine: 1, endColumn: 1 };
/** A thrown value becomes one Host diagnostic at `span`; an `authoring.*` message keeps its own key. */
function programThrew(error, span) {
  const wrapped = new Error(String(error?.message ?? error), { cause: error });
  wrapped.circularDiagnostics = [exceptionDiagnostic(diagnostic(
    typeof error?.message === 'string' && error.message.startsWith('authoring.')
      ? error.message : 'authoring.execution-program-threw', span, 'Host'), error)];
  return wrapped;
}
/**
 * Wire values decode as null-prototype objects (hostile-key safety) while authored literals are plain
 * objects; `isDeepStrictEqual` treats that prototype difference as inequality (measured live: map's
 * admitted config and authored_actor came back "unequal" while byte-identical). Compare structure only.
 */
function plain(value) {
  if (Array.isArray(value)) return value.map(plain);
  if (value instanceof Uint8Array || value === null || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, plain(item)]));
}
export function wireDeepEqual(left, right) { return isDeepStrictEqual(plain(left), plain(right)); }
export function resolvedPorts(inputs, outputs) {
  const primary = list => { const selected = list.filter(p => p.primary); return selected.length === 1 ? selected[0].id : null; };
  const lookup = (list, name) => {
    if (!list.some(p => p.id === name)) throw new TypeError(`authoring.port.unknown: ${name}`);
    return name;
  };
  return { inputs, outputs, defaultInput: primary(inputs), defaultOutput: primary(outputs),
    input: name => lookup(inputs, name), output: name => lookup(outputs, name) };
}
async function admit(session, program, scope) {
  const table = new Map(), pairs = [];
  const catalog = await actorCatalog(session);
  if (catalog.status === 'rejected') throw authoringError(catalog.diagnostics[0].message, fallbackSpan, 'Admission', catalog.diagnostics[0]);
  const templateModule = module => Boolean(program.templates?.some(template => template.module === module));
  for (const call of program.calls.filter(c => c.spelling)) {
    const row = catalog.value.items.find(x => x.actor_type === call.actorType);
    if (!row) throw authoringError(call.actorType === 'match' ? 'authoring.match.engine-capture-pending' : 'authoring.actor.not-published', call.origin);
    let value;
    if (call.templateName) {
      const child = program.scopePlan.find(plan => plan.module === call.childModule);
      const ports = direction => child.boundaries.filter(b => b.direction === direction).map(b => ({
        id: deriveBoundaryPortId(direction === 'in' ? 'inlet' : 'outlet', { scope: [], local: program.calls[b.call].binding }, 0n), primary: false,
      }));
      value = { config: call.config, in_ports: call.actorType === 'replicator' ? row.in_ports : ports('in'), out_ports: ports('out') };
    } else if (call.actorType === 'pipeline_actor') {
      if (call.config !== null) throw authoringError('authoring.actor.config-not-empty', call.origin);
      value = { ...row, config: row.template_config };
    } else if (row.config_schema === 1n) {
      if (!row.creatable) throw authoringError('authoring.actor.not-creatable', call.origin, 'Admission', { code: 0, message: row.unavailable_reason });
      value = { ...row, config: call.config };
    } else {
      const key = { scope: templateModule(call.module) ? [] : [...scope, ...call.scope], local: call.binding };
      const result = await actorCreateAdmission(session, call.actorType, call.config, key);
      if (result.status === 'rejected') throw authoringError(result.diagnostics[0].message, call.origin, 'Admission', result.diagnostics[0]);
      value = result.value.items[0];
      const expectedKey = declarationAddressValue({ arm: 'absolute', value: key }, 'actor', 'mutation')[1];
      if (result.value.items.length !== 1 || result.value.anchor !== call.actorType || value.actor_type !== call.actorType
        || !wireDeepEqual(value.authored_actor, expectedKey)) throw authoringError('authoring.actor.admission-identity-mismatch', call.origin);
      pairs.push({ module: call.module, binding: call.binding, actorType: call.actorType,
        authored: call.config, admitted: value.config, equal: wireDeepEqual(call.config, value.config),
        ...(call.actorType === 'replicator' ? { admissionPath: 'actor.create-admission' } : {}) });
    }
    if ((call.childModule || call.authoredConfig) && !wireDeepEqual(call.config, value.config)) {
      throw authoringError('authoring.actor.boundary-admission-mismatch', call.origin);
    }
    const ports = resolvedPorts(value.in_ports, value.out_ports);
    if (call.templateName) {
      const child = program.scopePlan.find(plan => plan.module === call.childModule);
      for (const [direction, method] of [['in', 'input'], ['out', 'output']]) {
        if (call.actorType === 'replicator' && direction === 'in') continue;
        const lookup = ports[method];
        ports[direction === 'in' ? 'inputs' : 'outputs'] = child.boundaries.filter(b => b.direction === direction).map(b => ({ id: b.topic, primary: false }));
        ports[method] = topic => {
          const boundary = child.boundaries.find(b => b.direction === direction && b.topic === topic);
          return lookup(boundary ? deriveBoundaryPortId(direction === 'in' ? 'inlet' : 'outlet', { scope: [], local: program.calls[boundary.call].binding }, 0n) : topic);
        };
      }
    }
    table.set(call.id, { declaration: { actorType: call.actorType, config: value.config,
      flags: { bypass: false, mute: false, pause: false } }, ports });
  }
  for (const plan of program.scopePlan.filter(p => p.container !== null)) {
    const topics = { inlets: new Map(), outlets: new Map() };
    for (const item of plan.boundaries) {
      const call = program.calls[item.call], resolution = table.get(item.call);
      const ports = item.direction === 'in' ? resolution.ports.outputs : resolution.ports.inputs;
      if (ports.length !== 1) throw authoringError('authoring.actor.boundary-admission-mismatch', call.origin);
      topics[item.direction === 'in' ? 'inlets' : 'outlets'].set(item.topic, ports[0].id);
    }
    const resolution = table.get(plan.container);
    const ports = resolvedPorts([...topics.inlets.keys()].map(id => ({ id, primary: false })),
      [...topics.outlets.keys()].map(id => ({ id, primary: false })));
    resolution.ports = { ...ports, input: name => topics.inlets.get(ports.input(name)), output: name => topics.outlets.get(ports.output(name)) };
  }
  return { table, pairs };
}
function preparedEvaluator(program, table, scope, currentModule, entry = program.executable.entry) {
  let binding = null, call = null, allocations = 0, activeModule = entry, activeScope = [];
  const frames = [];
  const mounts = createExportCollector(scope, message => { throw authoringError(message, call?.origin ?? fallbackSpan); });
  const full = relative => ({ ...relative, scope: [...scope, ...relative.scope] });
  const actorKey = name => ({ arm: 'epochLocal', value: { scope: [...scope, ...activeScope], local: name } });
  const relativeActor = address => {
    if (address.arm !== 'epochLocal' && address.arm !== 'absolute') throw authoringError('authoring.edge.scope-mismatch', call?.origin ?? fallbackSpan);
    if (!isDeepStrictEqual(address.value.scope.slice(0, scope.length), scope)) throw authoringError('authoring.edge.scope-mismatch', call?.origin ?? fallbackSpan);
    return { ...address.value, scope: address.value.scope.slice(scope.length) };
  };
  const edgeDeclaration = declaration => {
    if (!isDeepStrictEqual(relativeActor(declaration.from.actor).scope, relativeActor(declaration.to.actor).scope)) {
      throw authoringError('authoring.edge.scope-mismatch', call?.origin ?? fallbackSpan);
    }
    return {
    from: { actor: full(relativeActor(declaration.from.actor)), port: declaration.from.port },
    to: { actor: full(relativeActor(declaration.to.actor)), port: declaration.to.port },
    ordinal: declaration.ordinal, attrs: declaration.attrs,
    };
  };
  const instrumentation = {
    surface(id) {
      const entry = program.surfaces[id];
      if (entry.module !== activeModule) throw authoringError('authoring.prepass.required', fallbackSpan);
      mounts.surface(activeScope, entry.name, entry.value);
    },
    bind(name, callback) {
      const previous = [binding, allocations]; binding = name; allocations = 0;
      try { return callback(); } finally { [binding, allocations] = previous; }
    },
    call(id, callback) {
      const previous = call; call = program.calls[id];
      try {
        const handle = callback();
        if (call.childModule && !call.templateName) {
          const framed = openScope(call.binding);
          try {
            emitDeclaration({ kind: 'UpsertScope', scope: framed.scope,
              declaration: { role: SCOPE_ROLES.concrete, boundary: { inlets: [], outlets: [] } } });
            const children = evaluateModule(call.childModule);
            Object.defineProperty(handle, 'actors', { value: Object.freeze(Object.assign(Object.create(null), children)), enumerable: true });
          } finally { closeScope(); }
        }
        return handle;
      } catch (error) {
        if (Array.isArray(error?.circularDiagnostics)) throw error;
        throw programThrew(error, call.origin);
      } finally { call = previous; }
    },
    forward(name) {
      const entry = program.bindings.find(b => b.module === activeModule && b.name === name);
      const resolved = table.get(entry?.call);
      if (!resolved) throw authoringError('authoring.prepass.forward-reference-unresolved', call?.origin ?? fallbackSpan);
      return createCurrentActorHandle({ ...resolved, actor: actorKey(name), spelling: program.calls[entry.call].spelling });
    },
  };
  function openScope(name) {
    const container = actorKey(name);
    frames.push(activeScope);
    activeScope = [...activeScope, { name }];
    return { scope: { arm: 'epochLocal', value: [...scope, ...activeScope] }, container };
  }
  function closeScope() {
    if (!frames.length) throw authoringError('authoring.scope.frame-underflow', fallbackSpan);
    activeScope = frames.pop();
  }
  function evaluateModule(path) {
    const previous = activeModule; activeModule = path;
    try {
      const text = new TextDecoder().decode(program.executable.modules.get(path));
      const compiled = ts.transpileModule(text, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, sourceMap: true }, fileName: path });
      const load = specifier => {
        if (specifier === '@circular/core') return core;
        if (specifier === '@circular/exports') return exportsRuntime;
        if (specifier === 'circular:current' && currentModule) {
          let local = currentModule.exports;
          for (const segment of activeScope) {
            local = local[segment.name]?.actors;
            if (!local) throw authoringError('authoring.current.scope-missing', call?.origin ?? fallbackSpan);
          }
          return activeScope.length ? Object.assign(Object.create(null), local,
            { current: currentModule.current, default: currentModule.current }) : local;
        }
        throw authoringError('authoring.prepass.import-provenance', fallbackSpan);
      };
      const module = { exports: {} };
      const filename = `circular-authoring:${path}`;
      const body = compiled.outputText.replace(/\n\/\/# sourceMappingURL=[^\n]*\s*$/, '\n');
      const run = vm.compileFunction(body, ['require', 'module', 'exports', '__circular'], { filename });
      try { run(load, module, module.exports, instrumentation); }
      catch (error) {
        if (Array.isArray(error?.circularDiagnostics)) throw error;
        const span = thrownSourceSpan(error, { filename, path, transpiledSourceMap: compiled.sourceMapText, sourceMap: program.sourceMap });
        throw span === null ? error : programThrew(error, span);
      }
      return module.exports;
    } finally { activeModule = previous; }
  }
  return {
    deferCommands: true,
    openScope, closeScope,
    declareExportSurface({ name, surface }) { mounts.surface(activeScope, name, surface); },
    declareExportMount({ name, role, endpoint }) {
      const mountScope = [...activeScope];
      const actor = relativeActor(endpoint.actor);
      if (!isDeepStrictEqual(actor.scope.slice(0, mountScope.length), mountScope)) throw authoringError('authoring.export.scope-mismatch', call?.origin ?? fallbackSpan);
      mounts.role(mountScope, name, role, { actor: full(actor), port: endpoint.port });
    },
    resolve(spelling, config) {
      if (!call?.spelling || call.spelling !== spelling || call.binding !== binding) throw authoringError('authoring.prepass.unbound-actor', call?.origin ?? fallbackSpan);
      if (!isDeepStrictEqual(config, call.authoredConfig ?? call.config)) throw authoringError('authoring.prepass.config-not-literal', call.origin);
      return table.get(call.id);
    },
    allocateReference(kind, declaration) {
      if (kind === 'edge') return { arm: 'epochLocal', value: edgeKeyFromDeclaration(edgeDeclaration(declaration)) };
      if (kind === 'annotation' && call?.annotation && binding && ++allocations === 1) return actorKey(declaration ?? binding);
      if (kind !== 'actor' || !binding || ++allocations !== 1) throw authoringError('authoring.prepass.unbound-actor', call?.origin ?? fallbackSpan);
      return actorKey(binding);
    },
    lowerCommand(command) {
      if (command.kind === 'SetPresentation' && command.presentation.anchor?.target?.arm) {
        const anchor = command.presentation.anchor;
        const owner = relativeActor(command.owner.actor ?? command.owner.annotation), target = relativeActor(anchor.target);
        if (!isDeepStrictEqual(owner.scope, target.scope)) throw authoringError('authoring.presentation.scope-mismatch', call?.origin ?? fallbackSpan);
        return { ...command, presentation: { ...command.presentation, anchor: { ...anchor, target: full(target) } } };
      }
      if (command.kind === 'UpsertAnnotation') {
        const owner = relativeActor(command.annotation), refs = command.declaration.refs.map(ref => relativeActor(ref));
        if (refs.some(ref => !isDeepStrictEqual(ref.scope, owner.scope))) throw authoringError('authoring.prepass.note-ref-unresolved', call?.origin ?? fallbackSpan);
        const lowered = { ...command, declaration: { ...command.declaration, refs: refs.map(full) } };
        return declarationCommandFromValue(lowered.kind, declarationPayloadValue(lowered));
      }
      if (command.kind === 'UpsertEdge') return { ...command, declaration: edgeDeclaration(command.declaration) };
      return command;
    },
    evaluate(input, { current }) {
      if (!isPreparedProgram(input) || input !== program) throw authoringError('authoring.prepass.required', fallbackSpan);
      evaluateModule(entry);
      for (const command of mounts.commands()) emitDeclaration(command);
    },
  };
}
/** Collect the prepared closure with the ordinary core runtime, without opening a writer epoch. */
export async function collectTemplateProgram(session, program, options, edgeFactory, currentModule = null, policies = defaultEdgePolicies()) {
  if (!isPreparedProgram(program)) throw authoringError('authoring.prepass.required', fallbackSpan);
  if (!program.templates || options.targetScope.length) throw authoringError('authoring.template.root-epoch-required', fallbackSpan);
  const admitted = await admit(session, program, options.targetScope);
  if (!isPreparedProgram(program)) throw authoringError('authoring.prepass.bundle-digest-mismatch', fallbackSpan);
  const authoredOrderEntry = Boolean(currentModule) || program.scopePlan.some(plan => plan.container !== null);
  const collect = (module, relative) => {
    const evaluator = preparedEvaluator(program, admitted.table, relative ? [] : options.targetScope, currentModule, module);
    const edges = edgeFactory({ policies }).create(options), commands = [];
    const presentations = standingPresentations(relative ? null : options.currentSnapshot, options.targetScope);
    runWithExecutionContext({
      presentationOf: presentations.of,
      allocateReference: evaluator.allocateReference,
      resolveActor: evaluator.resolve,
      resolveEdge: (...args) => edges.resolve(...args),
      declareExportMount: evaluator.declareExportMount,
      declareExportSurface: evaluator.declareExportSurface,
      emit: command => {
        if (command?.kind === 'RetireEdge') edges.release?.(command.edge);
        presentations.observe(command);
        commands.push(evaluator.lowerCommand(command));
      },
    }, () => evaluator.evaluate(program, { current: currentModule?.current ?? null }));
    return !relative && authoredOrderEntry ? commands : compactTemplateCommands(commands, relative);
  };
  const templates = new Map();
  for (const plan of program.scopePlan.filter(plan => plan.role === 'Template')
    .sort((a, b) => Buffer.compare(Buffer.from(a.name), Buffer.from(b.name)))) {
    const commands = collect(plan.module, true);
    if (templates.has(plan.name) && !wireDeepEqual(templates.get(plan.name).commands, commands)) {
      throw authoringError('authoring.template.name-conflict', fallbackSpan);
    }
    templates.set(plan.name, { kind: 'UpsertTemplate', name: plan.name, commands });
  }
  return { commands: [...templates.values(), ...collect(program.executable.entry, false)], admissionPairs: admitted.pairs,
    declarations: generateProgramDeclarations(program, admitted.table) };
}

/**
 * The product edge policy: omitted options mean `DEFAULT_EDGE_ATTRS`; an explicit
 * `policy` (a complete protocol WirePolicy) or `delay` (protocol rational seconds)
 * overrides the corresponding member. No other vocabulary is interpreted here.
 */
export function defaultEdgePolicies() {
  return Object.freeze({
    resolve(options = {}) {
      const delay = options.delay === undefined ? DEFAULT_EDGE_ATTRS.delay : options.delay;
      if (!delay || typeof delay.num !== 'bigint' || typeof delay.den !== 'bigint'
        || delay.num < 0n || delay.den <= 0n) throw new TypeError('authoring.edge.delay-invalid');
      return Object.freeze({
        delay: Object.freeze({ num: delay.num, den: delay.den }),
        policy: options.policy ?? DEFAULT_EDGE_ATTRS.policy,
      });
    },
  });
}
/** Supplies the five existing host seams. All async work finishes before BeginEpoch. */
export function installCodeExecutionHost(dependencies, compose, edgeFactory) {
  const policies = dependencies.policies ?? defaultEdgePolicies();
  const session = adaptAuthoringSession(dependencies.session);
  return Object.freeze({
    async execute(source, options) {
      let pairs = [], declarations;
      try {
        if (source?.executable && !isPreparedProgram(source)) throw authoringError('authoring.prepass.required', fallbackSpan);
        const prepared = isPreparedProgram(source) ? { status: 'complete', value: source }
          : semanticPrepass(source, dependencies.profile, {});
        if (prepared.status === 'rejected') return { ...prepared, protocol: null };
        const program = prepared.value;
        if (program.templates) {
          try { tagsFor('Declaration', 'UpsertTemplate'); tagsFor('Declaration', 'RetireTemplate'); }
          catch { throw authoringError('authoring.template.engine-capture-pending', fallbackSpan); }
          let currentModule = null;
          if (program.imports.some(item => item.specifier === 'circular:current')) {
            currentModule = (await hydrateCurrentProject(dependencies.session, options.targetScope, dependencies.pageLimit ?? 64, options.currentSnapshot ?? null)).module;
          }
          const collected = await collectTemplateProgram(dependencies.session, program, options, edgeFactory, currentModule, policies);
          const host = compose({ session, actors: { resolve: () => { throw new Error('prepared declarations require no actor allocation'); } },
            evaluator: { deferCommands: true, evaluate: () => { for (const command of collected.commands) emitDeclaration(command); } },
            edges: edgeFactory({ policies }),
            currentProjectResolver: { resolve: () => ({ status: 'complete', value: currentModule, diagnostics: [] }) } });
          return { ...await host.execute(program, { ...options, currentSnapshot: null }),
            admissionPairs: collected.admissionPairs, declarations: collected.declarations };
        }
        const admitted = await admit(dependencies.session, program, options.targetScope);
        pairs = admitted.pairs;
        declarations = generateProgramDeclarations(program, admitted.table);
        let currentModule = null, snapshot = options.currentSnapshot ?? null;
        if (program.imports.some(x => x.specifier === 'circular:current')) {
          const hydrated = await hydrateCurrentProject(dependencies.session, options.targetScope, dependencies.pageLimit ?? 64, snapshot);
          currentModule = hydrated.module; snapshot = hydrated.snapshot;
        }
        if (!isPreparedProgram(program)) throw authoringError('authoring.prepass.bundle-digest-mismatch', fallbackSpan, 'Prepass');
        const evaluator = preparedEvaluator(program, admitted.table, options.targetScope, currentModule);
        const host = compose({ session, actors: { resolve: evaluator.resolve }, evaluator,
          edges: edgeFactory({ policies }),
          currentProjectResolver: { resolve: () => ({ status: 'complete', value: currentModule, diagnostics: [] }) } });
        const result = await host.execute(program, { ...options, currentSnapshot: snapshot });
        return { ...result, admissionPairs: pairs, declarations };
      } catch (error) {
        return { status: 'rejected', protocol: null, admissionPairs: pairs, ...(declarations ? { declarations } : {}),
          diagnostics: error?.circularDiagnostics ?? [exceptionDiagnostic(diagnostic('authoring.host.adapter-failed', fallbackSpan, 'Host',
            { code: 0, message: String(error?.message ?? error) }), error)] };
      }
    },
  });
}
