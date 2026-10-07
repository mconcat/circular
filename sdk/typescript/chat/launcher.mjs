import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { generateProgram, generatorEnvironment } from '@circular/generator';
import { establish, actorCatalog, actorCreateAdmission } from '@circular/client';
import { actorQueryResultFromValue } from '@circular/protocol/actor-query';
import { connectOwnerLocal } from '@circular/client/owner-local';
import { readAgentHarnesses, harnessInstructionContext, renderInstructions, bindingProgram } from './harness-instructions.mjs';
import { harnessRow, installedProgram } from './harnesses.mjs';
import { CHAT_DIRECTORY } from './defaults.mjs';
import { PUBLIC_INSTRUCTION_FILES, connectionSection, publicInstruction, referencedDocuments } from './session-instructions.mjs';
import { SOCKET_NAME, cliIdentity } from '../cli/common.mjs';
import { currentFileSource } from './current-file.mjs';

export const workspace = path.resolve(import.meta.dirname, '..');
export const instructionBody = fs.readFileSync(new URL('./instructions.md', import.meta.url), 'utf8');
export const ceilings = Object.freeze({ maximumBytes: 1 << 20, maximumDepth: 64, maximumContainerEntries: 4096, maximumStringBytes: 65536 });
const packageRoot = specifier => new URL('../', import.meta.resolve(specifier));

export const shellQuote = value => `'${String(value).replaceAll("'", "'\\''")}'`;
const below = (parent, child) => child.startsWith(`${parent}${path.sep}`);

export function validateState(state, home = os.userInfo().homedir) {
  if (!path.isAbsolute(state)) throw new Error('--state requires an absolute directory');
  const actualHome = fs.realpathSync(home);
  const canonical = fs.realpathSync(path.resolve(state));
  if (!below(actualHome, canonical)) throw new Error('state must be strictly below the user home');
  const stat = fs.lstatSync(canonical);
  if (!stat.isDirectory() || stat.uid !== process.getuid() || (stat.mode & 0o7777) !== 0o700) {
    throw new Error('state must be an owned directory with mode 0700');
  }
  return canonical;
}

function privateDirectory(directory) {
  try { fs.mkdirSync(directory, { mode: 0o700 }); }
  catch (error) { if (error.code !== 'EEXIST') throw error; }
  const stat = fs.lstatSync(directory);
  if (!stat.isDirectory() || stat.isSymbolicLink() || stat.uid !== process.getuid() || (stat.mode & 0o7777) !== 0o700) {
    throw new Error(`unsafe chat directory: ${directory}`);
  }
}

export function harnessEnvironment(base = process.env, harness = null) {
  const env = { ...base };
  if (harness !== null) for (const name of harnessRow(harness).removeEnvironment) delete env[name];
  return env;
}

export function resolveHarnessProgram(name, searchPath = process.env.PATH ?? '') {
  return installedProgram(name, searchPath) ?? name;
}

export function inspectHarness(harness, program, cwd, run = spawnSync) {
  const row = harnessRow(harness);
  const result = run(program, ['--help'], { cwd, env: harnessEnvironment(process.env, harness), encoding: 'utf8', timeout: 5000, maxBuffer: 1024 * 1024 });
  const help = result.stdout ?? '';
  const verified = row.helpPatterns !== null && !result.error && result.status === 0 && row.helpPatterns.every(pattern => pattern.test(help));
  return { verified, help, diagnostic: verified ? null : 'interactive arguments were not verified by --help; read first-message.md and load it manually using your CLI documentation' };
}

export function renderScript({ directory, program, verified, harness }) {
  const lines = ['#!/bin/sh', 'set -eu', `cd ${shellQuote(directory)}`];
  for (const name of harnessRow(harness).removeEnvironment) lines.push(`unset ${name}`);
  if (verified) lines.push('# Positional prompt and interactive default verified by this executable\'s --help.', `exec ${shellQuote(program)} "$(cat first-message.md)"`);
  else lines.push("printf '%s\\n' 'Interactive arguments could not be verified. Read first-message.md and the instruction file named there; load them manually using your CLI documentation.'", `printf '%s\\n' ${shellQuote(path.join(directory, 'first-message.md'))}`);
  return `${lines.join('\n')}\n`;
}

/** The SDK dependencies belong to the CLI package manifest. */
export const SDK_PACKAGES = Object.freeze(Object.keys(
  JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url), 'utf8')).dependencies,
).filter(name => name.startsWith('@circular/')).sort());

export function packageManifest() {
  const dependencies = {};
  for (const name of SDK_PACKAGES) dependencies[name] = `file:${fileURLToPath(packageRoot(name)).replace(/\/$/, '')}`;
  dependencies.typescript = `file:${fileURLToPath(new URL('../', import.meta.resolve('typescript'))).replace(/\/$/, '')}`;
  return `${JSON.stringify({ name: 'circular-chat-session', private: true, type: 'module', dependencies }, null, 2)}\n`;
}

export function installPackages(directory, run = spawnSync) {
  const result = run('npm', ['install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', '--package-lock=false', '--cache', path.join(directory, '.npm-cache')],
    { cwd: directory, env: harnessEnvironment(), encoding: 'utf8', timeout: 30000, maxBuffer: 1024 * 1024 });
  if (result.status === 0 && !result.error) return 'npm install --offline succeeded; @circular/* are local file: dependencies.';
  const modules = path.join(directory, 'node_modules');
  if (fs.existsSync(modules)) fs.renameSync(modules, path.join(directory, 'node_modules.partial'));
  fs.symlinkSync(path.join(workspace, 'node_modules'), modules, 'dir');
  return `npm install --offline failed (${result.error?.code ?? result.status}); node_modules links to the installed SDK workspace. If imports fail, restore local SDK dependencies before retrying; do not fetch packages or credentials.`;
}

export const emptySource = '// No pipeline yet — save a new SDK program here.\nexport {};\n';

async function daemonAuthoringMetadata(session) {
  const catalog = await actorCatalog(session);
  if (catalog.status !== 'accepted') throw new Error(`actor.catalog refused: ${catalog.diagnostics.map(d => d.message).join('; ')}`);
  const answer = await session.exchange('Query', 'Query', { name: 'actor.create-inputs', args: null });
  if (answer.kind?.verb !== 'QueryResult') throw new Error('actor.create-inputs returned an unexpected response');
  const inputs = actorQueryResultFromValue(answer.payload, item => {
    if (!item || typeof item.actor_type !== 'string' || !Array.isArray(item.state)) throw new Error('actor.create-inputs incomplete item');
    return item;
  });
  if (inputs.status !== 'accepted') throw new Error(`actor.create-inputs refused: ${inputs.diagnostics.map(d => d.message).join('; ')}`);
  const names = catalog.value.items.map(row => row.actor_type);
  if (names.length !== inputs.value.items.length || names.some((name, i) => inputs.value.items[i].actor_type !== name)) {
    throw new Error('actor.catalog and actor.create-inputs identities differ');
  }
  const show = value => JSON.stringify(value, (_, v) => typeof v === 'bigint' ? `${v}n` : v);
  const reference = '# Connected daemon authoring reference\n\n'
    + 'These are actor.catalog and actor.create-inputs responses from this session. '
    + 'The execution host uses actor.create-admission before mutation. This file is guidance, not authored state.\n\n'
    + catalog.value.items.map((row, i) => `## ${row.actor_type}\n\n${row.label}: ${row.description}\n\n`
      + `Inlets: ${show(row.in_ports)}\n\nOutlets: ${show(row.out_ports)}\n\n`
      + `Config input metadata: ${show(inputs.value.items[i].state)}\n`).join('\n');
  return { catalog: catalog.value.items, reference };
}

export async function snapshotFromSession(session) {
  const metadata = await daemonAuthoringMetadata(session);
  const snapshot = await session.authoringSnapshot([], 256);
  if (snapshot.status !== 'accepted') throw new Error(`authoring snapshot refused (${snapshot.reason ?? snapshot.status}): ${[...(snapshot.diagnostics ?? []), snapshot.diagnostic].filter(Boolean).map(d => `${d.code}: ${d.message}`).join("; ")}`);
  const admissions = new Map();
  for (const command of snapshot.value.commands.filter(c => c.kind === 'UpsertActor')) {
    const row = metadata.catalog.find(r => r.actor_type === command.declaration.actorType);
    if (!row) throw new Error(`actor.catalog does not publish ${command.declaration.actorType}`);
    if (row.ports_unavailable_reason === null || command.declaration.actorType === 'pipeline_actor') continue;
    const result = await actorCreateAdmission(session, command.declaration.actorType, command.declaration.config, command.actor.value);
    if (result.status !== 'accepted' || result.value.items.length !== 1) throw new Error(`actor.create-admission refused: ${result.diagnostics?.map(d => d.message).join('; ') ?? 'incomplete'}`);
    const key = [...command.actor.value.scope.map(s => s.name), command.actor.value.local].join('/');
    admissions.set(key, result.value.items[0]);
  }
  const generated = generateProgram(snapshot.value.commands, {
    ...generatorEnvironment, specSet: snapshot.value.anchor.environment.specSet, catalog: metadata.catalog, admissions,
  });
  if (generated.status !== 'complete') throw new Error(`current reconstruction refused: ${generated.diagnostics.map(d => `${d.message} ${d.args?.filter(Boolean).join(' ')}`).join('; ')}`);
  const { program } = generated.value;
  const actors = snapshot.value.commands.filter(command => command.kind === 'UpsertActor').length;
  return { program, cursor: snapshot.value.anchor.cursor, source: snapshot.value.commands.length ? new TextDecoder().decode(program.modules.get(program.entry)) : emptySource,
    actors, scope: snapshot.value.anchor.scope ?? [], daemon: 'connected', authoringReference: metadata.reference };
}

export async function readCurrent(state) {
  let transport;
  try { transport = await connectOwnerLocal({ root: state, socketName: SOCKET_NAME }); }
  catch (error) {
    if (error.code !== 'SOCKET_MISSING'
      && !(error.code === 'CONNECT_FAILED' && ['ECONNREFUSED', 'ENOENT'].includes(error.cause?.code))) throw error;
    return { source: emptySource, actors: 0, scope: [], daemon: 'absent; no pipeline yet' };
  }
  let session;
  try {
    session = await establish(transport, { hello: { requestedRoles: [1n] }, resourceCeilings: ceilings, requestTimeoutMs: 5000 });
    return { ...await snapshotFromSession(session), agentHarnesses: await readAgentHarnesses(session) };
  } finally {
    if (session) await session.goodbye().catch(() => session.close());
    else await transport.close();
  }
}

export function firstMessage({ state, project, instructionPath, current, installation, harnessContext = harnessInstructionContext(current) }) {
  const programFile = current.program?.modules.size > 1 ? 'current/main.ts and its scope modules' : current.program ? 'current.ts' : 'main.ts';
  return `Read these files in this directory, in this order, before you change anything:\n1. AGENTS.md: how you author a pipeline here. Its last section describes this session and lists where the reference pages are on this machine.\n2. ${instructionPath}: the SDK spellings for this daemon and this state. Where it is more specific than AGENTS.md, it wins.\n3. ${programFile}: the pipeline SDK code standing now. An absent daemon has an empty skeleton, not a recovered graph.\n\nState directory: ${JSON.stringify(state)}\nProject label: ${JSON.stringify(project ?? '(unnamed)')} (display only; not a semantic ProjectId or scope selector)\nDaemon: ${current.daemon}\nActors: ${current.actors}\nScope: ${JSON.stringify(current.scope)}\n\n${harnessContext.text}\n\nThe current program comes from the SDK's program generator and is read-only. Programs you write run through the installed code execution host.\n${installation}\n\nWork on what the user asks for. Do not deploy, pause or edit the pipeline on your own initiative, and do not start a tutorial unless the user asks for one.\n\nDo not read, copy, or print credentials, set environment variables for behavior, or write outside this state directory. node_modules and its SDK targets are read-only.\n`;
}

export async function createChatSession(options, dependencies = {}) {
  const diagnostics = [];
  const row = harnessRow(options.harness);
  const state = validateState(options.state);
  const current = await (dependencies.readCurrent ?? readCurrent)(state);
  const selected = dependencies.selectHarness?.(current, options);
  const parent = path.join(state, CHAT_DIRECTORY);
  privateDirectory(parent);
  const stamp = (dependencies.now?.() ?? new Date()).toISOString().replaceAll(':', '-');
  let directory = path.join(parent, `${stamp}-${encodeURIComponent(options.harness)}`);
  for (let collision = 0; ; collision++) {
    try { fs.mkdirSync(directory, { mode: 0o700 }); break; }
    catch (error) {
      if (error.code !== 'EEXIST') throw error;
      directory = path.join(parent, `${stamp}-${collision + 1}-${encodeURIComponent(options.harness)}`);
    }
  }
  const write = (file, body, mode = 0o600) => fs.writeFileSync(path.join(directory, file), body, { mode, flag: 'wx' });
  write('package.json', packageManifest());
  const installation = options.dryRun ? 'Dry run: recovery files written; package installation deferred.'
    : (dependencies.installPackages ?? installPackages)(directory);
  const instructionPath = row.instruction.path;
  if (instructionPath.includes('/')) fs.mkdirSync(path.dirname(path.join(directory, instructionPath)), { recursive: true, mode: 0o700 });
  const program = selected?.program ?? options.harnessBin ?? resolveHarnessProgram(options.harness);
  const harnessContext = harnessInstructionContext(current, program);
  const instructions = renderInstructions(instructionBody, harnessContext)
    + (current.authoringReference ? '\nRead actor-catalog.md for the connected daemon’s actor/config/port metadata. Admission responses are authoritative.\n' : '');
  if (current.authoringReference) write('actor-catalog.md', current.authoringReference);
  write(instructionPath, `${row.instruction.frontMatter ?? ''}${instructions}`);
  if (current.program?.modules.size > 1) {
    for (const [module, bytes] of current.program.modules) {
      const file = path.join('current', module);
      fs.mkdirSync(path.dirname(path.join(directory, file)), { recursive: true, mode: 0o700 });
      write(file, currentFileSource(bytes, current.cursor));
    }
  } else write(current.program ? 'current.ts' : 'main.ts', currentFileSource(current.source, current.program ? current.cursor : null));
  write('first-message.md', firstMessage({ state, project: options.project, instructionPath, current, installation, harnessContext }));
  const rootInstructions = [];
  const identity = await cliIdentity();
  for (const name of PUBLIC_INSTRUCTION_FILES) {
    const shipped = publicInstruction(name);
    if (shipped.text === null) {
      diagnostics.push(`public instruction ${name} is not in this installation; the session directory has none`);
      continue;
    }
    const body = name === 'AGENTS.md'
      ? shipped.text + connectionSection({
        state, socket: path.join(state, SOCKET_NAME), agentCli: options.harness, agentCliProgram: program,
        instructionPath, cliVersion: identity.version, cliCommit: identity.commit,
        daemonVersion: dependencies.daemonVersion ?? null, daemon: current.daemon,
        documents: referencedDocuments(path.dirname(shipped.path), path.join(workspace, 'templates')),
      })
      : shipped.text;
    write(name, body);
    rootInstructions.push({ file: name, origin: shipped.origin, source: shipped.path });
  }
  const inspection = options.dryRun ? { verified: false, diagnostic: 'Dry run: agent CLI inspection deferred.' }
    : (dependencies.inspectHarness ?? inspectHarness)(options.harness, program, directory);
  const script = renderScript({ directory, program, verified: inspection.verified, harness: options.harness });
  write('launch.command', script, 0o700);
  const canonical = value => { try { return fs.realpathSync(value); } catch { return path.resolve(value); } };
  const boundHarness = current.agentHarnesses?.status === 'reported' && current.agentHarnesses.bindings
    .find(binding => canonical(bindingProgram(binding)) === canonical(program));
  return { directory, harnessProgram: program, boundHarness: boundHarness?.name ?? null, instructionPath, rootInstructions, actors: current.actors,
    scriptPath: path.join(directory, 'launch.command'), script,
    diagnostics: [installation, ...(inspection.diagnostic ? [inspection.diagnostic] : []), ...diagnostics] };
}

export function openTerminal(scriptPath, run = spawnSync) {
  return run('/usr/bin/open', ['-a', 'Terminal', scriptPath], { encoding: 'utf8', timeout: 5000, env: harnessEnvironment() });
}
