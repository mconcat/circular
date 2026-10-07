#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';

const workspace = path.resolve(import.meta.dirname, '..');
const source = path.resolve(workspace, '..', '..');
const names = ['AGENTS.md', 'CLAUDE.md'];

let copied = 0;
for (const name of names) {
  const from = path.join(source, name);
  if (!fs.existsSync(from)) {
    process.stderr.write(`copy-public-instructions: ${from} is absent; refusing to pack without it\n`);
    process.exit(1);
  }
  fs.copyFileSync(from, path.join(workspace, name));
  copied += 1;
}
process.stderr.write(`copy-public-instructions: ${copied} file(s) copied from ${source}\n`);
