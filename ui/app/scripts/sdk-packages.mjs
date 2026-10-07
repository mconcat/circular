import fs from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';

export const appRoot = path.resolve(import.meta.dirname, '..');
export const sdkRoot = path.resolve(appRoot, '../../sdk/typescript/packages');
const { dependencies } = JSON.parse(await fs.readFile(path.join(appRoot, 'package.json'), 'utf8'));
const packages = Object.keys(dependencies).filter(name => name.startsWith('@circular/'))
  .map(name => name.slice('@circular/'.length)).sort();

async function files(root, relative = '') {
  const entries = await fs.readdir(path.join(root, relative), { withFileTypes: true });
  return (await Promise.all(entries.map(async entry => {
    const name = path.join(relative, entry.name);
    assert(!entry.isSymbolicLink(), `SDK package must be a copy, not a symlink: ${name}`);
    return entry.isDirectory() ? files(root, name) : [name];
  }))).flat().sort();
}

export async function verifyPackages(destination = path.join(appRoot, 'node_modules/@circular')) {
  let count = 0;
  for (const name of packages) {
    const source = path.join(sdkRoot, name), target = path.join(destination, name);
    assert(!(await fs.lstat(target)).isSymbolicLink(), `${name} must be copied`);
    const manifest = JSON.parse(await fs.readFile(path.join(source, 'package.json'), 'utf8'));
    const expected = ['README.md', 'package.json', ...(await Promise.all(manifest.files.map(async dir =>
      (await files(path.join(source, dir))).map(f => `${dir}/${f}`)))).flat()].sort();
    assert.deepEqual(await files(target), expected, `${name}: package file list`);
    for (const file of expected) {
      assert.deepEqual(await fs.readFile(path.join(target, file)), await fs.readFile(path.join(source, file)), `${name}/${file}`);
      count++;
    }
  }
  return count;
}

export const appPackages = app => path.join(app, 'Contents/Resources/app/node_modules/@circular');
export async function verifyApp(app) {
  const scopes = (await fs.readdir(appPackages(app))).sort();
  assert.deepEqual(scopes, packages, 'the bundle carries exactly the public SDK packages');
  const count = await verifyPackages(appPackages(app));
  const packaged = path.relative(app, appPackages(app));
  const strays = (await fs.readdir(app, { recursive: true })).filter(file => !file.startsWith(`${packaged}/`) &&
    (/(^|\/)client\/src\/internal\/|(^|\/)cli\/common\.mjs$/.test(file) ||
     /(^|\/)circular(-daemon)?$/.test(file)));
  assert.deepEqual(strays, [], 'no SDK internal copy, CLI constant file or daemon/CLI executable outside the packages');
  return count;
}
