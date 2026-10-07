import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';
import { EXIT, TargetError, UsageError, claimPath, cliIdentity, parseFlags, programAbsent, requireState, siblingProgram, socketPath, writeJson } from './common.mjs';
import { connect, plain, query, socketPresent, withSession } from './session.mjs';

export const DAEMON_PROGRAM = 'circular-daemon';

export const usage = `usage: circular daemon start --state <dir> [--json]
       circular daemon stop --state <dir> [--json]
       circular daemon restart --state <dir> [--json]
       circular daemon status --state <dir> [--json]
       circular daemon logs --state <dir> [--follow] [--lines <n>]
       circular daemon install --state <dir>
       circular daemon uninstall --state <dir>

start   launches ${DAEMON_PROGRAM} for this state, detached, with stdout and stderr appended to
        the operating log below. A daemon that already answers is reported and left alone.
        It answers once the daemon answers on its socket or its process ends, however long
        the daemon's recovery takes; a refusal carries the operating log's last lines.
stop    sends SIGTERM to the process holding this state's claim file and waits for that process
        to actually leave. A daemon that is still there when the wait ends is reported, not
        assumed gone. Stopping the process is not pausing a pipeline: use Pause for that.
restart is stop followed by start.
status  separates running (a process holds the claim) from answering (daemon.health came back).
logs    tails the daemon's own operating log. It is not the arrival journal: recorded arrivals
        are read with the arrival queries, never from this file.
install writes the per-user LaunchAgent that starts circular-daemon for this state; uninstall
        removes it. Neither starts or stops a process, and neither touches state data. Both
        exit with the registration executable's own code (0 done, 2 fix the request, 3 fix or
        create the location).

The operating log is ~/Library/Logs/Circular/direct/<digest>.log — outside the state directory,
at the same destination the desktop application uses for this state.
Options come only from arguments; no environment variable supplies one.
`;

const ACTIONS = ['start', 'stop', 'restart', 'status', 'logs', 'install', 'uninstall'];

export function operatingLogPath(state, home = os.userInfo().homedir) {
  const digest = createHash('sha256').update(Buffer.from(state, 'utf8')).digest('hex').slice(0, 16);
  return path.join(home, 'Library', 'Logs', 'Circular', 'direct', `${digest}.log`);
}

export function holdingPid(state, run = spawnSync) {
  const claim = claimPath(state);
  if (!fs.existsSync(claim)) return null;
  const result = run('/usr/sbin/lsof', ['-t', '--', claim], { encoding: 'utf8', timeout: 5000 });
  if (result.error || typeof result.stdout !== 'string') return null;
  const pids = result.stdout.split('\n').map(line => Number(line.trim())).filter(Number.isSafeInteger).filter(pid => pid > 0);
  return pids.length ? pids[0] : null;
}

const alive = pid => { try { process.kill(pid, 0); return true; } catch (error) { return error.code === 'EPERM'; } };
const sleep = ms => new Promise(resolve => { setTimeout(resolve, ms); });

export async function healthOf(state, transport) {
  if (!socketPresent(state)) return { answering: false, diagnostic: 'no socket file at this state' };
  try {
    return await withSession(state, async session => {
      const health = plain(await query(session, 'daemon.health'));
      return { answering: true, health };
    }, { transport });
  } catch (error) {
    return { answering: false, diagnostic: error.message };
  }
}

export function installedDaemon(launcher) {
  const found = siblingProgram(DAEMON_PROGRAM, launcher);
  if (found.path === null) return { program: null, version: null, source: found.source };
  const result = spawnSync(found.path, ['--version'], { encoding: 'utf8', timeout: 5000 });
  const version = result.status === 0 && typeof result.stdout === 'string' ? result.stdout.split('\n')[0].trim() : null;
  return { program: found.path, version, source: found.source };
}

async function statusValue(state, launcher) {
  const pid = holdingPid(state);
  const reachable = await healthOf(state);
  const installed = installedDaemon(launcher);
  const anchor = reachable.answering ? reachable.health?.anchor ?? null : null;
  return {
    state,
    running: pid !== null,
    pid,
    answering: reachable.answering,
    socket: socketPath(state),
    log: operatingLogPath(state),
    installedDaemon: installed.program,
    installedDaemonVersion: installed.version,
    lifecycle: reachable.answering ? (anchor?.lifecycle ?? null) : null,
    storage: reachable.answering ? (anchor?.storage ?? null) : null,
    health: reachable.answering ? (reachable.health?.items ?? []) : null,
    diagnostic: reachable.answering ? null : reachable.diagnostic,
  };
}

function logTail(file, lines) {
  if (!fs.existsSync(file)) return null;
  const text = fs.readFileSync(file, 'utf8');
  const split = text.split('\n');
  const start = Math.max(0, (split.at(-1) === '' ? split.length - 1 : split.length) - lines);
  return split.slice(start).join('\n');
}

async function start(state, options, io, launcher) {
  const before = await statusValue(state, launcher);
  if (before.answering) {
    if (options.json) writeJson(io, { ...before, started: false, reason: 'already answering' });
    else io.stdout.write(`daemon already answering for ${state} (pid ${before.pid ?? 'unknown'}); socket ${before.socket}\n`);
    return EXIT.OK;
  }
  const installed = installedDaemon(launcher);
  if (installed.program === null) throw new TargetError(programAbsent(DAEMON_PROGRAM, launcher));
  const log = operatingLogPath(state);
  fs.mkdirSync(path.dirname(log), { recursive: true, mode: 0o700 });
  const handle = fs.openSync(log, 'a', 0o600);
  const child = spawn(installed.program, ['--state', state], { detached: true, stdio: ['ignore', handle, handle] });
  child.unref();
  fs.closeSync(handle);
  let ended = null;
  const end = new Promise(resolve => {
    const settle = value => { ended ??= value; resolve(ended); };
    child.once('exit', (code, signal) => settle({ code, signal }));
    child.once('error', error => settle({ error }));
  });
  let answer = null;
  for (;;) {
    const transport = await connect(state).catch(() => null);
    if (transport !== null) { answer = await healthOf(state, transport); break; }
    if (ended !== null) break;
    await Promise.race([end, sleep(50)]);
  }
  if (answer?.answering) {
    const after = await statusValue(state, launcher);
    if (options.json) writeJson(io, { ...after, started: true, reason: null });
    else io.stdout.write(`daemon started for ${state} (pid ${after.pid ?? child.pid}); socket ${after.socket}; log ${log}\n`);
    return EXIT.OK;
  }
  const tail = logTail(log, 20);
  const reason = answer !== null
    ? `the daemon published its socket and did not answer: ${answer.diagnostic}`
    : ended.error !== undefined
      ? `could not execute ${installed.program}: ${ended.error.message}`
      : ended.code === 4
        ? 'another instance already holds this state'
        : `the daemon did not publish its socket (${ended.code === null ? `signal ${ended.signal}` : `exit ${ended.code}`})`;
  if (options.json) writeJson(io, { ...await statusValue(state, launcher), started: false, reason, logTail: tail });
  else io.stderr.write(`circular daemon start: ${reason}; log ${log}\n${tail ?? ''}\n`);
  return EXIT.FAILED;
}

async function stop(state, options, io, launcher) {
  const pid = holdingPid(state);
  if (pid === null) {
    const now = await statusValue(state, launcher);
    if (options.json) writeJson(io, { ...now, stopped: false, reason: 'no process holds this state' });
    else io.stdout.write(`no daemon holds ${state}\n`);
    return EXIT.OK;
  }
  try { process.kill(pid, 'SIGTERM'); }
  catch (error) { throw new TargetError(`could not signal pid ${pid}: ${error.message}`); }
  for (let attempt = 0; attempt < 300; attempt += 1) {
    if (!alive(pid) && holdingPid(state) === null) {
      const now = await statusValue(state, launcher);
      if (options.json) writeJson(io, { ...now, stopped: true, reason: null, pid });
      else io.stdout.write(`daemon ${pid} left ${state}\n`);
      return EXIT.OK;
    }
    await sleep(100);
  }
  const now = await statusValue(state, launcher);
  if (options.json) writeJson(io, { ...now, stopped: false, reason: `pid ${pid} still holds ${claimPath(state)}` });
  else io.stderr.write(`circular daemon stop: pid ${pid} still holds ${claimPath(state)}\n`);
  return EXIT.FAILED;
}

async function logs(state, options, io) {
  const file = operatingLogPath(state);
  const lines = options.lines === undefined ? 200 : Number(options.lines);
  if (!Number.isSafeInteger(lines) || lines <= 0) throw new UsageError('--lines must be a positive integer');
  if (!fs.existsSync(file)) {
    io.stderr.write(`circular daemon logs: no operating log at ${file}; a daemon started by launchd writes to its LaunchAgent log instead\n`);
    return EXIT.FAILED;
  }
  io.stdout.write(`${logTail(file, lines)}`);
  if (!options.follow) return EXIT.OK;
  let position = fs.statSync(file).size;
  for (;;) {
    await sleep(250);
    const size = fs.statSync(file).size;
    if (size < position) position = 0;
    if (size === position) continue;
    const handle = fs.openSync(file, 'r');
    const buffer = Buffer.alloc(size - position);
    fs.readSync(handle, buffer, 0, buffer.length, position);
    fs.closeSync(handle);
    position = size;
    io.stdout.write(buffer.toString('utf8'));
  }
}

function registration(action, state, io, launcher) {
  const installed = siblingProgram(DAEMON_PROGRAM, launcher);
  if (installed.path === null) throw new TargetError(programAbsent(DAEMON_PROGRAM, launcher));
  const result = spawnSync(installed.path, [action, '--state', state], { stdio: 'inherit' });
  if (result.error) throw new TargetError(`cannot execute ${installed.path}: ${result.error.message}`);
  return result.status ?? EXIT.FAILED;
}

export async function main(argv, io = process, launcher = process.argv[1]) {
  let options;
  try {
    options = parseFlags(argv, {
      values: { '--state': 'state', '--lines': 'lines' },
      flags: { '--json': 'json', '--follow': 'follow', '-f': 'follow' },
    });
    if (options.help) { io.stdout.write(usage); return EXIT.OK; }
    if (options.positional.length !== 1 || !ACTIONS.includes(options.positional[0])) {
      throw new UsageError(`one of ${ACTIONS.join('|')} is required`);
    }
  } catch (error) {
    io.stderr.write(`circular daemon: ${error.message}\n${usage}`);
    return EXIT.USAGE;
  }
  const action = options.positional[0];
  try {
    if (action === 'install' || action === 'uninstall') {
      return registration(action, requireState(options, { mustExist: false }), io, launcher);
    }
    const state = requireState(options);
    if (action === 'logs') return await logs(state, options, io);
    if (action === 'status') {
      const value = await statusValue(state, launcher);
      if (options.json) writeJson(io, value);
      else {
        const identity = await cliIdentity();
        io.stdout.write([
          `state       ${value.state}`,
          `running     ${value.running ? `yes (pid ${value.pid})` : 'no'}`,
          `answering   ${value.answering ? 'yes' : `no (${value.diagnostic})`}`,
          `socket      ${value.socket}`,
          `cli         ${identity.version} (${identity.commit})`,
          `daemon      ${value.installedDaemonVersion ?? 'not installed'} (installed binary${value.installedDaemon ? ` ${value.installedDaemon}` : ''})`,
          `lifecycle   ${value.answering ? (value.lifecycle ?? 'none') : 'unknown'}`,
          `health      ${value.answering ? `${value.health.length} actor row(s)` : 'unknown'}`,
          `recorder    ${value.answering ? (value.storage === null ? 'standing' : `stopped (code ${value.storage})`) : 'unknown'}`,
          `log         ${value.log}`,
        ].join('\n') + '\n');
      }
      return value.answering ? EXIT.OK : EXIT.FAILED;
    }
    if (action === 'start') return await start(state, options, io, launcher);
    if (action === 'stop') return await stop(state, options, io, launcher);
    const stopped = await stop(state, options, io, launcher);
    if (stopped !== EXIT.OK) return stopped;
    return await start(state, options, io, launcher);
  } catch (error) {
    if (error instanceof UsageError) { io.stderr.write(`circular daemon: ${error.message}\n${usage}`); return EXIT.USAGE; }
    io.stderr.write(`circular daemon: ${error.message}\n`);
    return EXIT.FAILED;
  }
}
