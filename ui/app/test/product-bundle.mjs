import assert from 'node:assert/strict';
import fs from 'node:fs';

const read = name => fs.readFileSync(new URL('../' + name, import.meta.url), 'utf8');
export function productBundle() {
  const boot = read('bootstrap.js');
  const list = name => {
    const found = boot.match(new RegExp(`${name} = (?:new Set\\()?(\\[[^\\]]*\\])`));
    assert.ok(found, `bootstrap.js ${name}`);
    return JSON.parse(found[1].replace(/'/g, '"'));
  };
  const files = [...list('canvasScripts'), 'bootstrap.js', 'appearance.js', 'index.html'];
  for (const dir of ['renderer', 'bridge'])
    for (const file of fs.readdirSync(new URL('../' + dir, import.meta.url))) if (/\.m?js$/.test(file)) files.push(`${dir}/${file}`);
  return new Map(files.map(file => [file, read(file)]));
}
