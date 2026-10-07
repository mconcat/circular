#!/usr/bin/env node
/**
 * Opens actor.events before an optional HTTP injection and waits for the classified observation.
 * It is also usable as a watch-only monitor while an OTLP producer forwards an actual journal.
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { randomUUID } from "node:crypto";
import { pathToFileURL } from "node:url";

import { connectOwnerLocal } from "@circular/client/owner-local";
import { establish } from "@circular/client";
import { openSubscription } from "@circular/client/subscription";
import { CLASSIFICATIONS, OBSERVATION_MOUNTS } from "./graph.mjs";
import { INGRESS_MOUNTS, SIGNALS } from "./source-adapter.mjs";

const CEILINGS = Object.freeze({
  maximumBytes: 1 << 20,
  maximumDepth: 64,
  maximumContainerEntries: 4096,
  maximumStringBytes: 65536,
});

function parseArguments(argv) {
  const options = { timeoutMs: 15000 };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--state") options.state = argv[(index += 1)];
    else if (argument === "--signal") options.signal = argv[(index += 1)];
    else if (argument === "--expect") options.expect = argv[(index += 1)];
    else if (argument === "--sample") options.sample = argv[(index += 1)];
    else if (argument === "--bind") options.bind = argv[(index += 1)];
    else if (argument === "--bearer-file") options.bearerFile = argv[(index += 1)];
    else if (argument === "--contains") options.contains = argv[(index += 1)];
    else if (argument === "--timeout-ms") options.timeoutMs = Number(argv[(index += 1)]);
    else return null;
  }
  if (!options.state || !SIGNALS.includes(options.signal) || !CLASSIFICATIONS.includes(options.expect)) return null;
  if (!Number.isSafeInteger(options.timeoutMs) || options.timeoutMs <= 0) return null;
  if (options.sample && (!options.bind || !options.bearerFile)) return null;
  return options;
}

const jsonable = (_key, item) => typeof item === "bigint" ? `${item}` : item;
const textOf = (value) => JSON.stringify(value, jsonable);

async function injectSample(options) {
  let token = fs.readFileSync(path.resolve(options.bearerFile), "utf8");
  if (token.endsWith("\n")) token = token.slice(0, -1);
  if (!token) throw new Error("--bearer-file must name a nonempty UTF-8 bearer value");
  const body = fs.readFileSync(path.resolve(options.sample));
  const response = await fetch(`http://${options.bind}/v1/ingress/${INGRESS_MOUNTS[options.signal]}`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
      "Idempotency-Key": `agent-session-monitor-${randomUUID()}`,
    },
    body,
  });
  if (response.status !== 202) {
    const detail = (await response.text()).trim().slice(0, 512);
    throw new Error(`sample injection returned HTTP ${response.status}${detail ? `: ${detail}` : ""}`);
  }
}

export async function liveSmoke(options) {
  const transport = await connectOwnerLocal({ root: options.state, socketName: "daemon.sock" });
  const session = await establish(transport, { hello: { requestedRoles: [1n, 4n, [2n, []]] },
    resourceCeilings: CEILINGS,
    requestTimeoutMs: 5000,
  });
  let feed;
  try {
    feed = await openSubscription(session, { target: "actor.events", args: null, initialCredit: 256 });
    if (feed.ack !== 1n) throw new Error(`actor.events subscription was rejected: ${textOf(feed.ack)}`);
    if (options.sample) await injectSample(options);

    const wantedActor = `observed_${options.signal}`;
    const joinActor = `parse_${options.signal}`;
    const deadline = Date.now() + options.timeoutMs;
    let frames = 0;
    const candidates = [];
    const visitedActors = [];
    let containsMatched = options.contains === undefined;
    let matchedObservation = null;
    while (Date.now() < deadline) {
      const frame = await feed.receive(Math.min(1000, Math.max(1, deadline - Date.now())));
      if (frame === null) continue;
      frames += 1;
      const actor = frame.payload?.actor?.local;
      const body = frame.payload?.body;
      if (typeof actor === "string") visitedActors.push(actor);
      if (actor === joinActor && options.contains !== undefined && textOf(body).includes(options.contains)) {
        containsMatched = true;
      }
      if (actor === wantedActor) {
        candidates.push(body?.classification ?? null);
        if (body?.classification === options.expect) matchedObservation = body;
      }
      if (matchedObservation !== null && containsMatched) {
        return {
          observed: true,
          actor: wantedActor,
          signal: matchedObservation.signal,
          classification: matchedObservation.classification,
          evidence: matchedObservation.evidence,
          contains_matched: options.contains === undefined ? null : true,
          frames_read: frames,
        };
      }
    }
    throw new Error(
      `timed out waiting for ${wantedActor} classification=${options.expect}; `
        + `frames=${frames} observed_candidates=${textOf(candidates)} visited_actors=${textOf(visitedActors)}`,
    );
  } finally {
    feed?.release();
    await session.goodbye().catch(() => session.close());
  }
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options === null) {
    console.error(
      "usage: node live-smoke.mjs --state <daemon state> --signal <logs|metrics> "
        + "--expect <normal|warning|error|unclassified> "
        + "[--sample file --bind host:port --bearer-file file] [--contains text] [--timeout-ms n]",
    );
    return 2;
  }
  try {
    console.log(JSON.stringify(await liveSmoke(options), jsonable));
    return 0;
  } catch (error) {
    console.error(error.message ?? error);
    return 1;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(fs.realpathSync(process.argv[1])).href) {
  process.exitCode = await main();
}
