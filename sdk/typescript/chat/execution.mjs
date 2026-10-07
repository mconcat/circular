/** File input for the installed SDK host, shared by edit and its session deploy command. */
import fs from 'node:fs';
import path from 'node:path';
import { randomBytes } from 'node:crypto';
import ts from 'typescript';
import { createCodeExecutionHost, semanticPrepass } from '@circular/authoring';
import { establish, waitForAdoption } from '@circular/client';
import { connectOwnerLocal } from '@circular/client/owner-local';
import { ceilings } from './launcher.mjs';

export function loadProgram(file, root = path.dirname(file)) {
  root = fs.realpathSync(root);
  const moduleRoot = path.dirname(fs.realpathSync(file));
  const modules = new Map();
  function visit(file) {
    const actual = fs.realpathSync(file);
    if (!actual.startsWith(root + path.sep) || !actual.endsWith('.ts')) {
      throw new Error('SDK program modules must be TypeScript files inside the session');
    }
    if (!actual.startsWith(moduleRoot + path.sep)) throw new Error('SDK program modules must stay inside the session program directory');
    const name = path.relative(moduleRoot, actual).split(path.sep).join('/');
    if (modules.has(name)) return name;
    const bytes = fs.readFileSync(actual);
    modules.set(name, bytes);
    for (const imported of ts.preProcessFile(bytes.toString('utf8'), true, true).importedFiles) {
      if (!imported.fileName.startsWith('.')) continue;
      const target = path.resolve(path.dirname(actual), imported.fileName);
      visit(path.extname(target) ? target : target + '.ts');
    }
    return name;
  }
  const entry = visit(file);
  function collect(directory) {
    if (!fs.existsSync(directory)) return;
    if (!fs.realpathSync(directory).startsWith(moduleRoot + path.sep)) throw new Error('bundle directory must stay inside the session');
    for (const item of fs.readdirSync(directory, { withFileTypes: true })) {
      const target = path.join(directory, item.name);
      if (item.isDirectory()) collect(target);
      else if (item.name.endsWith('.ts')) visit(target);
      else if (item.isSymbolicLink()) throw new Error('bundle symlink must stay inside the session');
    }
  }
  collect(path.join(moduleRoot, 'scopes'));
  return { entry, modules };
}

/**
 * One line for one diagnostic: its message and arguments, the daemon's own message and numeric code,
 * its source position, and the daemon's hint. A placeholder source such as `<authoring>` names no file,
 * so it is not printed as a position.
 */
export function diagnosticLine(diagnostic, locate = source => source) {
  const span = diagnostic.primary?.kind === 'Source' && !diagnostic.primary.span.source.startsWith('<')
    ? diagnostic.primary.span : null;
  const args = diagnostic.args?.filter(arg => arg !== null && arg !== undefined) ?? [];
  const daemon = diagnostic.protocol?.message && diagnostic.protocol.message !== diagnostic.message
    ? `: ${diagnostic.protocol.message}` : '';
  return `${diagnostic.message}${args.length ? ': ' + args.join('; ') : ''}${daemon}`
    + `${typeof diagnostic.code === 'number' && diagnostic.code !== 0 ? ` (code ${diagnostic.code})` : ''}`
    + `${span ? ` at ${locate(span.source)}:${span.startLine}:${span.startColumn}` : ''}`
    + `${typeof diagnostic.protocol?.hint === 'string' ? ` (hint: ${diagnostic.protocol.hint})` : ''}`;
}

/** A refusal carries its diagnostics, not only their text, so a reporter can still say where. */
export function refusal(diagnostics) {
  const error = new Error(diagnostics.map(diagnostic => diagnosticLine(diagnostic)).join('; '));
  error.diagnostics = diagnostics;
  return error;
}

export async function openAuthoringSession(state) {
  const transport = await connectOwnerLocal({ root: state, socketName: 'daemon.sock' });
  try { return await establish(transport, { hello: { requestedRoles: [1n, 4n, [2n, []]] }, resourceCeilings: ceilings, requestTimeoutMs: 5000 }); }
  catch (error) { await transport.close(); throw error; }
}

export async function executeProgram(session, program, snapshot) {
  if (snapshot.status !== 'accepted') throw new Error(`authoring snapshot refused (${snapshot.reason ?? snapshot.status}): ${[...(snapshot.diagnostics ?? []), snapshot.diagnostic].filter(Boolean).map(d => `${d.code}: ${d.message}`).join("; ")}`);
  const prepared = semanticPrepass(program);
  if (prepared.status !== 'complete') throw refusal(prepared.diagnostics);
  const result = await createCodeExecutionHost({ session }).execute(program, {
    targetScope: [], commitId: randomBytes(16),
    expectedRevision: snapshot.value.anchor.authoringRevision,
    expectedEnvironment: snapshot.value.anchor.environment,
    currentSnapshot: snapshot.value,
  });
  if (result.status !== 'committed') return result;
  return { ...result, adoption: await waitForAdoption(session, result.commit.cursor) };
}
