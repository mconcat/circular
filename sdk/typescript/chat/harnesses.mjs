
import fs from 'node:fs';
import path from 'node:path';

export const PROMPT = Symbol('circular.harness.prompt');

function summaryLine(text) {
  const lines = String(text ?? '').trim().split(/\r?\n/);
  const first = lines[0].length > 200 ? `${lines[0].slice(0, 199)}…` : lines[0];
  return lines.length > 1 ? `${first} …` : first;
}

function toolTarget(input) {
  for (const key of ['file_path', 'notebook_path', 'command', 'pattern', 'url', 'query', 'path', 'description']) {
    if (typeof input?.[key] === 'string' && input[key].trim() !== '') return input[key];
  }
  return null;
}

const claudeOutput = Object.freeze({
  format: 'jsonl',
  events: value => {
    if (value.type === 'system' && value.subtype === 'permission_denied') {
      return [{ type: value.subtype, summary: summaryLine(value.message ?? value.tool_name) }];
    }
    if (value.type !== 'assistant' || !Array.isArray(value.message?.content)) return [];
    return value.message.content.flatMap(block => {
      if (block.type === 'tool_use') return [{ type: block.name, summary: summaryLine(toolTarget(block.input) ?? block.name) }];
      if (block.type === 'text' && String(block.text ?? '').trim() !== '') return [{ type: block.type, summary: summaryLine(block.text) }];
      return [];
    });
  },
  result: (value, previous = {}) => {
    const refusal = value.type === 'system' && value.subtype === 'permission_denied';
    if (!refusal && value.type !== 'result') return previous;
    const denied = [...(previous.denied ?? [])];
    for (const entry of refusal ? [value] : (Array.isArray(value.permission_denials) ? value.permission_denials : [])) {
      const index = typeof entry.tool_use_id === 'string' ? denied.findIndex(item => item.id === entry.tool_use_id) : -1;
      const seen = denied[index];
      const target = toolTarget(entry.tool_input);
      const item = {
        id: entry.tool_use_id,
        tool: entry.tool_name ?? seen?.tool,
        reason: entry.decision_reason_type ?? seen?.reason ?? null,
        summary: target === null ? seen?.summary ?? summaryLine(entry.message ?? entry.tool_name) : summaryLine(target),
      };
      if (index < 0) denied.push(item);
      else denied[index] = item;
    }
    if (refusal) return { ...previous, denied };
    return {
      ...previous,
      output: value.result,
      session: value.session_id ?? null,
      denied,
      failed: value.is_error === true,
    };
  },
});

const codexOutput = Object.freeze({
  format: 'jsonl',
  events: value => {
    const item = value.item;
    if (value.type === 'item.started' && item?.type === 'command_execution') {
      return [{ type: item.type, summary: summaryLine(Array.isArray(item.command) ? item.command.join(' ') : item.command) }];
    }
    if (value.type === 'item.completed' && item?.type === 'file_change' && Array.isArray(item.changes)) {
      return [{ type: item.type, summary: summaryLine(item.changes.map(change => change.path).join(' ')) }];
    }
    if (value.type === 'item.completed' && item?.type === 'agent_message') return [{ type: item.type, summary: summaryLine(item.text) }];
    return [];
  },
  result: (value, previous = {}) => {
    if (value.type === 'thread.started' && typeof value.thread_id === 'string') return { ...previous, session: value.thread_id };
    if (value.type === 'item.completed' && value.item?.type === 'agent_message' && typeof value.item.text === 'string') {
      return { ...previous, output: value.item.text };
    }
    if (value.type === 'turn.failed') return { ...previous, failed: true };
    return previous;
  },
});

export const AUTHORING_INSTRUCTION_FILE = 'circular-authoring.md';

export const HARNESSES = Object.freeze({
  claude: Object.freeze({
    name: 'claude',
    flag: '--claude',
    helpPatterns: Object.freeze([/Usage: claude \[options\] \[command\] \[prompt\]/, /interactive session by default/]),
    instruction: Object.freeze({
      path: '.claude/skills/circular-authoring/SKILL.md',
      frontMatter: '---\nname: circular-authoring\ndescription: Build, modify, save and load Circular pipelines with the local TypeScript SDK.\n---\n\n',
    }),
    removeEnvironment: Object.freeze(['ANTHROPIC_API_KEY']),
    headless: Object.freeze({
      args: Object.freeze(['-p', '--permission-mode', 'acceptEdits', '--output-format', 'stream-json', '--verbose']),
      stdin: PROMPT, output: claudeOutput,
      readable: paths => (paths.length === 0 ? [] : ['--allowedTools', ...paths.map(entry => `Read(/${entry})`)]),
    }),
    editPermissions: '\nThe parent circular edit command passes --permission-mode acceptEdits to claude -p.\n'
      + 'When running a headless turn manually, include --permission-mode acceptEdits too.\n'
      + 'If the CLI rejects that option or reports permission_denials, stop and report the reason; do not retry with default permissions.\n'
      + 'This permits proposal file edits, not deployment approval. SDK targets remain read-only.\n',
  }),
  codex: Object.freeze({
    name: 'codex',
    flag: '--codex',
    helpPatterns: Object.freeze([/Usage: codex \[OPTIONS\] \[PROMPT\]/, /no subcommand is specified[\s\S]*interactive CLI/]),
    instruction: Object.freeze({ path: AUTHORING_INSTRUCTION_FILE, frontMatter: null }),
    removeEnvironment: Object.freeze([]),
    headless: Object.freeze({
      args: Object.freeze(['exec', '--json', '--skip-git-repo-check', PROMPT]),
      stdin: null, output: codexOutput,
    }),
    editPermissions: null,
  }),
  pi: Object.freeze({
    name: 'pi',
    flag: '--pi',
    helpPatterns: Object.freeze([/^\s*[Uu]sage:\s*pi\b[^\n]*\[(?:PROMPT|prompt)\]/m]),
    instruction: Object.freeze({ path: AUTHORING_INSTRUCTION_FILE, frontMatter: null }),
    removeEnvironment: Object.freeze([]),
    headless: Object.freeze({ args: Object.freeze(['-p', PROMPT]), stdin: null, output: Object.freeze({ format: 'text' }) }),
    editPermissions: null,
  }),
});

export const HARNESS_NAMES = Object.freeze(Object.keys(HARNESSES));

export const HARNESS_FLAGS = Object.freeze(new Map(HARNESS_NAMES.map(name => [HARNESSES[name].flag, name])));

export const harnessFlagChoice = () => HARNESS_NAMES.map(name => HARNESSES[name].flag).join('|');

export const UNPINNED_HARNESS = Object.freeze({
  name: null,
  flag: null,
  helpPatterns: null,
  instruction: Object.freeze({ path: AUTHORING_INSTRUCTION_FILE, frontMatter: null }),
  removeEnvironment: Object.freeze([]),
  headless: null,
  editPermissions: null,
});

export function harnessRow(name) {
  return HARNESSES[name] ?? UNPINNED_HARNESS;
}

export function installedProgram(name, searchPath = process.env.PATH ?? '') {
  for (const directory of searchPath.split(path.delimiter)) {
    const candidate = path.resolve(directory || '.', name);
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      if (fs.statSync(candidate).isFile()) return fs.realpathSync(candidate);
    } catch {   }
  }
  return null;
}

export function installedHarnesses(searchPath = process.env.PATH ?? '') {
  return HARNESS_NAMES
    .map(name => ({ name, program: installedProgram(name, searchPath) }))
    .filter(entry => entry.program !== null);
}

export function headlessInvocation(row, prompt) {
  return {
    args: row.headless.args.map(argument => (argument === PROMPT ? prompt : argument)),
    stdin: row.headless.stdin === PROMPT ? prompt : null,
    output: row.headless.output,
    removeEnvironment: row.removeEnvironment,
  };
}
