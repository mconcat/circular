#!/usr/bin/env node
import { spawn } from 'node:child_process';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs/promises';
import electron from 'electron';
import { PNG } from 'pngjs';
import pixelmatch from 'pixelmatch';
import { sceneDefinition } from '../renderer/scenes.mjs';

const here = path.resolve(import.meta.dirname, '..');
const args = process.argv.slice(2);
const option = name => args.includes(name) ? args[args.indexOf(name) + 1] : undefined;
const scene = option('--scene') ?? 'canvas';
sceneDefinition(scene);
const themes = option('--theme') ? [option('--theme')] : ['light', 'dark'];
if (themes.some(theme => !['light', 'dark'].includes(theme))) throw new Error('--theme must be light or dark');
const fixture = !args.includes('--state');
const sdkFixture = args.includes('--sdk-fixture');
const directory = path.resolve(option('--out') ?? path.join(here, '.shots', 'app'));
await fs.mkdir(directory, { recursive: true });
for (const theme of themes) {
  const stem = `${scene}-${sdkFixture ? 'sdk-' : fixture ? '' : 'daemon-'}${theme}`;
  const output = path.join(directory, `${stem}.png`);
  const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'circular-shot-'));
  const child = spawn(electron, [sdkFixture ? path.join(here, 'scripts/sdk-fixture-main.mjs') : here, '--capture', output, '--scene', scene, '--theme', theme,
    ...(sdkFixture ? [] : fixture ? ['--fixture'] : ['--state', option('--state')]), `--user-data-dir=${profile}`], { stdio: 'inherit' });
  try {
    await new Promise((resolve, reject) => {
      child.once('error', reject);
      child.once('exit', (code, signal) => code === 0 ? resolve() : reject(new Error(`Electron ${code ?? signal}`)));
    });
  } finally { await fs.rm(profile, { recursive: true, force: true }); }
  if (!fixture) continue;
  const referencePath = path.join(here, `design/appearance-shots/${scene}-${theme}.png`);
  const reference = PNG.sync.read(await fs.readFile(referencePath));
  const capture = PNG.sync.read(await fs.readFile(output));
  if ([reference, capture].some(p => p.width !== 1600 || p.height !== 1050)) {
    throw new Error('Both reference and capture must be 1600 × 1050; no scaling is allowed.');
  }
  const {width, height} = capture, diff = new PNG({width,height});
  const changed = pixelmatch(reference.data, capture.data, diff.data, width, height, {threshold:0.1});
  let exact = 0;
  for (let i=0; i<capture.data.length; i+=4) {
    if (capture.data.subarray(i,i+4).some((byte, channel) => byte !== reference.data[i+channel])) exact++;
  }
  const pair = new PNG({width:width*2, height});
  PNG.bitblt(reference, pair, 0, 0, width, height, 0, 0);
  PNG.bitblt(capture, pair, 0, 0, width, height, width, 0);
  await fs.writeFile(path.join(directory, `${stem}-diff.png`), PNG.sync.write(diff));
  await fs.writeFile(path.join(directory, `${stem}-pair.png`), PNG.sync.write(pair));
  console.log(JSON.stringify({scene,theme,width,height,pixels:width*height,exactChangedPixels:exact,
    exactChangedPercent:100*exact/(width*height),pixelmatchChangedPixels:changed,
    pixelmatchChangedPercent:100*changed/(width*height),threshold:0.1,
    note:'Numbers are evidence; layout, hierarchy and palette still require visual approval.'},null,2));
}
