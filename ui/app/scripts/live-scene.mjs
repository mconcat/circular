#!/usr/bin/env node
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { establish, OWNER_LOCAL_RESOURCE_CEILINGS } from '@circular/client';
import { connect } from '../bridge/frames.mjs';
import { canvasHello } from '../renderer/session.mjs';
import { readSurface, sceneDefinition } from '../renderer/scenes.mjs';

export async function liveScene(state, name) {
  sceneDefinition(name);
  if (!state || !path.isAbsolute(state)) throw new Error('An absolute isolated state is required');
  const userState = path.join(os.homedir(), 'Library/Application Support/Circular');
  const isUserState = candidate => candidate === userState || candidate.startsWith(`${userState}${path.sep}`);
  if (isUserState(path.resolve(state))) throw new Error('Use an isolated QA state');
  const root = await fs.realpath(state);
  if (isUserState(root)) throw new Error('Use an isolated QA state');
  const session = await establish(await connect({ state: root }), { hello: canvasHello(), resourceCeilings: OWNER_LOCAL_RESOURCE_CEILINGS });
  try { return await readSurface(session, name); }
  finally { try { await session.goodbye(); } finally { await session.close(); } }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [state, name, ...extra] = process.argv.slice(2);
  if (!name || extra.length) throw new Error('Usage: node scripts/live-scene.mjs <isolated state> <scene>');
  const surface = await liveScene(state, name);
  console.log(`scene=${surface.kind} actors=${surface.graph.nodes.length} edges=${surface.graph.edges.length}`);
  console.log(`unimplemented=${(surface.missing ?? []).join(',') || 'none'}`);
}
