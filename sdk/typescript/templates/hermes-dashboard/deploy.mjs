#!/usr/bin/env node
/** Deploys the Hermes Dashboard graph through the public owner-local declaration path. */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { connectOwnerLocal } from "@circular/client/owner-local";
import { establish } from "@circular/client";
import { runValidatedEpoch } from "@circular/client/epoch-runner";
import { DEFAULTS, OBSERVATION_MOUNTS, hermesDashboardGraph, hermesOptions } from "./graph.mjs";

const CEILINGS = Object.freeze({
  maximumBytes: 1 << 20,
  maximumDepth: 64,
  maximumContainerEntries: 4096,
  maximumStringBytes: 65536,
});

const USAGE = "usage: node deploy.mjs --state <daemon state dir> --root <hermes profiles dir>"
  + " --agents <name,name,…> [--log-file gateway.log] [--poll-ms 2000] [--window-ms 300000]"
  + " [--emission-period-ms 60000] [--stall-quiet-ms 300000] [--glob <pattern>] [--skip-preflight]";

/** Milliseconds reach the wire as `Int`, so they are parsed as BigInt here and never as a Number. */
const MILLISECOND_FLAGS = Object.freeze({
  "--poll-ms": "pollMs",
  "--window-ms": "windowMs",
  "--emission-period-ms": "emissionPeriodMs",
  "--stall-quiet-ms": "stallQuietMs",
});

export function parseArguments(argv) {
  const options = { skipPreflight: false };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--state") options.state = argv[(index += 1)];
    else if (argument === "--root") options.root = argv[(index += 1)];
    else if (argument === "--glob") options.glob = argv[(index += 1)];
    else if (argument === "--log-file") options.logFile = argv[(index += 1)];
    else if (argument === "--agents") options.agents = (argv[(index += 1)] ?? "").split(",").filter(Boolean);
    else if (argument === "--skip-preflight") options.skipPreflight = true;
    else if (argument in MILLISECOND_FLAGS) {
      const text = argv[(index += 1)] ?? "";
      if (!/^[0-9]+$/.test(text) || BigInt(text) <= 0n) {
        console.error(`${argument} takes a positive whole number of milliseconds`);
        return null;
      }
      options[MILLISECOND_FLAGS[argument]] = BigInt(text);
    } else {
      console.error(`unknown argument: ${argument}`);
      return null;
    }
  }
  if (!options.state || !options.root || !options.agents) {
    console.error(USAGE);
    return null;
  }
  return options;
}

/** Explains missing deployment pieces before the owner-local socket is contacted. */
export function preflight({ state, root, glob, logFile = DEFAULTS.logFile, agents = [] }) {
  const errors = [];
  const warnings = [];
  if (!path.isAbsolute(state)) {
    errors.push(`${state} is not an absolute daemon state path`);
  } else if (!fs.existsSync(path.join(state, "daemon.sock"))) {
    errors.push(`${state}/daemon.sock does not exist; start circular-daemon and wait for its claim line`);
  }
  if (!path.isAbsolute(root)) {
    errors.push(`${root} is not an absolute Hermes profiles directory`);
  } else if (!fs.existsSync(root)) {
    errors.push(`${root} does not exist; it is the FsRead capability root the listener is granted`);
  } else if (glob === undefined) {
    for (const agent of agents) {
      const file = path.join(root, agent, "logs", logFile);
      if (!fs.existsSync(file)) warnings.push(`${file} does not exist yet; that agent's lane stays idle until it does`);
    }
  }
  warnings.push(
    "the fleet activity row counts parsed lines per window; a line the two patterns do not match"
      + " is observed on the unparsed mount instead of being silently dropped",
  );
  return { errors, warnings };
}

export async function deploy(options) {
  const transport = await connectOwnerLocal({ root: options.state, socketName: "daemon.sock" });
  const session = await establish(transport, {
    hello: { requestedRoles: [1n, 4n, [2n, []]] },
    resourceCeilings: CEILINGS,
    requestTimeoutMs: 5000,
  });
  try {
    const graph = hermesDashboardGraph(options);
    const result = await runValidatedEpoch(session, graph.commands);
    return result.acceptedCommands;
  } finally {
    await session.goodbye().catch(() => session.close());
  }
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options === null) return 2;
  let resolved;
  try {
    resolved = hermesOptions(options);
  } catch (error) {
    console.error(error.message ?? error);
    return 2;
  }
  const { errors, warnings } = preflight({ ...resolved, glob: options.glob });
  for (const warning of warnings) console.error(`preflight (warning): ${warning}`);
  for (const error of errors) console.error(`preflight: ${error}`);
  if (errors.length > 0 && !options.skipPreflight) {
    console.error("preflight refused; fix the findings above or pass --skip-preflight to inspect daemon refusal");
    return 2;
  }
  try {
    const accepted = await deploy({ ...options, state: resolved.state });
    console.log(JSON.stringify({
      deployed: true,
      accepted_commands: accepted,
      graph: "hermes-dashboard",
      agents: resolved.agents,
      glob: resolved.glob,
      observation_mounts: Object.values(OBSERVATION_MOUNTS),
    }));
    return 0;
  } catch (error) {
    console.error(error.message ?? error);
    const { exitFor } = await import("../../cli/common.mjs");
    return exitFor(error);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(fs.realpathSync(process.argv[1])).href) {
  process.exitCode = await main();
}
