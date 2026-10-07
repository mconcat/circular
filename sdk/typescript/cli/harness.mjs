import path from 'node:path';
import { EXIT, UsageError, failure, parseFlags, requireState, writeJson } from './common.mjs';
import { query, withSession } from './session.mjs';

export const usage = `usage: circular harness list --state <dir> [--json]
       circular harness bind <name> --program <absolute-path> --state <dir> [--json]
       circular harness unbind <name> --state <dir> [--json]

list    reports agent.harnesses: for each binding name, the program the standing pipeline
        holds (program) and the program saved in <state>/config.toml (saved); either can
        be none. It also reports agent.harness-candidates: for each harness the daemon has
        an adapter for, the first of that adapter's install locations where the daemon
        found a program it would accept (found), or none. Use a reported binding name
        verbatim in agent({ harness: ... }).
bind    sends one SetAgentHarness command. The daemon checks the name and the absolute
        program path, writes that name's entry in <state>/config.toml [agent] harnesses,
        and hands the binding to the standing pipeline. Nothing is written to the journal.
        The name and the path are yours to supply; neither is guessed and neither is
        substituted with the reference test executor.
unbind  sends the same command with no program: the daemon removes that name's entry.

When the command's answer does not arrive, bind and unbind read agent.harnesses: a saved
program that shows the request exits 0; otherwise the command exits 4, and the same
command can be called again. Both are idempotent.

A harness is a pipeline binding. The agent CLI that circular chat opens for you is a
different thing and is selected with chat's own flags.
`;

const ROLES = [1n, 4n];
/** The session's codes for a request that may have reached the daemon and was not answered. */
const UNANSWERED = new Set(['SESSION_CLOSED', 'REQUEST_TIMED_OUT', 'TRANSPORT_FAILED']);

export async function listBindings(state) {
  const { agentHarnesses, agentHarnessesPageFromValue } = await import('@circular/client');
  return withSession(state, async session => agentHarnessesPageFromValue(await query(session, agentHarnesses.name))
    .items.map(({ name, program, saved }) => ({ name, program, saved })));
}

export async function listCandidates(state) {
  const { agentHarnessCandidates, agentHarnessCandidatesPageFromValue } = await import('@circular/client');
  return withSession(state, async session => agentHarnessCandidatesPageFromValue(await query(session, agentHarnessCandidates.name))
    .items.map(({ name, found }) => ({ name, found })));
}

export async function setBinding(state, name, program) {
  const { setAgentHarness } = await import('@circular/client');
  return withSession(state, async session => {
    try { return await setAgentHarness(session, { name, program }); }
    catch (error) {
      if (!UNANSWERED.has(error?.code)) throw error;
      throw Object.assign(new Error(`SetAgentHarness was sent but its answer did not arrive (${error.message}). `
        + 'Whether the daemon saved it is not known; it was not refused.'), { answerLost: true, cause: error });
    }
  }, { roles: ROLES });
}

const shown = value => value ?? 'none';

export async function main(argv, io = process) {
  let options, verb;
  try {
    options = parseFlags(argv, { values: { '--state': 'state', '--program': 'program' }, flags: { '--json': 'json' } });
    if (options.help) { io.stdout.write(usage); return EXIT.OK; }
    verb = options.positional[0];
    if (!['list', 'bind', 'unbind'].includes(verb)) throw new UsageError('one of list|bind|unbind is required');
    if (verb === 'list') {
      if (options.positional.length !== 1) throw new UsageError('list takes no positional argument');
    } else {
      if (options.positional.length !== 2) throw new UsageError(`${verb} takes exactly one name`);
      if (verb === 'bind' && (!options.program || !path.isAbsolute(options.program))) throw new UsageError('--program requires an absolute path');
      if (verb === 'unbind' && options.program !== undefined) throw new UsageError('unbind takes no --program');
    }
  } catch (error) {
    io.stderr.write(`circular harness: ${error.message}\n${usage}`);
    return EXIT.USAGE;
  }
  try {
    const state = requireState(options);
    if (verb === 'list') {
      const bindings = await listBindings(state);
      const candidates = await listCandidates(state);
      if (options.json) writeJson(io, { state, bindings, candidates });
      else {
        io.stdout.write(bindings.length === 0 ? 'no harness is bound in this state\n'
          : bindings.map(b => `${b.name}\tprogram ${shown(b.program)}\tsaved ${shown(b.saved)}\n`).join(''));
        io.stdout.write(candidates.map(c => `${c.name}\tfound ${shown(c.found)}\n`).join(''));
      }
      return EXIT.OK;
    }
    const name = options.positional[1], program = verb === 'bind' ? options.program : null;
    const done = verb === 'bind'
      ? { json: { state, bound: name, program }, text: `bound ${name} to ${program}` }
      : { json: { state, unbound: name }, text: `unbound ${name}` };
    let answer;
    try {
      answer = await setBinding(state, name, program);
    } catch (error) {
      if (!error?.answerLost) throw error;
      const bindings = await listBindings(state).catch(() => null);
      const saved = bindings?.find(binding => binding.name === name)?.saved ?? null;
      if (bindings === null || saved !== program) {
        io.stderr.write(`circular harness: ${error.message} agent.harnesses ${bindings === null ? 'could not be read'
          : `does not show ${verb === 'bind' ? `${name} saved as ${program}` : `${name} removed`} yet`}; `
          + `check with: circular harness list --state ${state}. Calling the same ${verb} again is safe: ${verb} is idempotent.\n`);
        return EXIT.WAIT;
      }
      if (options.json) writeJson(io, { ...done.json, confirmedBy: 'agent.harnesses' });
      else io.stdout.write(`${done.text} (the answer was lost; agent.harnesses shows it saved)\n`);
      return EXIT.OK;
    }
    if (answer[0] !== 1n) {
      const { code, message, hint } = answer[1];
      io.stderr.write(`circular harness: SetAgentHarness refused (${code}): ${message}${hint ? ` (${hint})` : ''}\n`);
      return EXIT.FAILED;
    }
    if (options.json) writeJson(io, done.json);
    else io.stdout.write(`${done.text}\n`);
    return EXIT.OK;
  } catch (error) {
    if (error instanceof UsageError) { io.stderr.write(`circular harness: ${error.message}\n${usage}`); return EXIT.USAGE; }
    return failure(io, 'circular harness', error);
  }
}
