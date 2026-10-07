#!/usr/bin/env node

import process from 'node:process';

const tree = [
  ['chat', 'open an agent session that can edit the pipeline standing in a state'],
  ['edit', 'propose, review and apply one SDK change through a bound harness'],
  ['daemon', 'start, stop, restart, inspect, tail and register the daemon for a state'],
  ['doctor', 'diagnose this installation, this state and the agent CLIs; change nothing'],
  ['bugreport', 'write one local diagnostic file; upload nothing'],
  ['version', 'print this command line\'s version, and the installed daemon\'s with --state'],
  ['harness', 'list the harness bindings of a state and the programs the daemon found; bind or unbind one'],
  ['template', 'list the templates this installation ships, or deploy one'],
];

const help = `usage: circular <command> [options]

${tree.map(([verb, summary]) => `       circular ${verb.padEnd(10)} ${summary}`).join('\n')}

Use --version for the same string as the version command, and <command> --help for one
command's own usage.

Every command that names a state takes --state <absolute directory>; none is guessed.
Options come only from arguments. No environment variable configures this command line.
Exit codes: 0 done, 1 the target failed, 2 the request was malformed, 3 the registrar refused
the state location (daemon install|uninstall pass its code through), 4 the daemon has not
answered yet: a commit or a harness binding was sent, its answer did not arrive and the record
(the commit record, or agent.harnesses for a binding) does not show it, so call the same
command again.
`;

const modules = {
  chat: './chat/cli.mjs',
  edit: './edit/cli.mjs',
  daemon: './cli/daemon.mjs',
  doctor: './cli/doctor.mjs',
  bugreport: './cli/bugreport.mjs',
  version: './cli/version.mjs',
  harness: './cli/harness.mjs',
  template: './cli/template.mjs',
};

const [verb, ...rest] = process.argv.slice(2);
if (verb === '--version' || verb === '-v') {
  const { cliIdentity } = await import('./cli/common.mjs');
  const identity = await cliIdentity();
  process.stdout.write(`circular ${identity.version} (${identity.commit})\n`);
} else if (verb === '--help' || verb === '-h') {
  process.stdout.write(help);
} else if (Object.hasOwn(modules, verb)) {
  const { main } = await import(modules[verb]);
  process.exitCode = await main(rest);
} else {
  process.stderr.write(`${verb === undefined ? '' : `circular: unknown command ${JSON.stringify(verb)}\n`}${help}`);
  process.exitCode = 2;
}
