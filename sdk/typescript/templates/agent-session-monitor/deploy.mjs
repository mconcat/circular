#!/usr/bin/env node
/** Deploys the Agent Session Monitor graph through the public owner-local declaration path. */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { connectOwnerLocal } from "@circular/client/owner-local";
import { establish } from "@circular/client";
import { runValidatedEpoch } from "@circular/client/epoch-runner";
import { INGRESS_MOUNTS } from "./source-adapter.mjs";
import { OBSERVATION_MOUNTS, agentSessionMonitorGraph } from "./graph.mjs";

const CEILINGS = Object.freeze({
  maximumBytes: 1 << 20,
  maximumDepth: 64,
  maximumContainerEntries: 4096,
  maximumStringBytes: 65536,
});

function parseArguments(argv) {
  const options = { skipPreflight: false };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--state") options.state = argv[(index += 1)];
    else if (argument === "--skip-preflight") options.skipPreflight = true;
    else {
      console.error(`unknown argument: ${argument}`);
      return null;
    }
  }
  if (!options.state) {
    console.error("usage: node deploy.mjs --state <daemon state dir> [--skip-preflight]");
    return null;
  }
  return options;
}

/** Explains missing deployment pieces before the owner-local socket is contacted. */
export function preflight({ state }) {
  const errors = [];
  const warnings = [];
  if (!path.isAbsolute(state)) {
    errors.push(`${state} is not an absolute daemon state path`);
  } else if (!fs.existsSync(path.join(state, "daemon.sock"))) {
    errors.push(`${state}/daemon.sock does not exist; start circular-daemon and wait for its claim line`);
  }
  if (!fs.existsSync(path.join(state, "config.toml"))) {
    errors.push(`${state}/config.toml does not exist; bind webhook ingress before deploying`);
  }
  warnings.push(
    "metrics are preserved and routed as unclassified until the source supplies measured outcome evidence",
  );
  return { errors, warnings };
}

export async function deploy({ state }) {
  const transport = await connectOwnerLocal({ root: state, socketName: "daemon.sock" });
  const session = await establish(transport, { hello: { requestedRoles: [1n, 4n, [2n, []]] },
    resourceCeilings: CEILINGS,
    requestTimeoutMs: 5000,
  });
  try {
    const graph = agentSessionMonitorGraph();
    const result = await runValidatedEpoch(session, graph.commands);
    return result.acceptedCommands;
  } finally {
    await session.goodbye().catch(() => session.close());
  }
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options === null) return 2;
  const { errors, warnings } = preflight(options);
  for (const warning of warnings) console.error(`preflight (warning): ${warning}`);
  for (const error of errors) console.error(`preflight: ${error}`);
  if (errors.length > 0 && !options.skipPreflight) {
    console.error("preflight refused; fix the findings above or pass --skip-preflight to inspect daemon refusal");
    return 2;
  }
  try {
    const accepted = await deploy(options);
    console.log(JSON.stringify({
      deployed: true,
      accepted_commands: accepted,
      graph: "agent-session-monitor",
      ingress_mounts: Object.values(INGRESS_MOUNTS),
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
