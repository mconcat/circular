import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { EXIT, TargetError, UsageError, failure, packageRoot, parseFlags, writeJson } from './common.mjs';
import { establish } from '@circular/client';
import { connectOwnerLocal } from '@circular/client/owner-local';
import { ceilings } from '../chat/launcher.mjs';
import { commitsAfter, cutOf, deltaLines, openCommits } from './commit-delta.mjs';

export const usage = `usage: circular template list [--json]
       circular template deploy <name> --state <dir> [template options...]

list    names the templates this installation ships and the first line of each README.
deploy  runs that template's own deploy.mjs with the arguments you pass. Everything after the
        name is handed to the template unchanged; there is no second configuration format.
        The exit code is the template's. A deploy whose commit answer did not arrive, and
        which the commit record does not show, exits 4.

Read templates/<name>/README.md for what each one declares and what it needs installed.
`;

export const templatesRoot = path.join(packageRoot, 'templates');

export function listTemplates(root = templatesRoot) {
  if (!fs.existsSync(root)) return [];
  return fs.readdirSync(root, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && fs.existsSync(path.join(root, entry.name, 'deploy.mjs')))
    .map(entry => {
      const readme = path.join(root, entry.name, 'README.md');
      const summary = fs.existsSync(readme)
        ? (fs.readFileSync(readme, 'utf8').split('\n').find(line => line.trim().length > 0) ?? '').replace(/^#+\s*/, '').trim()
        : null;
      return { name: entry.name, directory: path.join(root, entry.name), summary };
    })
    .sort((left, right) => (left.name < right.name ? -1 : 1));
}

async function openWatch(state) {
  const transport = await connectOwnerLocal({ root: state, socketName: 'daemon.sock' });
  const session = await establish(transport, { hello: { requestedRoles: [1n] }, resourceCeilings: ceilings, requestTimeoutMs: 5000 });
  return { session, commits: await openCommits(session, cutOf(await session.authoringSnapshot([], 256))) };
}

export async function main(argv, io = process) {
  const action = argv[0];
  if (argv.includes('--help') || argv.includes('-h') || action === undefined) {
    io.stdout.write(usage);
    return action === undefined ? EXIT.USAGE : EXIT.OK;
  }
  if (action !== 'list' && action !== 'deploy') {
    io.stderr.write(`circular template: one of list|deploy is required\n${usage}`);
    return EXIT.USAGE;
  }
  try {
    if (action === 'list') {
      const options = parseFlags(argv.slice(1), { flags: { '--json': 'json' } });
      if (options.positional.length) throw new UsageError('list takes no positional argument');
      const templates = listTemplates();
      if (options.json) writeJson(io, { root: templatesRoot, templates });
      else if (templates.length === 0) io.stdout.write(`no template ships with this installation (${templatesRoot})\n`);
      else for (const template of templates) io.stdout.write(`${template.name}\t${template.summary ?? '(no README)'}\n`);
      return EXIT.OK;
    }
    const name = argv[1];
    if (name === undefined || name.startsWith('-')) throw new UsageError('deploy takes a template name first');
    const template = listTemplates().find(entry => entry.name === name);
    if (!template) {
      throw new TargetError(`no template named ${name}; this installation ships ${listTemplates().map(entry => entry.name).join(', ') || '(none)'}`);
    }
    const entry = path.join(template.directory, 'deploy.mjs');
    const state = argv.slice(2).find((value, index, args) => args[index - 1] === '--state');
    const watch = state && fs.existsSync(path.join(state, 'daemon.sock')) ? await openWatch(state).catch(error => ({
      session: null, commits: { reason: { at: 'before', code: error?.code ?? error?.name ?? 'Error' } } })) : null;
    const saved = process.argv;
    process.argv = [saved[0], entry, ...argv.slice(2)];
    try {
      await import(pathToFileURL(entry).href);
      if (watch) for (const line of deltaLines(await commitsAfter(watch.session, watch.commits))) io.stderr.write(`${line}\n`);
    } finally {
      process.argv = saved;
      if (watch?.session) await watch.session.goodbye().catch(() => watch.session.close());
    }
    return process.exitCode ?? EXIT.OK;
  } catch (error) {
    if (error instanceof UsageError) { io.stderr.write(`circular template: ${error.message}\n${usage}`); return EXIT.USAGE; }
    return failure(io, 'circular template', error);
  }
}
