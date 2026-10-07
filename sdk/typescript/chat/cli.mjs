import path from 'node:path';
import { createChatSession, openTerminal, validateState } from './launcher.mjs';
import { HARNESS_FLAGS, harnessFlagChoice } from './harnesses.mjs';
import { AgentCliSelectionError, selectAgentCli } from './select.mjs';

const oneAgentCli = `choose at most one of ${harnessFlagChoice()}`;

export const usage = `usage: circular chat --state <dir> [${harnessFlagChoice()}]
    [--project <name>] [--cli-bin <absolute-path>] [--no-open] [--print-script] [--dry-run]

Opens an agent CLI in a session directory under <state>/chat/, holding the pipeline that is
deployed right now as SDK code, the authoring instructions generated from this daemon, and the
public AGENTS.md and CLAUDE.md this installation ships, with a connection section appended.

Without an agent-CLI flag the choice is made from what is installed on this machine and saved
as this state's default; a saved default is reused from then on. With several installed and no
saved default, an interactive shell asks and a non-interactive one lists them and exits 2.

The agent CLI opened here is not a pipeline harness. Bind a harness with circular harness bind.

--state must exist below your home, belong to you, and have mode 0700; it is never guessed.
--project labels the session; the daemon snapshot scope is root.
--dry-run writes recovery files without installing packages, probing a CLI, or opening Terminal.
--print-script prints the script without opening Terminal; --no-open prints its path.
--cli-bin overrides the executable path of the selected agent CLI.
No environment variable supplies a chat option. Terminal.app is fixed.
`;

export function parseArguments(argv) {
  const options = { noOpen: false, printScript: false, harness: null };
  const seen = new Set();
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (seen.has(arg)) throw new Error(`duplicate argument: ${arg}`);
    seen.add(arg);
    if (arg === '--help' || arg === '-h') options.help = true;
    else if (HARNESS_FLAGS.has(arg)) {
      if (options.harness) throw new Error(oneAgentCli);
      options.harness = HARNESS_FLAGS.get(arg);
    } else if (arg === '--no-open') options.noOpen = true;
    else if (arg === '--print-script') options.printScript = true;
    else if (arg === '--dry-run') options.dryRun = true;
    else if (['--state', '--project', '--cli-bin'].includes(arg)) {
      const value = argv[++i];
      if (!value || value.startsWith('-') || /[\x00-\x1f\x7f]/.test(value)) throw new Error(`${arg} requires a value without control characters`);
      options[{ '--state': 'state', '--project': 'project', '--cli-bin': 'harnessBin' }[arg]] = value;
    } else throw new Error(`unknown argument: ${arg}`);
  }
  if (options.help) return options;
  if (!options.state || !path.isAbsolute(options.state)) throw new Error('--state requires an absolute directory');
  if (options.harnessBin && !path.isAbsolute(options.harnessBin)) throw new Error('--cli-bin requires an absolute path');
  return options;
}

export async function main(argv, io = process, dependencies = {}) {
  let options;
  try { options = parseArguments(argv); }
  catch (error) { io.stderr.write(`circular chat: ${error.message}\n${usage}`); return 2; }
  if (options.help) { io.stdout.write(usage); return 0; }
  try {
    const accept = dependencies.validateState ?? validateState;
    const state = accept(options.state);
    let chosen;
    try {
      chosen = await (dependencies.selectAgentCli ?? selectAgentCli)({
        state, requested: options.harness, program: options.harnessBin ?? null, io,
      });
    } catch (error) {
      if (!(error instanceof AgentCliSelectionError)) throw error;
      io.stderr.write(`circular chat: ${error.message}\n`);
      return error.usage ? 2 : 1;
    }
    const resolved = { ...options, harness: chosen.name, harnessBin: options.harnessBin ?? chosen.program };
    const session = await (dependencies.createChatSession ?? createChatSession)(resolved);
    for (const diagnostic of session.diagnostics) io.stderr.write(`circular chat: ${diagnostic}\n`);
    for (const written of session.rootInstructions) {
      io.stderr.write(`circular chat: ${written.file} copied from the ${written.origin} instruction file ${written.source}\n`);
    }
    if (options.printScript) io.stdout.write(session.script);
    else io.stdout.write(`${session.scriptPath}\n`);
    if (!options.noOpen && !options.printScript && !options.dryRun) {
      const opened = (dependencies.openTerminal ?? openTerminal)(session.scriptPath);
      if (opened.status !== 0 || opened.error) {
        io.stderr.write(`circular chat: Terminal could not open (${opened.error?.code ?? opened.status}); run ${session.scriptPath}\n`);
        return 1;
      }
    }
    return 0;
  } catch (error) { io.stderr.write(`circular chat: ${error.message}\n`); return 1; }
}
