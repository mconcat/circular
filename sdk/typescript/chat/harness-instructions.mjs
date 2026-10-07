/** Read names from the daemon; never infer the runtime name from the authoring CLI. */
import fs from 'node:fs';
import path from 'node:path';
import { ACTOR_TYPE_NAMES } from '@circular/specs';
import { agentHarnesses, agentHarnessesPageFromValue } from '@circular/client';
import { harnessFlagChoice } from './harnesses.mjs';

export async function readAgentHarnesses(session) {
  try {
    const answer = await session.exchange('Query', 'Query', {name: agentHarnesses.name, args: null});
    const body = answer.payload;
    if (body?.[0] === 2n) throw new Error(body[1]?.message ?? 'agent.harnesses rejected');
    if (answer.kind?.verb !== 'QueryResult' || body?.[0] !== 1n) throw new Error('agent.harnesses returned no result');
    const page = agentHarnessesPageFromValue(body[1]);
    return {status: 'reported', bindings: page.items.map(({name, program, saved}) => ({name, program, saved}))};
  } catch (error) {
    return {status: 'unavailable', bindings: [], diagnostic: error.message};
  }
}

/** The program a binding names for this state: the saved one, or the one the pipeline holds. */
export const bindingProgram = binding => binding.saved ?? binding.program;

const canonicalProgram = program => {
  if (!program) return null;
  try { return fs.realpathSync(program); } catch { return path.resolve(program); }
};
export function harnessInstructionContext(current, program) {
  const report = current.agentHarnesses;
  if (report?.status !== 'reported') return {
    name: null,
    text: `Runtime agent harness bindings: unknown (${JSON.stringify(report?.diagnostic ?? 'daemon bindings were not read')}). Agent examples are unavailable; do not deploy an agent until bindings are reported.`,
  };
  if (report.bindings.length === 0) return {
    name: null,
    text: 'Runtime agent harness bindings: none (agent.harnesses returned an empty complete page). Agent examples are unavailable; bind a harness before deploying an agent.',
  };
  const matches = report.bindings.filter(binding => canonicalProgram(bindingProgram(binding)) === canonicalProgram(program));
  const selected = matches[0] ?? report.bindings[0];
  const reason = matches.length ? 'matching the authoring executable' : 'first reported binding; the authoring executable has no matching configured program';
  return {name: selected.name, text: [
    'Runtime agent harness bindings reported by agent.harnesses:',
    ...report.bindings.map(binding => `- name: ${JSON.stringify(binding.name)}; program: ${JSON.stringify(binding.program ?? null)}; saved: ${JSON.stringify(binding.saved ?? null)}`),
    `Agent examples use harness: ${JSON.stringify(selected.name)} (${reason}). Other reported names may be selected verbatim.`,
  ].join('\n')};
}

/** Harness facts the instructions state, read from the table rather than typed into the prose. */
export function harnessProseFacts() {
  return {
    __CHAT_AGENT_CLI_FLAGS__: harnessFlagChoice(),
  };
}

export function renderInstructions(template, context) {
  let text = template.replaceAll('__CATALOG_CONSTRUCTOR_COUNT__', String(ACTOR_TYPE_NAMES.length));
  for (const [placeholder, value] of Object.entries(harnessProseFacts())) text = text.replaceAll(placeholder, value);
  text = text.replace('__AGENT_HARNESS_REPORT__', context.text);
  if (context.name === null) {
    text = text.replace(/```ts\n(?:(?!```)[\s\S])*?__BOUND_AGENT_HARNESS__(?:(?!```)[\s\S])*?```/g,
      'Agent example unavailable until a runtime harness binding is reported. Follow the binding steps above.');
  } else {
    text = text.replaceAll('__BOUND_AGENT_HARNESS__', JSON.stringify(context.name));
  }
  return text;
}
