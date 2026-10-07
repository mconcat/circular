#!/usr/bin/env node
/**
 * Deploys the Astronomy Shop on-call demo to a running daemon.
 *
 *   node deploy.mjs --state <daemon state dir> [--prometheus <url>] [--demo <url>]
 *
 * It fills the three values in program.ts (this template's directory, the demo's Prometheus
 * and the demo's web address) and runs the result through the same installed host as
 * `circular edit --approve`. The program is SDK code; nothing else is deployed.
 *
 * The demo goes into an empty pipeline: a state that already has actors is refused, because a
 * template's actors would replace any of the same name. Use a fresh state directory.
 *
 * Exit codes: 0 committed · 1 the daemon refused the program, or any other failure, such as a
 * --prometheus or --demo value that is not a plain http(s) URL, or a daemon.sock that exists with
 * no daemon answering on it · 2 usage, a missing bin/ tool, a state directory with no daemon.sock,
 * or a state that already has a pipeline · 4 the commit was sent, its answer did not arrive and the
 * commit record does not show it.
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = path.dirname(fs.realpathSync(fileURLToPath(import.meta.url)));
const USAGE = "usage: node deploy.mjs --state <daemon state dir> [--prometheus <url>] [--demo <url>]";
export const DEFAULTS = Object.freeze({ prometheus: "http://127.0.0.1:9090", demo: "http://127.0.0.1:8080" });

export function parseArguments(argv) {
  const options = { ...DEFAULTS };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const value = argv[index + 1];
    if (!["--state", "--prometheus", "--demo"].includes(argument) || value === undefined || value.startsWith("--")) {
      return null;
    }
    options[argument.slice(2)] = value;
    index += 1;
  }
  return options.state ? options : null;
}

/** A value lands inside a string literal of the program, so it must be a plain one. */
function literal(value, what) {
  if (/["'`\\\u0000-\u001f\u007f]/.test(value)) throw new Error(`${what} ${JSON.stringify(value)} holds a quote, a backslash or a control character`);
  return value;
}

function base(url, what) {
  let parsed;
  try { parsed = new URL(url); } catch { throw new Error(`${what} ${JSON.stringify(url)} is not a URL`); }
  if (!["http:", "https:"].includes(parsed.protocol)) throw new Error(`${what} ${url} is not an http or https URL`);
  return literal(url.replace(/\/+$/, ""), what);
}

/** program.ts with its three values filled in, as the one module of an SDK program. */
export function astronomyProgram({ prometheus, demo, template = HERE }) {
  const values = { $TEMPLATE: literal(template, "template directory"), $PROMETHEUS: base(prometheus, "--prometheus"), $DEMO: base(demo, "--demo") };
  let source = fs.readFileSync(path.join(HERE, "program.ts"), "utf8");
  for (const [name, value] of Object.entries(values)) source = source.replaceAll(name, value);
  return { entry: "main.ts", modules: new Map([["main.ts", new TextEncoder().encode(source)]]) };
}

/** What the operator is told before anything is sent: a finding here does not stop the deploy. */
async function reachable(url) {
  try {
    const response = await fetch(url, { signal: AbortSignal.timeout(5000) });
    return response.ok ? null : `answered ${response.status}`;
  } catch (error) {
    return error.cause?.code ?? error.name;
  }
}

export async function deploy(options) {
  const program = astronomyProgram(options);
  for (const tool of ["page", "flag-off"]) {
    try { fs.accessSync(path.join(HERE, "bin", tool), fs.constants.X_OK); }
    catch { throw Object.assign(new Error(`${path.join(HERE, "bin", tool)} is missing or not executable`), { exit: 2 }); }
  }
  for (const [what, url] of [["Prometheus", `${options.prometheus.replace(/\/+$/, "")}/-/ready`],
    ["the demo's flag editor", `${options.demo.replace(/\/+$/, "")}/feature/api/read-file`]]) {
    const problem = await reachable(url);
    if (problem) console.error(`warning: ${what} at ${url} did not answer (${problem}); the pipeline deploys anyway and its requests fail until it does`);
  }
  if (!fs.existsSync(path.join(options.state, "daemon.sock"))) {
    throw Object.assign(new Error(`${options.state}/daemon.sock does not exist; start the daemon for this state first`), { exit: 2 });
  }
  const { diagnosticLine, executeProgram, openAuthoringSession } = await import("../../chat/execution.mjs");
  const session = await openAuthoringSession(options.state);
  try {
    const snapshot = await session.authoringSnapshot([], 256);
    if (snapshot.status !== 'accepted') throw new Error(`authoring snapshot refused (${snapshot.reason ?? snapshot.status}): ${[...(snapshot.diagnostics ?? []), snapshot.diagnostic].filter(Boolean).map(d => `${d.code}: ${d.message}`).join('; ')}`);
    const actors = snapshot.value.commands.filter((command) => command.kind === "UpsertActor").length;
    if (actors > 0) {
      throw Object.assign(new Error(`this state already has ${actors} actors; deploy the demo into a fresh state directory`), { exit: 2 });
    }
    const result = await executeProgram(session, program, snapshot);
    if (result.status !== "committed") {
      throw Object.assign(new Error(result.diagnostics.map((diagnostic) => diagnosticLine(diagnostic)).join("\n")),
        result.status === "unknown" ? { commitUnknown: true, commitId: result.commitId } : { exit: 1 });
    }
    if (result.adoption.status !== 'adopted') {
      throw Object.assign(new Error(`commit accepted; adoption ${result.adoption.status} (code ${result.adoption.code})`),
        { code: result.adoption.code, adoption: result.adoption, commitAccepted: true });
    }
    return result;
  } finally {
    await session.goodbye().catch(() => session.close());
  }
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options === null) {
    console.error(USAGE);
    process.exit(2);
  }
  try {
    await deploy(options);
    console.log(`deployed otel-astronomy: the pipeline polls ${options.prometheus} every 15 seconds; `
      + `open this state in the app to watch it`);
  } catch (error) {
    for (const line of String(error.message ?? error).split("\n")) console.error(`otel-astronomy: ${line}`);
    const { exitFor } = await import("../../cli/common.mjs");
    process.exit(error.exit ?? exitFor(error));
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(fs.realpathSync(process.argv[1])).href) {
  await main();
}
