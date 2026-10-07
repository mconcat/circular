import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { EXIT, UsageError, cliIdentity, packageRoot, parseFlags, programRemedy, requireState, siblingProgram, socketPath, writeJson } from './common.mjs';
import { healthOf, holdingPid, installedDaemon, operatingLogPath } from './daemon.mjs';
import { HARNESSES, HARNESS_NAMES, installedProgram } from '../chat/harnesses.mjs';
import { readChatDefaults } from '../chat/defaults.mjs';

export const usage = `usage: circular doctor [--state <dir>] [--json]

Reports what is installed, what this machine can reach, and what is missing. It changes
nothing: it installs no package, starts no process, and writes no settings file.

Each row is a stable code and one measured line. A row that names something to fix also
carries a remedy; the remedy is null otherwise. Status is ok, failed, or unknown; unknown
means the fact could not be measured here, never that it passed.
Without --state only the installation-wide rows are reported.

Exit code is 0 when no row failed, 1 when one did, 2 for a usage error.
`;

export const INSTALLED_PROGRAMS = Object.freeze(['circular-daemon', 'circular']);

const row = (code, status, detail, remedy = null) => ({ code, status, detail, remedy });

const ACTOR_TROUBLE = new Set(['backpressure', 'failed']);

export function actorsRow(items) {
  const trouble = items.filter(item => ACTOR_TROUBLE.has(item.state));
  if (trouble.length === 0) {
    return row('daemon.actors', 'ok', `no actor reports backpressure or failure (${items.length} actor row(s) read)`);
  }
  const named = trouble.map(item => `${item.actor?.local ?? '?'} ${item.state}${item.reason ? ` ${item.reason}` : ''}`);
  return row('daemon.actors', 'failed', `${trouble.length} of ${items.length} actor row(s): ${named.join(', ')}`,
    'read actor.events for those actors; a reason code names what to fix');
}

export async function diagnose({ state = null, launcher = process.argv[1], searchPath = process.env.PATH ?? '' } = {}) {
  const rows = [];
  const identity = await cliIdentity();
  rows.push(row('install.cli', 'ok', `circular ${identity.version} (${identity.commit}) at ${packageRoot}`));

  const major = Number(process.versions.node.split('.')[0]);
  rows.push(major >= 22
    ? row('runtime.node', 'ok', `node ${process.versions.node} at ${process.execPath}`)
    : row('runtime.node', 'failed', `node ${process.versions.node} is older than 22`, 'install Node.js 22 or newer and put it on PATH'));

  for (const name of INSTALLED_PROGRAMS) {
    const found = siblingProgram(name, launcher);
    rows.push(found.path === null
      ? row(`install.program.${name}`, 'failed', `${name} is neither beside this command nor on PATH`, programRemedy(name))
      : row(`install.program.${name}`, 'ok', `${found.path} (${found.source})`));
  }
  const daemon = installedDaemon(launcher);
  rows.push(daemon.version === null
    ? row('install.daemon.version', 'unknown', 'the installed circular-daemon did not answer --version',
      'run circular-daemon --version yourself and read its diagnostic')
    : row('install.daemon.version', 'ok', daemon.version));

  const present = HARNESS_NAMES.map(name => ({ name, program: installedProgram(name, searchPath) }));
  const installedCli = present.filter(entry => entry.program !== null);
  rows.push(installedCli.length === 0
    ? row('agent-cli.available', 'failed', `none of ${HARNESS_NAMES.join(', ')} is on PATH`,
      'install one of those agent CLIs and log into it yourself; Circular never holds its credentials')
    : row('agent-cli.available', 'ok',
      `installed: ${installedCli.map(entry => entry.name).join(', ')}`
      + `; not on PATH: ${present.filter(entry => entry.program === null).map(entry => entry.name).join(', ') || 'none'}`));
  for (const entry of installedCli) {
    const help = spawnSync(entry.program, ['--help'], { encoding: 'utf8', timeout: 5000 });
    const patterns = HARNESSES[entry.name].helpPatterns;
    const verified = patterns !== null && !help.error && help.status === 0 && patterns.every(pattern => pattern.test(help.stdout ?? ''));
    rows.push(verified
      ? row(`agent-cli.${entry.name}`, 'ok', `${entry.program}; --help matches the pinned interactive spelling`)
      : row(`agent-cli.${entry.name}`, 'unknown', `${entry.program}; --help did not match the pinned spelling, so chat falls back to manual loading`,
        'read the session first-message.md and load the instruction file with your CLI documentation'));
  }
  rows.push(row('agent-cli.login', 'unknown', 'this command runs no agent CLI, so no login state is measured here',
    `run ${installedCli[0]?.name ?? 'your agent CLI'} yourself once and confirm it starts without asking you to log in`));

  if (state === null) {
    rows.push(row('state', 'unknown', 'no --state was given, so nothing about a state directory was measured',
      'pass --state <absolute directory> to measure a state'));
    return rows;
  }

  rows.push(row('state.directory', 'ok', `${state} is an owned 0700 directory below your home`));

  const config = path.join(state, 'config.toml');
  if (!fs.existsSync(config)) {
    rows.push(row('state.config', 'ok',
      `${config} is absent; the daemon stands on its defaults, and daemon.health names each one`));
  } else {
    const mode = fs.statSync(config).mode & 0o7777;
    rows.push((mode & 0o077) === 0
      ? row('state.config', 'ok', `${config} mode ${mode.toString(8).padStart(4, '0')}`)
      : row('state.config', 'failed', `${config} mode ${mode.toString(8).padStart(4, '0')} grants group or other bits`,
        `chmod 600 ${config}`));
  }

  const pid = holdingPid(state);
  rows.push(pid === null
    ? row('daemon.process', 'failed', `no process holds ${state}`, `circular daemon start --state ${state}`)
    : row('daemon.process', 'ok', `pid ${pid} holds this state's claim file`));

  const reachable = await healthOf(state);
  if (reachable.answering) {
    rows.push(row('daemon.socket', 'ok', `${socketPath(state)} answered daemon.health`));
    const anchor = reachable.health?.anchor ?? null;
    rows.push(row('daemon.config', 'ok', 'the running daemon accepted this state\'s configuration; it refuses to start on a wrong value'));
    const defaults = anchor?.config_defaults;
    rows.push(!Array.isArray(defaults)
      ? row('daemon.config_defaults', 'unknown', 'daemon.health carried no config_defaults, so the defaults in force were not measured',
        'install the circular-daemon that ships beside this command')
      : row('daemon.config_defaults', 'ok', defaults.length === 0
        ? 'none; config.toml sets every operating value'
        : `in force: ${defaults.map(entry => `${entry.key}=${entry.value}`).join(', ')}`));
    rows.push(row('daemon.lifecycle', 'ok', `lifecycle ${anchor?.lifecycle ?? 'none'}`));
    rows.push(anchor?.storage === null || anchor?.storage === undefined
      ? row('daemon.recorder', 'ok', 'this daemon\'s arrival recorder stands')
      : row('daemon.recorder', 'failed', `the arrival recorder stopped with code ${anchor.storage}`,
        'read the operating log and the dead.letters query before restarting'));
    rows.push(actorsRow(reachable.health?.items ?? []));
  } else {
    rows.push(row('daemon.socket', 'failed', `${socketPath(state)} did not answer: ${reachable.diagnostic}`,
      `circular daemon start --state ${state}, then read circular daemon logs --state ${state}`));
    rows.push(row('daemon.config', 'unknown', 'the daemon did not answer, so its acceptance of this state\'s configuration was not measured',
      `circular daemon start --state ${state} prints ConfigRejected naming the key whose value it refuses`));
  }

  const log = operatingLogPath(state);
  rows.push(fs.existsSync(log)
    ? row('daemon.log', 'ok', `${log} (${fs.statSync(log).size} bytes)`)
    : row('daemon.log', 'unknown', `${log} does not exist yet`,
      'a daemon started by launchd writes to its LaunchAgent log instead of this file'));

  const stored = readChatDefaults(state);
  if (stored.diagnostic !== null) {
    rows.push(row('chat.default', 'failed', `${stored.path}: ${stored.diagnostic}`,
      'delete that file and run circular chat with an agent-CLI flag to write it again'));
  } else if (stored.agentCli === null) {
    rows.push(row('chat.default', 'unknown', 'this state has no saved default agent CLI yet',
      'run circular chat with an agent-CLI flag once; the choice is saved for next time'));
  } else {
    const program = installedProgram(stored.agentCli, searchPath);
    rows.push(program === null
      ? row('chat.default', 'failed', `the saved default ${stored.agentCli} is no longer on PATH`,
        'run circular chat with an agent-CLI flag to choose again')
      : row('chat.default', 'ok', `${stored.agentCli} (${program})`));
  }
  return rows;
}

export async function main(argv, io = process, launcher = process.argv[1]) {
  let options;
  try {
    options = parseFlags(argv, { values: { '--state': 'state' }, flags: { '--json': 'json' } });
    if (options.help) { io.stdout.write(usage); return EXIT.OK; }
    if (options.positional.length) throw new UsageError('doctor takes no positional argument');
  } catch (error) {
    io.stderr.write(`circular doctor: ${error.message}\n${usage}`);
    return EXIT.USAGE;
  }
  let state = null;
  try { state = options.state === undefined ? null : requireState(options); }
  catch (error) {
    if (error instanceof UsageError) { io.stderr.write(`circular doctor: ${error.message}\n${usage}`); return EXIT.USAGE; }
    const rows = [row('state.directory', 'failed', error.message, 'create the directory with mode 0700 below your home')];
    if (options.json) writeJson(io, { state: options.state, rows });
    else io.stdout.write(`failed   state.directory   ${error.message}\n         remedy: ${rows[0].remedy}\n`);
    return EXIT.FAILED;
  }
  const rows = await diagnose({ state, launcher });
  if (options.json) writeJson(io, { state, rows });
  else {
    for (const entry of rows) {
      io.stdout.write(`${entry.status.padEnd(8)} ${entry.code.padEnd(28)} ${entry.detail}\n`);
      if (entry.status !== 'ok' && entry.remedy) io.stdout.write(`${' '.repeat(9)}remedy: ${entry.remedy}\n`);
    }
  }
  return rows.some(entry => entry.status === 'failed') ? EXIT.FAILED : EXIT.OK;
}
