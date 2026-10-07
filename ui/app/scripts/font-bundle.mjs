import fs from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';

const appRoot = path.resolve(import.meta.dirname, '..');
const fonts = (await fs.readdir(path.join(appRoot, 'fonts'))).sort();

export async function verifyFontBundle(directory) {
  if (directory) assert.deepEqual((await fs.readdir(directory)).sort(), [...fonts].sort());
  let bytes = 0;
  for (const name of fonts) {
    const original = await fs.readFile(path.join(appRoot, 'fonts', name));
    assert(original.length > 0, `${name}: empty font asset`);
    if (directory) assert.deepEqual(await fs.readFile(path.join(directory, name)), original, name);
    bytes += original.length;
  }
  return { files: fonts.length, bytes };
}
