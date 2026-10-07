import path from 'node:path';
import fs from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { connect } from './frames.mjs';
import { noDaemon } from '../renderer/reasons.mjs';

const fail = code => Object.assign(new Error(code), { code });

export const installedCli = executable => path.resolve(path.dirname(executable), '../../..', 'circular');

export function runCli(program, args) {
  return new Promise((resolve, reject) => execFile(program, args, (error, stdout, stderr) => {
    if (error && typeof error.code !== 'number') return reject(fail('CLI_UNAVAILABLE'));
    resolve({ code: error?.code ?? 0, stdout, stderr });
  }));
}

async function daemonStatus(run, cli, state) {
  const result = await run(cli, ['daemon', 'status', '--state', state, '--json']);
  let value;
  try { value = JSON.parse(result.stdout); } catch { throw fail('CLI_STATUS_UNAVAILABLE'); }
  if (![0, 1].includes(result.code) || typeof value?.running !== 'boolean' ||
      typeof value.answering !== 'boolean' || value.state !== state) throw fail('CLI_STATUS_UNAVAILABLE');
  return value;
}

export async function attachState({ state, cli, run = runCli, open = connect }) {
  if (!state) throw fail('STATE_REQUIRED');
  try { return await open({ state }); }
  catch (error) {
    if (!noDaemon.has(error?.code)) throw error;
    let presence;
    try { presence = await daemonStatus(run, cli, state); }
    catch (probe) { throw probe?.code === 'CLI_UNAVAILABLE' ? error : probe; }
    if (presence.running && !presence.answering) throw fail('DAEMON_NOT_ANSWERING');
    throw error;
  }
}

export async function startDaemon({ state, cli, run = runCli, open = connect }) {
  if (!state) throw fail('STATE_REQUIRED');
  const started = await run(cli, ['daemon', 'start', '--state', state, '--json']);
  if (started.code === 0) return attachState({ state, cli, run, open });
  let value;
  try { value = JSON.parse(started.stdout); } catch { value = undefined; }
  if (value?.state !== state || value.started !== false || typeof value.reason !== 'string')
    throw Object.assign(fail('CLI_STATUS_UNAVAILABLE'), { evidence: { stderr: started.stderr || null } });
  throw Object.assign(fail('CLI_START_FAILED'),
    { evidence: { reason: value.reason, logTail: value.logTail ?? null, log: value.log ?? null } });
}

export async function stopDaemon({ state, cli, run = runCli }) {
  if (!state) throw fail('STATE_REQUIRED');
  const result = await run(cli, ['daemon', 'stop', '--state', state, '--json']);
  let value;
  try { value = JSON.parse(result.stdout); } catch { value = undefined; }
  if (value?.state !== state || typeof value.stopped !== 'boolean')
    throw Object.assign(fail('CLI_STATUS_UNAVAILABLE'), { evidence: { stderr: result.stderr || null } });
  if (result.code !== 0) throw Object.assign(fail('CLI_STOP_FAILED'), { evidence: { reason: value.reason ?? null } });
  return { stopped: value.stopped };
}

export const RECENT_FILE = 'ui.json';
export const RECENT_LIMIT = 32;
const VERSION = 1;
async function readPreferences(file) {
  let text;
  try { text = await fs.readFile(file, 'utf8'); }
  catch (error) { if (error.code === 'ENOENT') return { version: VERSION }; throw fail('RECENT_READ_FAILED'); }
  let value;
  try { value = JSON.parse(text); } catch { throw fail('RECENT_INVALID'); }
  const paths = value?.recent_state_locations ?? [];
  if (!value || typeof value !== 'object' || Array.isArray(value) || value.version !== VERSION ||
      !Array.isArray(paths) || paths.some(p => typeof p !== 'string')) throw fail('RECENT_INVALID');
  return value;
}
export async function readRecent(file) {
  return ((await readPreferences(file)).recent_state_locations ?? []).slice(0, RECENT_LIMIT);
}
export async function rememberState(file, state) {
  if (!state) throw fail('STATE_REQUIRED');
  const preferences = await readPreferences(file);
  const paths = [state, ...(preferences.recent_state_locations ?? []).filter(p => p !== state)].slice(0, RECENT_LIMIT);
  try {
    await fs.mkdir(path.dirname(file), { recursive: true });
    const staging = `${file}.tmp`;
    await fs.writeFile(staging, JSON.stringify({ ...preferences, recent_state_locations: paths }, null, 2) + '\n');
    await fs.rename(staging, file);
  } catch { throw fail('RECENT_WRITE_FAILED'); }
  return paths;
}

export async function chooseState(dialog) {
  const result = await dialog.showOpenDialog({ title: 'Open Project', message: 'Choose a project folder',
    buttonLabel: 'Open', properties: ['openDirectory'] });
  return result.canceled ? undefined : result.filePaths[0];
}

export async function createState(dialog, { recent = [], home, mkdir = fs.mkdir } = {}) {
  const near = recent.length ? path.dirname(recent[0]) : home;
  const result = await dialog.showSaveDialog({ title: 'New Project', message: 'Name the project and choose where its folder goes',
    buttonLabel: 'Create', nameFieldLabel: 'Project:', properties: ['createDirectory'],
    ...(near ? { defaultPath: path.join(near, 'Untitled') } : {}) });
  if (result.canceled || !result.filePath) return undefined;
  try { await mkdir(result.filePath, { mode: 0o700 }); }
  catch (error) { throw fail(error?.code === 'EEXIST' ? 'PROJECT_EXISTS' : 'PROJECT_CREATE_FAILED'); }
  return result.filePath;
}
