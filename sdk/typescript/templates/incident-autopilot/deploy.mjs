#!/usr/bin/env node
/**
 * Deploys the Incident Autopilot template to a running daemon.
 *
 *   node deploy.mjs --state <daemon state dir> --remediator <abs path> --verifier <abs path>
 *
 * Runs a preflight first, so missing executable files and daemon state are explained before the
 * owner-local socket is contacted. Capability bindings and secret references are owned by the
 * daemon's owner-private `<state>/config.toml`; the daemon is the authoritative parser for it.
 * Pass `--skip-preflight` to deploy anyway.
 *
 * Exit codes: 0 deployed and committed · 1 a command was rejected, or any other failure after the
 * checks, such as a daemon.sock that exists with no daemon answering on it · 2 usage, the preflight
 * refused, or the state directory has no daemon.sock · 4 CommitEpoch was sent, its answer did not
 * arrive and the commit record does not show it.
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { connectOwnerLocal } from "@circular/client/owner-local";
import { establish } from "@circular/client";
import { runValidatedEpoch } from "@circular/client/epoch-runner";
import { INGRESS_MOUNT, RESOLVED_MOUNT, incidentGraph } from "./graph.mjs";

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
    else if (argument === "--remediator") options.remediator = argv[(index += 1)];
    else if (argument === "--verifier") options.verifier = argv[(index += 1)];
    else if (argument === "--skip-preflight") options.skipPreflight = true;
    else {
      console.error(`unknown argument: ${argument}`);
      return null;
    }
  }
  if (!options.state || !options.remediator || !options.verifier) {
    console.error(
      "usage: node deploy.mjs --state <daemon state dir> --remediator <abs path> --verifier <abs path> [--skip-preflight]",
    );
    return null;
  }
  return options;
}

/**
 * Says what is missing, in terms of what to do about it.
 *
 * `errors` refuse the deploy; `warnings` name checks deliberately left to the daemon's one
 * authoritative config parser.
 */
export function preflight({ state, remediator, verifier }) {
  const errors = [];
  const warnings = [];
  for (const [role, program] of [["remediator", remediator], ["verifier", verifier]]) {
    if (!path.isAbsolute(program)) {
      errors.push(`${role} ${program} is not an absolute path; the daemon allowlist matches exact absolute paths`);
      continue;
    }
    try {
      fs.accessSync(program, fs.constants.X_OK);
    } catch {
      errors.push(`${role} ${program} is missing or not executable on this host`);
    }
  }
  if (!fs.existsSync(path.join(state, "config.toml"))) {
    errors.push(`${state}/config.toml does not exist; bind process, notify, and webhook capabilities before deploying`);
  }
  warnings.push("config.toml contents are validated by circular-daemon, not duplicated in this deploy client");
  return { errors, warnings };
}

export async function deploy({ state, remediator, verifier }) {
  const transport = await connectOwnerLocal({ root: state, socketName: "daemon.sock" });
  const session = await establish(transport, { hello: { requestedRoles: [1n, 4n, [2n, []]] },
    resourceCeilings: CEILINGS,
    requestTimeoutMs: 5000,
  });
  try {
    const graph = incidentGraph({ remediator, verifier });
    const result = await runValidatedEpoch(session, graph.commands);
    return result.acceptedCommands;
  } finally {
    await session.goodbye().catch(() => session.close());
  }
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options === null) process.exit(2);
  const { errors, warnings } = preflight(options);
  for (const warning of warnings) console.error(`preflight (warning): ${warning}`);
  for (const error of errors) console.error(`preflight: ${error}`);
  if (errors.length > 0 && !options.skipPreflight) {
    console.error("preflight refused; fix the findings above or pass --skip-preflight to deploy anyway");
    process.exit(2);
  }
  if (!fs.existsSync(path.join(options.state, "daemon.sock"))) {
    console.error(`${options.state}/daemon.sock does not exist; start the daemon and wait for its claim line`);
    process.exit(2);
  }
  try {
    const accepted = await deploy(options);
    console.log(
      `deployed incident-autopilot: ${accepted} commands accepted; ` +
        `POST /v1/ingress/${INGRESS_MOUNT} feeds it and "${RESOLVED_MOUNT}" holds the resolved arrivals`,
    );
  } catch (error) {
    console.error(error.message ?? error);
    const { exitFor } = await import("../../cli/common.mjs");
    process.exit(exitFor(error));
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(fs.realpathSync(process.argv[1])).href) {
  await main();
}
