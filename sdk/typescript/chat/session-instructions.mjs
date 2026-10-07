import fs from 'node:fs';
import path from 'node:path';

const packageRoot = path.resolve(import.meta.dirname, '..');
const checkoutRoot = path.resolve(packageRoot, '..', '..');

export const PUBLIC_INSTRUCTION_FILES = Object.freeze(['AGENTS.md', 'CLAUDE.md']);

export function publicInstruction(name, roots = [checkoutRoot, packageRoot]) {
  for (const root of roots) {
    const file = path.join(root, name);
    if (fs.existsSync(file)) return { path: file, text: fs.readFileSync(file, 'utf8'), origin: root === checkoutRoot ? 'checkout' : 'installed' };
  }
  return { path: null, text: null, origin: 'absent' };
}

const REFERENCED_DOCUMENTS = Object.freeze([
  'QUICKSTART.md', 'README.md', 'reference/actors/', 'reference/combinators.md', 'DATA.md', 'VERSIONING.md',
]);

export function referencedDocuments(documentRoot, templatesRoot) {
  const rows = REFERENCED_DOCUMENTS.map(name => {
    const file = documentRoot === null ? null : path.join(documentRoot, name);
    return { name, path: file !== null && fs.existsSync(file) ? file : null };
  });
  rows.push({ name: 'templates/<name>/README.md', path: fs.existsSync(templatesRoot) ? `${templatesRoot}/<name>/README.md` : null });
  return rows;
}

export function connectionSection({ state, socket, agentCli, agentCliProgram, instructionPath, cliVersion, cliCommit, daemonVersion, daemon, documents = [] }) {
  return [
    '',
    '---',
    '',
    '## This session',
    '',
    'This section was written by `circular chat` when it opened this directory. Everything above',
    'it is the shipped public instruction file, unchanged.',
    '',
    '| | |',
    '|---|---|',
    `| State directory | \`${state}\` |`,
    `| Daemon socket | \`${socket}\` |`,
    `| Daemon | ${daemon} |`,
    `| Agent CLI opening this session | \`${agentCli}\` (\`${agentCliProgram}\`) |`,
    `| circular CLI | ${cliVersion} (${cliCommit}) |`,
    `| circular-daemon (installed binary) | ${daemonVersion ?? 'not installed beside this command'} |`,
    `| Authoring instructions for this session | \`${instructionPath}\` |`,
    '',
    `**Read \`${instructionPath}\` before you write any SDK code.** It is generated from *this*`,
    "daemon's actor catalog and *this* state's harness bindings, and it — not the text above — is",
    'the authority on SDK spellings. `first-message.md` carries what was standing when this',
    'session opened.',
    '',
    'Pass `--state` explicitly to every `circular` command you run from here; nothing guesses it.',
    ...(documents.length === 0 ? [] : [
      '',
      'The documents named in "Where else to look" are not copied into this directory. On this',
      'machine they are here; read them in place and do not edit them:',
      '',
      '| Document | Path on this machine |',
      '|---|---|',
      ...documents.map(row => `| \`${row.name}\` | ${row.path === null ? 'not in this installation' : `\`${row.path}\``} |`),
    ]),
    '',
  ].join('\n');
}
