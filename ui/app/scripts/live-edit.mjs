#!/usr/bin/env node
import assert from 'node:assert/strict';
import path from 'node:path';
import os from 'node:os';
import fs from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { randomBytes } from 'node:crypto';
import { declarationPayloadValue } from '@circular/protocol/declaration';
import { establish, OWNER_LOCAL_RESOURCE_CEILINGS } from '@circular/client';
import { connect } from '../bridge/frames.mjs';
import { canvasHello } from '../renderer/session.mjs';
import { editor, present } from '../renderer/edit.mjs';
import { readSurface } from '../renderer/scenes.mjs';

const accepted = result => { assert.equal(result.status,'accepted'); return result.value; };
export async function roundTrip(session, local) {
  const before = await readSurface(session,'canvas');
  const matches = before.graph.nodes.filter(n => n.address.local === local);
  assert.equal(matches.length,1,'Choose one unambiguous fixture actor local');
  assert.ok(Object.keys(matches[0].presentation).length, 'Choose an actor with an explicit presentation to restore');
  const node = matches[0], changedLabel = `${node.title} · edited`;
  const original = {kind:'SetPresentation',owner:{ actor: {arm:'absolute',value:node.address} },presentation:{collapsed:false,...node.presentation}};
  const direct = {kind:'SetPresentation',owner:{ actor: {arm:'absolute',value:node.address} },presentation:{collapsed:false,...node.presentation,label:changedLabel}};
  const gesture = present(node,{label:changedLabel});
  assert.deepEqual(declarationPayloadValue(gesture),declarationPayloadValue(direct));
  const execute = async command => {
    const snapshot = accepted(await session.authoringSnapshot(before.graph.anchor.scope,256));
    const opened = accepted(await session.declare({kind:'BeginEpoch',scope:{arm:'absolute',value:snapshot.anchor.scope},commitId:randomBytes(16),
      expectedRevision:snapshot.anchor.authoringRevision,expectedEnvironment:snapshot.anchor.environment}));
    let terminal = false;
    try {
      accepted(await session.declare(command)); accepted(await session.declare({kind:'ValidateEpoch',epoch:opened.epoch}));
      terminal = true; accepted(await session.declare({kind:'CommitEpoch',epoch:opened.epoch}));
    } finally { if (!terminal) accepted(await session.declare({kind:'AbortEpoch',epoch:opened.epoch})); }
  };
  const edit = editor(session);
  accepted(await edit.prepare(before.graph.anchor,[gesture]));
  accepted(await edit.commit());
  const fromUI = accepted(await session.authoringSnapshot(before.graph.anchor.scope,256));
  await execute(original);
  await execute(direct);
  const fromSDK = accepted(await session.authoringSnapshot(before.graph.anchor.scope,256));
  assert.deepEqual(fromUI.commands,fromSDK.commands);
  assert.deepEqual(fromUI.anchor.authoringRevision,fromSDK.anchor.authoringRevision);
  const configured = await readSurface(session,'configure',{selected:node.id});
  assert.equal(configured.configuration.title,changedLabel);
  await execute(original);
  return {commandsEqual:true,revisionEqual:true,configuredLabelEqual:true};
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [state, local, ...extra] = process.argv.slice(2);
  if (!state || !path.isAbsolute(state) || !local || extra.length) throw new Error('Usage: node scripts/live-edit.mjs <absolute isolated state> <unique actor local>');
  const userState = path.join(os.homedir(),'Library/Application Support/Circular');
  const user = candidate => candidate === userState || candidate.startsWith(`${userState}${path.sep}`);
  if (user(path.resolve(state))) throw new Error('Use an isolated QA state');
  const root = await fs.realpath(state);
  if (user(root)) throw new Error('Use an isolated QA state');
  const session = await establish(await connect({state:root}), {hello:canvasHello(), resourceCeilings:OWNER_LOCAL_RESOURCE_CEILINGS});
  try { console.log(await roundTrip(session,local)); } finally { await session.close(); }
}
