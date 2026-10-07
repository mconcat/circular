import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { EXIT, UsageError, cliIdentity, packageRoot, parseFlags, requireState, siblingProgram, socketPath, writeJson } from './common.mjs';
import { healthOf, holdingPid, installedDaemon, operatingLogPath } from './daemon.mjs';
import { INSTALLED_PROGRAMS, diagnose } from './doctor.mjs';
import { withSession } from './session.mjs';

export const usage = `usage: circular bugreport [--state <dir>] [--out <absolute-file>] [--lines <n>]

Writes one JSON file on this machine and prints its path. Nothing is uploaded and no address
is contacted. The default destination is ~/circular-bugreport-<timestamp>.json.

It carries the doctor rows, the installed versions and their paths, daemon.health, a count of
journal records by kind, and the tail of the daemon's operating log with obvious secrets
redacted. It never carries the journal itself, your config.toml, your SDK sources, or a
credential. Read the file before you attach it anywhere: the redaction is a pass over known
shapes, not a proof that nothing sensitive is left.
`;

const SECRET_PATTERNS = Object.freeze([
  /\b(?:sk|pk|ghp|gho|ghs|github_pat|xox[abprs])[-_][A-Za-z0-9_-]{16,}/g,
  /\b(?:Bearer|Basic)\s+[A-Za-z0-9+/=._-]{16,}/gi,
  /\b(?:api[_-]?key|secret|token|password|passwd|authorization)\b\s*[:=]\s*\S+/gi,
  /\b[A-Za-z0-9+/]{40,}={0,2}\b/g,
]);

export function redact(text) {
  let redacted = text;
  for (const pattern of SECRET_PATTERNS) redacted = redacted.replaceAll(pattern, '<redacted>');
  return redacted;
}

export function storageCensus(state) {
  const census = [];
  const walk = (directory, relative) => {
    let entries;
    try { entries = fs.readdirSync(directory, { withFileTypes: true }); }
    catch (error) { census.push({ path: relative, error: error.code ?? error.message }); return; }
    let files = 0;
    let bytes = 0;
    for (const entry of entries) {
      const full = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        if (entry.name === 'node_modules') { census.push({ path: path.join(relative, entry.name), skipped: 'installed SDK copy' }); continue; }
        walk(full, path.join(relative, entry.name));
        continue;
      }
      files += 1;
      try { bytes += fs.statSync(full).size; } catch {   }
    }
    census.push({ path: relative, files, bytes });
  };
  walk(state, '.');
  return census.sort((left, right) => (left.path < right.path ? -1 : 1));
}

export async function recordCensus(state, limit = 4096) {
  try {
    return await withSession(state, async session => {
      const answer = await session.exchange('Query', 'Query', { name: 'records', args: { scope: [] }, page: { limit: BigInt(limit) } });
      const body = answer.payload;
      if (body?.[0] !== 1n) return { measured: false, diagnostic: body?.[1]?.message ?? 'records was refused' };
      const page = body[1];
      const kinds = {};
      for (const item of page.items ?? []) {
        const kind = 'arrival_fact';
        kinds[kind] = (kinds[kind] ?? 0) + 1;
      }
      return { measured: true, counted: (page.items ?? []).length, complete: page.terminal === 2n, kinds };
    });
  } catch (error) {
    return { measured: false, diagnostic: error.message };
  }
}

export async function collect({ state, launcher, lines }) {
  const identity = await cliIdentity();
  const report = {
    schema: 'circular-bugreport/1',
    writtenAt: new Date().toISOString(),
    platform: { platform: process.platform, release: os.release(), arch: process.arch, node: process.versions.node },
    cli: { version: identity.version, commit: identity.commit, packageRoot },
    installed: Object.fromEntries(INSTALLED_PROGRAMS.map(name => [name, siblingProgram(name, launcher).path])),
    installedDaemonVersion: installedDaemon(launcher).version,
    state: state ?? null,
    doctor: await diagnose({ state, launcher }),
  };
  if (state === null) return report;
  report.socket = socketPath(state);
  report.pid = holdingPid(state);
  const reachable = await healthOf(state);
  report.daemonHealth = reachable.answering ? reachable.health : null;
  report.daemonHealthDiagnostic = reachable.answering ? null : reachable.diagnostic;
  report.storage = storageCensus(state);
  report.records = reachable.answering ? await recordCensus(state) : { measured: false, diagnostic: 'the daemon did not answer' };
  const log = operatingLogPath(state);
  report.operatingLog = { path: log, present: fs.existsSync(log), tail: null, redacted: true };
  if (report.operatingLog.present) {
    const text = fs.readFileSync(log, 'utf8').split('\n');
    report.operatingLog.tail = redact(text.slice(Math.max(0, text.length - lines)).join('\n')).split('\n');
  }
  return report;
}

export async function main(argv, io = process, launcher = process.argv[1]) {
  let options;
  try {
    options = parseFlags(argv, { values: { '--state': 'state', '--out': 'out', '--lines': 'lines' }, flags: {} });
    if (options.help) { io.stdout.write(usage); return EXIT.OK; }
    if (options.positional.length) throw new UsageError('bugreport takes no positional argument');
    if (options.out !== undefined && !path.isAbsolute(options.out)) throw new UsageError('--out requires an absolute file path');
  } catch (error) {
    io.stderr.write(`circular bugreport: ${error.message}\n${usage}`);
    return EXIT.USAGE;
  }
  const lines = options.lines === undefined ? 500 : Number(options.lines);
  if (!Number.isSafeInteger(lines) || lines <= 0) {
    io.stderr.write(`circular bugreport: --lines must be a positive integer\n${usage}`);
    return EXIT.USAGE;
  }
  try {
    const state = options.state === undefined ? null : requireState(options);
    const report = await collect({ state, launcher, lines });
    const stamp = report.writtenAt.replaceAll(':', '-');
    const out = options.out ?? path.join(os.userInfo().homedir, `circular-bugreport-${stamp}.json`);
    fs.writeFileSync(out, `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 });
    io.stdout.write(`${out}\n`);
    io.stderr.write('circular bugreport: nothing was uploaded. Read the file before you attach it anywhere.\n');
    return EXIT.OK;
  } catch (error) {
    if (error instanceof UsageError) { io.stderr.write(`circular bugreport: ${error.message}\n${usage}`); return EXIT.USAGE; }
    io.stderr.write(`circular bugreport: ${error.message}\n`);
    return EXIT.FAILED;
  }
}
