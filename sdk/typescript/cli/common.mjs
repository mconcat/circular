import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const EXIT = Object.freeze({ OK: 0, FAILED: 1, USAGE: 2, WAIT: 4 });

export const exitFor = error => (error?.commitUnknown === true ? EXIT.WAIT : EXIT.FAILED);

export function failure(io, command, error, check = null) {
  io.stderr.write(`${command}: ${error?.message ?? error}${check === null ? '' : ` ${check}`}\n`);
  return exitFor(error);
}

import { OWNER_LOCAL_SOCKET_NAME as SOCKET_NAME, OWNER_LOCAL_RESOURCE_CEILINGS as CEILINGS } from '@circular/client';
export { SOCKET_NAME, CEILINGS };

export const CLAIM_SUFFIX = '.owner-local.lock';

export const packageRoot = path.resolve(import.meta.dirname, '..');

export class UsageError extends Error {}
export class TargetError extends Error {}

export function parseFlags(argv, spec) {
  const options = { positional: [] };
  for (const name of Object.values(spec.flags ?? {})) options[name] = false;
  const seen = new Set();
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--help' || argument === '-h') { options.help = true; continue; }
    if (Object.hasOwn(spec.values ?? {}, argument)) {
      if (seen.has(argument)) throw new UsageError(`duplicate argument: ${argument}`);
      seen.add(argument);
      const value = argv[index + 1];
      index += 1;
      if (value === undefined || value.startsWith('-') || /[\x00-\x1f\x7f]/.test(value)) {
        throw new UsageError(`${argument} requires a value without control characters`);
      }
      options[spec.values[argument]] = value;
      continue;
    }
    if (Object.hasOwn(spec.flags ?? {}, argument)) {
      if (seen.has(argument)) throw new UsageError(`duplicate argument: ${argument}`);
      seen.add(argument);
      options[spec.flags[argument]] = true;
      continue;
    }
    if (argument.startsWith('-')) throw new UsageError(`unknown argument: ${argument}`);
    options.positional.push(argument);
  }
  return options;
}

export function requireState(options, { mustExist = true } = {}) {
  const state = options.state;
  if (!state || !path.isAbsolute(state)) throw new UsageError('--state requires an absolute directory');
  if (!mustExist) return path.resolve(state);
  let canonical;
  try { canonical = fs.realpathSync(path.resolve(state)); }
  catch (error) { throw new TargetError(`--state ${state}: ${error.code === 'ENOENT' ? 'no such directory' : error.message}`); }
  const home = fs.realpathSync(os.userInfo().homedir);
  if (!canonical.startsWith(`${home}${path.sep}`)) throw new TargetError('state must be strictly below the user home');
  const stat = fs.lstatSync(canonical);
  if (!stat.isDirectory() || stat.uid !== process.getuid() || (stat.mode & 0o7777) !== 0o700) {
    throw new TargetError(`state must be an owned directory with mode 0700: chmod 700 ${canonical}`);
  }
  return canonical;
}

export const socketPath = state => path.join(state, SOCKET_NAME);
export const claimPath = state => path.join(state, SOCKET_NAME + CLAIM_SUFFIX);

export function siblingProgram(name, launcher = process.argv[1]) {
  const here = path.dirname(path.resolve(launcher ?? packageRoot));
  const sibling = path.join(here, name);
  if (fs.existsSync(sibling)) return { path: sibling, source: 'sibling' };
  for (const directory of (process.env.PATH ?? '').split(path.delimiter)) {
    const candidate = path.resolve(directory || '.', name);
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      if (fs.statSync(candidate).isFile()) return { path: fs.realpathSync(candidate), source: 'path' };
    } catch {   }
  }
  return { path: null, source: 'absent' };
}

export function programRemedy(name) {
  return name === 'circular'
    ? 'install Circular with scripts/install.sh (QUICKSTART §1); in a source checkout, run node sdk/typescript/circular.mjs instead (QUICKSTART §1, lane B)'
    : `install Circular with scripts/install.sh (QUICKSTART §1); in a source checkout, put the directory holding ${name} (cargo's target/debug) on PATH, or start the daemon by hand (QUICKSTART §3)`;
}

export function programAbsent(name, launcher = process.argv[1]) {
  const here = path.dirname(path.resolve(launcher ?? packageRoot));
  return `install.program.${name}: ${name} is neither beside this command (${here}) nor on PATH; ${programRemedy(name)}`;
}

export async function cliIdentity() {
  const manifest = JSON.parse(fs.readFileSync(path.join(packageRoot, 'package.json'), 'utf8'));
  try {
    const info = JSON.parse(fs.readFileSync(path.join(packageRoot, 'build-info.json'), 'utf8'));
    return { version: manifest.version, commit: info.commit + (info.dirty ? '-dirty' : '') };
  } catch {
    const { gitDescription } = await import('../scripts/write-build-info.mjs');
    const described = gitDescription(fileURLToPath(new URL('.', import.meta.url)));
    return { version: manifest.version, commit: described === null ? 'unknown' : described.sha + (described.dirty ? '-dirty' : '') };
  }
}

export const writeJson = (io, value) => io.stdout.write(`${JSON.stringify(value)}\n`);
