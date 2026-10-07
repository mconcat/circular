#!/usr/bin/env node
/**
 * Writes `build-info.json` next to `circular.mjs` — what `circular --version` prints.
 *
 * Runs as the package's `prepack` hook, so the tarball carries the version of the manifest it was
 * packed from and the git commit of the tree (production convention: version + short SHA). The
 * file is gitignored: a source checkout has none, and `--version` then asks git
 * directly. Nothing here is a product setting — it is a description of the artifact.
 */
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const workspace = path.resolve(import.meta.dirname, "..");
const manifest = JSON.parse(fs.readFileSync(path.join(workspace, "package.json"), "utf8"));

export function gitDescription(cwd) {
  const run = (args) => spawnSync("git", args, { cwd, encoding: "utf8", timeout: 5000 });
  const sha = run(["rev-parse", "--short=8", "HEAD"]);
  if (sha.status !== 0) return null;
  const dirty = run(["status", "--porcelain", "--untracked-files=no"]);
  return { sha: sha.stdout.trim(), dirty: dirty.status === 0 && dirty.stdout.trim().length > 0 };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const description = gitDescription(workspace);
  if (description === null) {
    process.stderr.write("write-build-info: not a git checkout; refusing to guess a commit\n");
    process.exit(1);
  }
  const info = {
    name: manifest.name,
    version: manifest.version,
    commit: description.sha,
    dirty: description.dirty,
    builtAt: new Date().toISOString(),
  };
  fs.writeFileSync(path.join(workspace, "build-info.json"), JSON.stringify(info, null, 2) + "\n");
  process.stderr.write(`build-info.json: ${info.name} ${info.version} (${info.commit}${info.dirty ? "-dirty" : ""})\n`);
}
