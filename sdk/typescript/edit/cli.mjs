import fs from 'node:fs';
import path from 'node:path';
import { runEditHarness } from './harness.mjs';
import { approveProgram, copyProgram, createEditSession, currentFile, rollbackProgram, sessionDirectory } from './workflow.mjs';
import { diagnosticLine, loadProgram } from '../chat/execution.mjs';
import { shellQuote } from '../chat/launcher.mjs';
import { EXIT, exitFor, failure } from '../cli/common.mjs';

export const usage = `usage: circular edit --state <dir> --harness <bound-name>
    [--template <name>] [--instruction <text>] [--harness-bin <absolute-path>]
    [--timeout-ms <milliseconds>] [--dry-run | --approve]
  circular edit --state <dir> --session <session-dir> [--program <relative.ts>] --approve
  circular edit --state <dir> --rollback <approved-current.ts> [--dry-run | --approve]

Harness names and executables come from the daemon agent.harnesses query.
An offline dry-run leaves harness binding unverified; it never launches a harness.
Templates supply task text only: agent-session-monitor, incident-autopilot.
--dry-run writes chat recovery and proposal files only. No harness runs or packages install.
Without --approve a harness proposal is saved for review; no program is executed.
--approve executes SDK code through the installed host. Rollback reexecutes preserved code
with the current revision anchor; it does not rewind the authoring journal.
Options come only from arguments, never environment variables.
`;

const flags = { '--state': { key: 'state', value: true }, '--harness': { key: 'harness', value: true },
  '--harness-bin': { key: 'harnessBin', value: true }, '--template': { key: 'template', value: true },
  '--instruction': { key: 'instruction', value: true }, '--timeout-ms': { key: 'timeoutMs', value: true },
  '--session': { key: 'session', value: true }, '--program': { key: 'program', value: true },
  '--rollback': { key: 'rollback', value: true }, '--approve': { key: 'approve' },
  '--dry-run': { key: 'dryRun' }, '--help': { key: 'help' }, '-h': { key: 'help' } };
const LONGEST_TIMER_MS = 2 ** 31 - 1;
const isOption = value => Object.hasOwn(flags, value);

export function parseArguments(argv) {
  const options = { approve: false, dryRun: false };
  const seen = new Set();
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (seen.has(arg)) throw new Error(`duplicate argument: ${arg}`);
    seen.add(arg);
    if (!isOption(arg)) throw new Error(`unknown argument: ${arg}`);
    const flag = flags[arg];
    if (flag.value) {
      const value = argv[++i];
      if (arg === '--instruction') {
        if (!value || isOption(value) || /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(value)) {
          throw new Error(`${arg} requires a value; line breaks and tabs are allowed, other control characters are not`);
        }
      } else if (!value || (arg === '--timeout-ms' ? isOption(value) : value.startsWith('-')) || /[\x00-\x1f\x7f]/.test(value)) {
        throw new Error(`${arg} requires a value without control characters`);
      }
      options[flag.key] = arg === '--timeout-ms' ? Number(value) : value;
    } else options[flag.key] = true;
  }
  if (options.help) return options;
  if (!options.state || !path.isAbsolute(options.state)) throw new Error('--state requires an absolute directory');
  if (options.approve && options.dryRun) throw new Error('--approve and --dry-run are mutually exclusive');
  if (options.session && options.rollback) throw new Error('--session and --rollback are mutually exclusive');
  if (options.program && !options.session) throw new Error('--program requires --session');
  if ((options.session || options.rollback) && (options.template || options.instruction || options.harness || options.harnessBin)) {
    throw new Error('saved code execution does not accept template, instruction, or harness options');
  }
  if (!options.session && !options.rollback && !options.harness) throw new Error('--harness must name a binding reported by agent.harnesses');
  if (options.harnessBin && !path.isAbsolute(options.harnessBin)) throw new Error('--harness-bin requires an absolute path');
  if (options.timeoutMs !== undefined && (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1 || options.timeoutMs > LONGEST_TIMER_MS)) {
    throw Object.assign(new Error(`--timeout-ms is a whole number of milliseconds from 1 to ${LONGEST_TIMER_MS}, the longest timer Node keeps`), { code: 'EDIT_TIMEOUT_RANGE' });
  }
  return options;
}

/** One denied harness call: our code, the harness's own reason as it gave it, the tool and its target. */
const deniedLine = harness => call => `circular edit: ${call.code} (${call.reason ?? 'no reason given'}): ${harness} ${call.tool}: ${call.summary}\n`;

export async function main(argv, io = process, dependencies = {}) {
  let options;
  try { options = parseArguments(argv); }
  catch (error) { io.stderr.write(`circular edit: ${error.code ? `${error.code}: ` : ''}${error.message}\n${usage}`); return EXIT.USAGE; }
  if (options.help) { io.stdout.write(usage); return EXIT.OK; }
  let directory, file, current, turn;
  try {
    if (options.session) {
      directory = sessionDirectory(options.state, options.session);
      current = currentFile(directory);
      file = path.resolve(directory, options.program ?? 'proposal.ts');
      loadProgram(file, directory);
    } else {
      const rollback = options.rollback ? rollbackProgram(options.state, options.rollback) : null;
      const created = await (dependencies.createEditSession ?? createEditSession)({ ...options,
        harness: options.harness ?? 'codex', dryRun: options.dryRun || Boolean(rollback) });
      ({ directory, current } = created);
      file = created.proposal;
      if (rollback) {
        const source = loadProgram(rollback);
        file = copyProgram(source, path.join(directory, 'rollback'), 'current.ts');
      } else if (!options.dryRun) {
        turn = await (dependencies.runEditHarness ?? runEditHarness)({ harness: options.harness,
          program: options.harnessBin ?? created.harnessProgram,
          prompt: fs.readFileSync(path.join(directory, 'first-message.md'), 'utf8'), cwd: directory, timeoutMs: options.timeoutMs,
          onEvent: event => io.stderr.write(`circular edit: ${event.harness} ${event.type}: ${event.summary}\n`), proposal: file });
      }
    }
    const denied = turn?.denied ?? [];
    io.stderr.write(denied.map(deniedLine(options.harness)).join('')
      + (turn?.unreadable > 0 ? `circular edit: ${options.harness} wrote ${turn.unreadable} output lines this command could not read\n` : ''));
    if (options.dryRun || !options.approve) {
      io.stdout.write(`${JSON.stringify({ committed: false, session: directory, current, program: file, ...(turn ? { denied, unreadable: turn.unreadable ?? 0 } : {}) })}\n`);
      if (!options.dryRun) io.stderr.write(`circular edit: ready — review ${shellQuote(path.relative(directory, file))}${turn?.apply ? `, then approve it with: ${turn.apply}` : ''}\n`);
      return EXIT.OK;
    }
    const result = await (dependencies.approveProgram ?? approveProgram)({ state: options.state, directory, file, approve: true });
    io.stdout.write(`${JSON.stringify({ ...result, session: directory, rollback: Boolean(options.rollback) })}\n`);
    return EXIT.OK;
  } catch (error) {
    if (error.commitUnknown) return failure(io, 'circular edit', error);
    const head = `circular edit: ${error.commitAccepted
      ? error.adoption ? 'commit accepted; adoption not recorded as successful' : 'commit accepted; history finalization failed'
      : 'refused'} — `;
    const code = error.code ?? (error.commitAccepted ? 'HISTORY_FINALIZATION_FAILED' : null);
    if (error.commitAccepted) io.stdout.write(`${JSON.stringify({ committed: true,
      code, current: error.current,
      previous: error.previous, program: error.program, session: directory,
      rollback: Boolean(options.rollback) })}\n`);
    const locate = source => source.startsWith('<') || !file ? source
      : path.relative(directory ?? process.cwd(), path.resolve(path.dirname(file), source));
    const diagnostics = error.diagnostics ?? error.circularDiagnostics;
    const lines = diagnostics?.length ? diagnostics.map(diagnostic => diagnosticLine(diagnostic, locate))
      : [code && !error.message.startsWith(`${code}:`) ? `${code}: ${error.message}` : error.message];
    io.stderr.write(lines.map(line => `${head}${line}\n`).join(''));
    return exitFor(error);
  }
}
