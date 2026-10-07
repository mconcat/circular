import { createInterface } from 'node:readline/promises';
import { HARNESS_NAMES, installedHarnesses, installedProgram } from './harnesses.mjs';
import { readChatDefaults, writeChatDefault } from './defaults.mjs';

export class AgentCliSelectionError extends Error {
  constructor(usage, message) { super(message); this.usage = usage; }
}

const list = installed => installed.map(entry => `  ${entry.name}\t${entry.program}`).join('\n');

export async function selectAgentCli({ state, requested, program: named = null, searchPath = process.env.PATH ?? '', io = process, interactive = process.stdin.isTTY === true }) {
  const installed = installedHarnesses(searchPath);
  if (requested !== null && requested !== undefined) {
    const program = named ?? installedProgram(requested, searchPath);
    if (program === null) {
      throw new AgentCliSelectionError(false, `${requested} is not installed on this machine; PATH holds ${installed.map(entry => entry.name).join(', ') || 'none of ' + HARNESS_NAMES.join(', ')}`);
    }
    const file = writeChatDefault(state, requested);
    io.stderr.write(`circular chat: agent CLI ${requested} (${program}); saved as this state's default in ${file}\n`);
    return { name: requested, program, source: 'flag', saved: file };
  }
  if (installed.length === 0) {
    throw new AgentCliSelectionError(false, `no agent CLI is installed on this machine; this command knows ${HARNESS_NAMES.join(', ')} and finds an executable of that name on PATH`);
  }
  const stored = readChatDefaults(state);
  if (stored.diagnostic !== null) io.stderr.write(`circular chat: stored default ignored — ${stored.diagnostic} (${stored.path})\n`);
  if (stored.agentCli !== null) {
    const program = installedProgram(stored.agentCli, searchPath);
    if (program !== null) {
      io.stderr.write(`circular chat: agent CLI ${stored.agentCli} (${program}); this state's saved default\n`);
      return { name: stored.agentCli, program, source: 'saved', saved: stored.path };
    }
    io.stderr.write(`circular chat: saved default ${stored.agentCli} is no longer installed; choosing again\n`);
  }
  if (installed.length === 1) {
    const only = installed[0];
    const file = writeChatDefault(state, only.name);
    io.stderr.write(`circular chat: agent CLI ${only.name} (${only.program}); the only one installed, saved as this state's default in ${file}\n`);
    return { name: only.name, program: only.program, source: 'only-installed', saved: file };
  }
  if (!interactive) {
    throw new AgentCliSelectionError(true, `several agent CLIs are installed and this state has no saved default; name one with its flag:\n${list(installed)}`);
  }
  const reader = createInterface({ input: process.stdin, output: process.stderr });
  try {
    io.stderr.write(`circular chat: several agent CLIs are installed:\n${list(installed)}\n`);
    for (;;) {
      const answer = (await reader.question(`choose one of ${installed.map(entry => entry.name).join(', ')}: `)).trim();
      const chosen = installed.find(entry => entry.name === answer);
      if (chosen) {
        const file = writeChatDefault(state, chosen.name);
        io.stderr.write(`circular chat: agent CLI ${chosen.name} (${chosen.program}); saved as this state's default in ${file}\n`);
        return { name: chosen.name, program: chosen.program, source: 'prompt', saved: file };
      }
      io.stderr.write(`${JSON.stringify(answer)} is not one of the installed names\n`);
    }
  } finally { reader.close(); }
}
