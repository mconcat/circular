/** Chat recovery -> SDK proposal -> explicit approval -> installed host execution. */
import fs from 'node:fs';
import path from 'node:path';
import { createChatSession, emptySource, snapshotFromSession, validateState } from '../chat/launcher.mjs';
import { EpochRunnerError } from '@circular/client/epoch-runner';
import { diagnosticLine, executeProgram, loadProgram, openAuthoringSession, refusal } from '../chat/execution.mjs';
import { commitsAfter, cutOf, deltaSummary, openCommits } from '../cli/commit-delta.mjs';
import { harnessRow } from '../chat/harnesses.mjs';
import { bindingProgram } from '../chat/harness-instructions.mjs';
import { CHAT_DIRECTORY } from '../chat/defaults.mjs';
import { currentFileSource } from '../chat/current-file.mjs';

const write = (file, body) => fs.writeFileSync(file, body, { mode: 0o600, flag: 'wx' });

export function currentFile(directory) {
  for (const relative of ['current.ts', 'current/main.ts', 'main.ts']) {
    if (fs.existsSync(path.join(directory, relative))) return path.join(directory, relative);
  }
  throw Object.assign(new Error('chat recovery code is absent; open a new session to regenerate current from the daemon'),
    { code: 'CURRENT_RECOVERY_ABSENT' });
}

/**
 * Writes the program standing now where the session keeps it: current.ts, or current/ when it has
 * scope modules. It is a projection of the fold, so it is replaced after every commit and never
 * edited in place. The genesis skeleton that stood in for it goes with it.
 */
export function replaceCurrent(directory, standing) {
  fs.rmSync(path.join(directory, 'current'), { recursive: true, force: true });
  fs.rmSync(path.join(directory, 'current.ts'), { force: true });
  const skeleton = path.join(directory, 'main.ts');
  if (fs.existsSync(skeleton) && fs.readFileSync(skeleton, 'utf8') === currentFileSource(emptySource, null)) fs.rmSync(skeleton);
  if (!standing) return null;
  if (standing.program?.modules.size > 1) return copyProgram(standing.program, path.join(directory, 'current'), 'main.ts', { cursor: standing.cursor });
  write(path.join(directory, 'current.ts'), currentFileSource(standing.source, standing.program ? standing.cursor : null));
  return path.join(directory, 'current.ts');
}

/** The file an edit starts from: the change to make, never a copy of what stands. */
function changeProgram(currentName) {
  return `// Write only the change you intend; everything you do not mention stays as it is.\n`
    + `// ${currentName} is the pipeline standing now. It is read-only and is regenerated after each approval.\n`
    + `// Import the actors you refer to from "circular:current", for example:\n`
    + `//   import { name } from "circular:current";\n`;
}

export function copyProgram(program, directory, entryName, origin = null) {
  fs.mkdirSync(directory, { recursive: true, mode: 0o700 });
  for (const [module, bytes] of program.modules) {
    const destination = path.join(directory, module === program.entry ? entryName : module);
    fs.mkdirSync(path.dirname(destination), { recursive: true, mode: 0o700 });
    write(destination, origin ? currentFileSource(bytes, origin.cursor) : bytes);
  }
  return path.join(directory, entryName);
}

export async function templateTask(template) {
  if (!template) return '';
  if (!['agent-session-monitor', 'incident-autopilot'].includes(template)) throw new Error(`unknown template: ${template}`);
  return (await import(`../templates/${template}/edit-task.mjs`)).task;
}

export function selectEditHarness(current, options) {
  if (options.rollback) return null;
  const report = current.agentHarnesses;
  if (report?.status !== 'reported') {
    if (options.dryRun) return null;
    throw new Error(`agent.harnesses unavailable: ${report?.diagnostic ?? 'daemon bindings were not read'}; connect to the daemon before running edit`);
  }
  const binding = report.bindings.find(binding => binding.name === options.harness);
  if (!binding) throw new Error(`--harness ${JSON.stringify(options.harness)} is not bound; agent.harnesses reports: ${report.bindings.map(binding => JSON.stringify(binding.name)).join(', ') || '(none)'}`);
  return { program: options.harnessBin ?? bindingProgram(binding) };
}

export async function createEditSession(options, dependencies = {}) {
  const task = await templateTask(options.template);
  const opened = await createChatSession(options, { ...dependencies, selectHarness: selectEditHarness });
  const baseline = currentFile(opened.directory);
  const currentName = path.relative(opened.directory, baseline);
  const empty = opened.actors === 0;
  const proposal = path.join(opened.directory, 'proposal.ts');
  write(proposal, empty ? emptySource : changeProgram(currentName));
  const relative = path.relative(opened.directory, proposal);
  const permissions = harnessRow(options.harness).editPermissions ?? '';
  const where = empty ? `Write the first program in ${relative}.\n`
    : `Write only the change in ${relative}. ${currentName} is the pipeline standing now: read it, but do not copy it into ${relative} or edit it; it is read-only and is regenerated after each approval.\nImport the actors you refer to from "circular:current" and declare only what is new or different.\n`;
  const instructions = `\nEdit task (these approval rules govern this edit session):\n${permissions}${task}\n${options.instruction ?? 'Review the current SDK code and prepare the requested change.'}\n\n${where}Save code only during the harness turn. The user reviews the program before execution.\nDo not run deploy.mjs or contact the daemon during the proposal turn.\nAfter approval, run node deploy.mjs --approve --program ${relative}; the parent CLI also accepts --session with --approve.\nThe installed host performs admission and BeginEpoch -> ValidateEpoch -> CommitEpoch.\nOmitting an actor from a program does not retire it: use the existing SDK current handles for explicit removal.\n`;
  fs.appendFileSync(path.join(opened.directory, 'first-message.md'), instructions);
  write(path.join(opened.directory, 'deploy.mjs'), `import { main } from ${JSON.stringify(new URL('./cli.mjs', import.meta.url).href)};\nprocess.exitCode = await main(['--state', ${JSON.stringify(options.state)}, '--session', import.meta.dirname, ...process.argv.slice(2)]);\n`);
  return { ...opened, current: baseline, proposal };
}

export function sessionDirectory(state, directory) {
  const root = path.join(validateState(state), CHAT_DIRECTORY) + path.sep;
  const actual = fs.realpathSync(directory);
  if (!actual.startsWith(root)) throw new Error('--session must be inside this state\'s chat directory');
  const stat = fs.statSync(actual);
  if (!stat.isDirectory() || stat.uid !== process.getuid() || (stat.mode & 0o777) !== 0o700) {
    throw new Error('session must be an owned directory with mode 0700');
  }
  return actual;
}

export function rollbackProgram(state, file) {
  const actual = fs.realpathSync(file);
  const directory = sessionDirectory(state, path.dirname(actual));
  if (path.basename(actual) !== 'current.ts' || !fs.existsSync(path.join(directory, 'committed.txt'))) {
    throw new Error('--rollback requires the preserved current.ts from a committed approval');
  }
  return actual;
}

export async function approveProgram({ state, directory, file, approve }, dependencies = {}) {
  if (approve !== true) throw new Error('deployment requires --approve');
  directory = sessionDirectory(state, directory);
  const program = loadProgram(file, directory);
  const session = await (dependencies.openSession ?? openAuthoringSession)(state);
  let committed = false, rollback, submitted, standingFile;
  try {
    const snapshot = await session.authoringSnapshot([], 256);
    const commits = await openCommits(session, cutOf(snapshot));
    const current = await snapshotFromSession({
      authoringSnapshot: async () => snapshot,
      exchange: (...args) => session.exchange(...args),
    });
    const history = fs.mkdtempSync(path.join(directory, 'approval-'));
    const recovered = current.program ?? { entry: 'main.ts', modules: new Map([['main.ts', current.source]]) };
    rollback = copyProgram(recovered, history, 'current.ts', { cursor: current.program ? current.cursor : null });
    submitted = copyProgram(program, path.join(history, 'submitted'), program.entry);
    const result = await (dependencies.executeProgram ?? executeProgram)(session, program, snapshot);
    if (result.status === 'unknown') {
      throw Object.assign(new EpochRunnerError('EPOCH_COMMIT_UNKNOWN', result.diagnostics.map(d => diagnosticLine(d)).join('; ')),
        { commitUnknown: true, commitId: result.commitId });
    }
    if (result.status !== 'committed') throw refusal(result.diagnostics);
    committed = true;
    write(path.join(history, 'committed.txt'), 'Approved SDK program committed. current.ts is the pre-execution recovery copy.\n');
    let standing;
    try { standing = await snapshotFromSession(session); }
    catch (error) {
      replaceCurrent(directory, null);
      error.code = 'CURRENT_RECONSTRUCTION_FAILED';
      throw error;
    }
    standingFile = replaceCurrent(directory, standing);
    const delta = deltaSummary(await commitsAfter(session, commits, { sent: result.commit }));
    if (result.adoption.status !== 'adopted') {
      throw Object.assign(new Error(`commit accepted; adoption ${result.adoption.status} (code ${result.adoption.code})`),
        { code: result.adoption.code, adoption: result.adoption });
    }
    return { committed: true, current: standingFile, previous: rollback, program: submitted, delta };
  } catch (error) {
    error.commitAccepted = committed;
    if (committed) {
      error.current = standingFile ?? null;
      error.previous = rollback;
      error.program = submitted;
    }
    throw error;
  } finally { await session.goodbye().catch(() => session.close()); }
}
