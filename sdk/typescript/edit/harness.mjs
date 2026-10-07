/** CLI execution protocol only; authoring input is the chat session SDK code. */

import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { StringDecoder } from "node:string_decoder";

import { semanticPrepass } from "@circular/authoring";

import { loadProgram } from "../chat/execution.mjs";
import { HARNESSES, headlessInvocation } from "../chat/harnesses.mjs";
import { shellQuote, workspace } from "../chat/launcher.mjs";
import { publicInstruction, referencedDocuments } from "../chat/session-instructions.mjs";

const MAX_HARNESS_OUTPUT_BYTES = 1024 * 1024;

export class EditHarnessError extends Error {
  constructor(code, detail) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.detail = detail;
  }
}

const fail = (code, message) => {
  throw new EditHarnessError(code, message);
};

function sessionReadable(cwd) {
  const shipped = publicInstruction("AGENTS.md");
  const documents = shipped.path === null
    ? [] : referencedDocuments(path.dirname(shipped.path), path.join(workspace, "templates"));
  const readable = documents.filter(row => row.path !== null).map(row => {
    const chosen = row.path.indexOf(`${path.sep}<name>`);
    if (chosen >= 0) return row.path.slice(0, chosen);
    return row.path.endsWith(path.sep) ? row.path.slice(0, -1) : row.path;
  });
  const modules = path.join(cwd, "node_modules");
  if (fs.existsSync(modules)) readable.push(modules, ...linkedTargets(modules));
  return readable;
}

function programImage(proposal, cwd, { prepass = false } = {}) {
  let program;
  try {
    program = loadProgram(proposal, cwd);
  } catch (error) {
    return { broken: error.message };
  }
  let empty;
  if (prepass) {
    const prepared = semanticPrepass(program);
    if (prepared.status !== "complete") {
      const first = prepared.diagnostics?.[0];
      return { broken: `semanticPrepass: ${[first?.message, ...(first?.args ?? [])].filter(Boolean).join(": ") || prepared.status}` };
    }
    const { calls, bindings, surfaces } = prepared.value;
    empty = calls.length === 0 && bindings.length === 0 && surfaces.length === 0;
  }
  return { empty, image: JSON.stringify([...program.modules].map(([name, bytes]) => [name, Buffer.from(bytes).toString("base64")])) };
}

function linkedTargets(modules) {
  const targets = [];
  const visit = entry => {
    if (!fs.lstatSync(entry).isSymbolicLink()) return false;
    try { targets.push(fs.realpathSync(entry)); } catch {   }
    return true;
  };
  if (visit(modules)) return targets;
  for (const name of fs.readdirSync(modules)) {
    if (name.startsWith(".")) continue;
    const entry = path.join(modules, name);
    if (visit(entry) || !name.startsWith("@")) continue;
    for (const scoped of fs.readdirSync(entry)) visit(path.join(entry, scoped));
  }
  return targets;
}

export function harnessInvocation(harness, prompt, readable = []) {
  const row = HARNESSES[harness];
  if (!row) fail("EDIT_HARNESS", `execution protocol is unavailable for reported harness ${harness}`);
  const invocation = headlessInvocation(row, prompt);
  const grant = row.headless.readable?.(readable) ?? [];
  return Object.freeze({
    args: Object.freeze([...invocation.args, ...grant]),
    stdin: invocation.stdin === null ? null : Buffer.from(invocation.stdin),
    removeEnvironment: invocation.removeEnvironment,
    output: invocation.output,
  });
}

function deniedOf(result) {
  return Object.freeze((result?.denied ?? []).map(entry => Object.freeze({
    tool: entry.tool, code: "EDIT_HARNESS_PERMISSION", reason: entry.reason ?? null, summary: entry.summary,
  })));
}

function refuse(code, detail, seen, { listDenied = true } = {}) {
  const notes = [
    ...(listDenied && seen.denied.length > 0 ? [`denied tool calls: ${JSON.stringify(seen.denied)}`] : []),
    ...(seen.unreadable > 0 ? [`${seen.unreadable} stdout lines could not be read`] : []),
  ];
  throw Object.assign(new EditHarnessError(code, [detail, ...notes].join("; ")), seen);
}

function settle(harness, result, seen) {
  if (result?.failed === true) refuse("EDIT_HARNESS_TURN", `${harness} marked the turn as an error`, seen);
  if (typeof result?.output !== "string") refuse("EDIT_HARNESS_OUTPUT", `${harness} returned no final result`, seen);
  return { output: result.output, harnessSession: result.session ?? null, ...seen };
}

function parseText(stdout) {
  return { output: stdout.trim(), harnessSession: null, denied: Object.freeze([]), unreadable: 0 };
}

function executablePath(program) {
  if (typeof program !== "string" || !path.isAbsolute(program)) {
    fail("EDIT_HARNESS_PATH", "the harness executable must be an absolute path");
  }
  let canonical;
  try {
    canonical = fs.realpathSync(program);
    fs.accessSync(canonical, fs.constants.X_OK);
  } catch (error) {
    fail("EDIT_HARNESS_PATH", `${program} is not an executable file: ${error.code ?? error.message}`);
  }
  if (!fs.statSync(canonical).isFile()) fail("EDIT_HARNESS_PATH", `${canonical} is not a regular file`);
  return canonical;
}

function appendBounded(chunks, chunk, state, child) {
  state.bytes += chunk.length;
  if (state.bytes > MAX_HARNESS_OUTPUT_BYTES) {
    state.overflow = true;
    child.kill("SIGTERM");
    return;
  }
  chunks.push(chunk);
}

export function harnessEnvironment(base, removeEnvironment = []) {
  const environment = { ...base };
  for (const name of Object.keys(environment)) {
    if (name.startsWith("CIRCULAR_")) delete environment[name];
  }
  for (const name of removeEnvironment) delete environment[name];
  return environment;
}

export async function runEditHarness({ harness, program, prompt, cwd, timeoutMs, onEvent, proposal }) {
  const executable = executablePath(program);
  if (!path.isAbsolute(cwd)) fail("EDIT_HARNESS_CWD", "the edit-session directory must be absolute");
  const invocation = harnessInvocation(harness, prompt, sessionReadable(cwd));
  const environment = harnessEnvironment(process.env, invocation.removeEnvironment);
  const deploy = path.join(cwd, "deploy.mjs");
  const where = Object.freeze({
    session: cwd,
    proposal: proposal ?? null,
    apply: !fs.existsSync(deploy) ? null
      : `node ${shellQuote(deploy)} --approve${proposal === undefined ? "" : ` --program ${shellQuote(path.relative(cwd, proposal))}`}`,
  });
  try {
    const before = proposal === undefined ? null : programImage(proposal, cwd);
    const proposalState = () => {
      if (before === null) return "no proposal file was named to check";
      const after = programImage(proposal, cwd, { prepass: true });
      if (after.broken !== undefined) return `the proposal is broken: ${after.broken}`;
      if (after.image === before.image) return "the proposal is unchanged since the turn began";
      if (after.empty) return "the proposal is empty: it declares no call, binding, output surface or note";
      return null;
    };
    const done = await turn({ harness, executable, invocation, environment, cwd, timeoutMs, onEvent });
    const verdict = proposalState();
    if (verdict !== null) {
      if (done.denied.length > 0) refuse("EDIT_HARNESS_PERMISSION",
        `${harness} denied tool calls this session does not allow: ${JSON.stringify(done.denied)}; ${verdict}`, done, { listDenied: false });
      refuse("EDIT_HARNESS_OUTPUT", verdict, done);
    }
    return Object.freeze({ status: "ready", ...where, ...done });
  } catch (error) {
    if (!(error instanceof EditHarnessError)) throw error;
    const kept = new EditHarnessError(error.code, `${error.detail}; the session directory ${shellQuote(cwd)} keeps what `
      + `the harness saved${where.apply === null ? "" : ` — review it, then apply it with: ${where.apply}`}`);
    throw Object.assign(kept, where, { denied: error.denied ?? Object.freeze([]), unreadable: error.unreadable ?? 0 });
  }
}

async function turn({ harness, executable, invocation, environment, cwd, timeoutMs, onEvent }) {
  const child = spawn(executable, invocation.args, {
    cwd,
    env: environment,
    stdio: ["pipe", "pipe", "pipe"],
  });
  const stdout = [];
  const stderr = [];
  const stdoutState = { bytes: 0, overflow: false };
  const stderrState = { bytes: 0, overflow: false };
  const lines = invocation.output.format === "jsonl";
  const decoder = new StringDecoder("utf8");
  let partial = "";
  let result;
  let unreadable = 0;
  const read = (line) => {
    if (line.trim() === "") return;
    let next, events;
    try {
      const value = JSON.parse(line);
      if (value === null || typeof value !== "object") throw new TypeError("not a JSON object");
      next = invocation.output.result(value, result);
      events = invocation.output.events(value);
    } catch {
      unreadable += 1;
      return;
    }
    result = next;
    if (!onEvent) return;
    for (const event of events) onEvent(Object.freeze({ harness, type: event.type, summary: event.summary }));
  };
  child.stdout.on("data", (chunk) => {
    if (!lines) return appendBounded(stdout, chunk, stdoutState, child);
    if (stdoutState.overflow) return;
    partial += decoder.write(chunk);
    for (let index = partial.indexOf("\n"); index >= 0; index = partial.indexOf("\n")) {
      read(partial.slice(0, index));
      partial = partial.slice(index + 1);
    }
    if (Buffer.byteLength(partial) > MAX_HARNESS_OUTPUT_BYTES) {
      stdoutState.overflow = true;
      child.kill("SIGTERM");
    }
  });
  child.stderr.on("data", (chunk) => appendBounded(stderr, chunk, stderrState, child));
  if (invocation.stdin === null) child.stdin.end();
  else child.stdin.end(invocation.stdin);

  let stopped = null;
  const forward = (signal) => {
    child.kill(stopped === null ? signal : "SIGKILL");
    stopped ??= signal;
  };
  process.on("SIGINT", forward);
  process.on("SIGTERM", forward);
  const timer = timeoutMs === undefined ? null : setTimeout(() => {
    stopped ??= "timeout";
    child.kill("SIGTERM");
  }, timeoutMs);
  let settled;
  try {
    settled = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("close", (code, signal) => resolve({ code, signal }));
    });
  } finally {
    clearTimeout(timer);
    process.off("SIGINT", forward);
    process.off("SIGTERM", forward);
  }

  if (lines && !stdoutState.overflow) {
    partial += decoder.end();
    if (partial.trim() !== "") read(partial);
  }
  const seen = Object.freeze({ denied: deniedOf(result), unreadable });
  if (stopped === "timeout") refuse("EDIT_HARNESS_TIMEOUT", `${harness} exceeded --timeout-ms ${timeoutMs}`, seen);
  if (stopped !== null) refuse("EDIT_HARNESS_STOPPED", `${harness} was stopped by ${stopped}`, seen);
  if (stdoutState.overflow || stderrState.overflow) {
    const where = stderrState.overflow ? "stderr" : lines ? "one stdout line" : "stdout";
    refuse("EDIT_HARNESS_OUTPUT_LIMIT", `${harness} exceeded the 1 MiB bound on ${where}`, seen);
  }
  const stderrText = Buffer.concat(stderr).toString("utf8");
  if (settled.code !== 0) {
    const diagnostic = stderrText.trim().split(/\r?\n/).slice(-4).join(" | ");
    refuse(
      "EDIT_HARNESS_EXIT",
      `${harness} exited ${String(settled.code)}${settled.signal ? ` (${settled.signal})` : ""}`
        + `${diagnostic ? `: ${diagnostic}` : ""}`,
      seen,
    );
  }
  if (!lines) return parseText(Buffer.concat(stdout).toString("utf8"));
  return settle(harness, result, seen);
}
