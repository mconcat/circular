import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawn } from 'node:child_process';

const appRoot = path.resolve(import.meta.dirname, '..');
const chrome = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

const outputs = paper => [
  { file: 'build/icon.png', transparent: false, body: `
  html, body { margin: 0; width: 1024px; height: 1024px; overflow: hidden; background: ${paper}; }
  body { display: grid; place-items: center; }
  img { display: block; width: 636px; height: auto; }`, markup: mark => mark },
  { file: 'build/dock.png', transparent: true, body: `
  html, body { margin: 0; width: 1024px; height: 1024px; overflow: hidden; background: transparent; }
  body { display: grid; place-items: center; }
  div { display: grid; place-items: center; width: 824px; height: 824px; border-radius: 185px; background: ${paper}; }
  img { display: block; width: 512px; height: auto; }`, markup: mark => `<div>${mark}</div>` },
];

async function capture(temporary, { file, transparent, body, markup }, mark) {
  const name = path.basename(file, '.png');
  const page = path.join(temporary, `${name}.html`);
  const png = path.join(temporary, `${name}.png`);
  await fs.writeFile(page, `<!doctype html>
<meta charset="utf-8">
<style>${body}
</style>
${markup(`<img alt="Circular" src="data:image/svg+xml;base64,${mark.toString('base64')}">`)}
`);
  const child = spawn(chrome, [
    '--headless=new', `--screenshot=${png}`, '--window-size=1024,1024',
    '--force-device-scale-factor=1',
    ...(transparent ? ['--default-background-color=00000000'] : []),
    '--hide-scrollbars', '--no-first-run', '--no-default-browser-check',
    '--disable-background-networking', `--user-data-dir=${path.join(temporary, `${name}-profile`)}`,
    pathToFileURL(page).href,
  ], { stdio: 'ignore' });
  const exited = new Promise(resolve => child.once('exit', resolve));
  let bytes = null;
  for (let i = 0; i < 600 && !bytes; i++) {
    const read = await fs.readFile(png).catch(() => null);
    if (read && read.length > 12 && read.subarray(-8, -4).toString('ascii') === 'IEND') bytes = read;
    else if (child.exitCode !== null) break;
    else await new Promise(resolve => setTimeout(resolve, 100));
  }
  if (child.exitCode === null) { child.kill(); await exited; }
  if (!bytes) throw Object.assign(new Error(`Chrome wrote no complete PNG for ${file}`), { code: 'APP_ICON_CAPTURE_MISSING' });
  if (bytes.length < 26 || bytes.subarray(0, 8).toString('hex') !== '89504e470d0a1a0a' ||
      bytes.readUInt32BE(16) !== 1024 || bytes.readUInt32BE(20) !== 1024 || (transparent && bytes[25] !== 6)) {
    throw Object.assign(new Error(`Expected a 1024 × 1024 PNG${transparent ? ' with alpha' : ''} for ${file}`),
      { code: 'APP_ICON_PNG_INVALID' });
  }
  return bytes;
}

async function generate() {
  const mark = await fs.readFile(path.resolve(appRoot, '../../assets/brand/mark.svg'));
  const paper = (await fs.readFile(path.join(appRoot, 'appearance.css'), 'utf8'))
    .match(/^:root\s*\{[^}]*--paper:\s*(#[0-9a-fA-F]{6});/m)?.[1];
  if (!paper) throw Object.assign(new Error('appearance.css names no light --paper'), { code: 'APP_ICON_PAPER_MISSING' });
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'circular-app-icon-'));
  try {
    const captured = [];
    for (const output of outputs(paper)) captured.push([path.join(appRoot, output.file), await capture(temporary, output, mark)]);
    for (const [file, bytes] of captured) {
      await fs.mkdir(path.dirname(file), { recursive: true });
      await fs.writeFile(file, bytes);
      console.log(JSON.stringify({ code: 'APP_ICON_WRITTEN', path: file, bytes: bytes.length }));
    }
  } finally {
    await fs.rm(temporary, { recursive: true, force: true });
  }
}

generate().catch(error => {
  console.error(JSON.stringify({ code: 'APP_ICON_GENERATION_FAILED', reason: error.code ?? null,
    message: error.message }));
  process.exitCode = 1;
});
