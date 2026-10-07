import { EXIT, UsageError, cliIdentity, parseFlags, requireState, writeJson } from './common.mjs';
import { installedDaemon } from './daemon.mjs';

export const usage = `usage: circular version [--state <dir>] [--json]

Prints this command line's version and the commit it was built from. With --state it also
prints the version of the circular-daemon executable installed beside it. No query publishes
the version of a daemon that is already running, so a running daemon's build is not claimed
here — restarting from this installation is what makes the two agree.

circular --version prints the first line alone.
`;

export async function main(argv, io = process, launcher = process.argv[1]) {
  let options;
  try {
    options = parseFlags(argv, { values: { '--state': 'state' }, flags: { '--json': 'json' } });
    if (options.help) { io.stdout.write(usage); return EXIT.OK; }
    if (options.positional.length) throw new UsageError('version takes no positional argument');
  } catch (error) {
    io.stderr.write(`circular version: ${error.message}\n${usage}`);
    return EXIT.USAGE;
  }
  const cli = await cliIdentity();
  try {
    if (options.state === undefined) {
      if (options.json) writeJson(io, { cli: cli.version, commit: cli.commit, installedDaemon: null, installedDaemonVersion: null });
      else io.stdout.write(`circular ${cli.version} (${cli.commit})\n`);
      return EXIT.OK;
    }
    const state = requireState(options);
    const daemon = installedDaemon(launcher);
    if (options.json) {
      writeJson(io, { state, cli: cli.version, commit: cli.commit, installedDaemon: daemon.program, installedDaemonVersion: daemon.version });
    } else {
      io.stdout.write(`circular ${cli.version} (${cli.commit})\n`
        + `${daemon.version ?? 'circular-daemon not installed'}${daemon.program ? ` (installed binary ${daemon.program})` : ''}\n`);
    }
    return EXIT.OK;
  } catch (error) {
    io.stderr.write(`circular version: ${error.message}\n`);
    return EXIT.FAILED;
  }
}
