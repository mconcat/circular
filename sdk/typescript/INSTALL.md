# The `circular` CLI component

You may be reading this from inside an installation, at
`<prefix>/src/sdk/typescript/INSTALL.md`. **You did not install this package on its
own.** The whole installation — the daemon, the desktop app and this CLI in the source tree it
came with — is placed in one step by `scripts/install.sh`, the one-line installer; `<prefix>/src/QUICKSTART.md`
is the walkthrough from there to a running, observable pipeline. This file says what this one
component is and is not.

This package is the **command line only**: `circular chat`, `circular edit`, `circular daemon`, `circular doctor`, `circular bugreport`, `circular version`, `circular harness`, `circular template` (see `circular --help`).
The daemon binary (`circular-daemon`) is a separate build product installed by the same installer, not
by this package. Nothing here talks to a daemon at install time. The installer runs `npm ci`
here with the Node.js 22.12 or newer on `PATH`; the installed `circular` command then runs this
directory's `circular.mjs` on the Node runtime inside the `Circular.app` in `<prefix>/bin`
(`scripts/circular-launcher.sh`).

Everything below the requirements table is the **contributor** view: how this component's tarball
is produced and checked from a source checkout. A beta user needs none of it.

## Requirements

| What | Value | Why |
|---|---|---|
| Node.js | `>= 22` (`engines.node`) | the launcher uses `import.meta.dirname`; the SDK is plain ES modules, no build step |
| npm | 10 or newer (ships with Node.js 22) | `bundleDependencies` are read from the tarball during a standalone CLI install |
| network | none for a standalone CLI install from a local tarball | every `@circular/*` package and `typescript` are bundled inside the tarball |

## The artifact

One tarball, produced from the SDK workspace:

```sh
cd sdk/typescript
npm run pack:cli            # prepack writes build-info.json (version + git commit); tarball lands in dist/
ls dist/circular-cli-*.tgz
```

The tarball carries `circular.mjs`, the `attach` launcher, the three shipped templates, and
`node_modules/` with the seven `@circular/*` workspace packages plus `typescript` bundled. Test files,
contracts, probes and reviews are not in it — the package's `files` allowlist decides that.

## Installing the CLI component on its own (contributors)

```sh
npm install -g ./circular-cli-<version>.tgz        # or: npm install -g --prefix <dir> ./circular-cli-<version>.tgz
circular --version                                # → circular <version> (<git sha>)
circular --help
```

`--version` prints the package version and the git commit the tarball was packed from
(`<sha>-dirty` when the tree had uncommitted changes). A development checkout without
`build-info.json` asks git for the tree it runs from.

## Verifying an install

```sh
circular --version | grep -E '^circular [0-9]'    # exit 0
circular chat --help
circular edit --help
```

## What this package does not do

- It does not install or start `circular-daemon`; `circular chat --state <dir>` expects a state
  directory a running daemon has claimed.
- It does not publish to the npm registry (`private: true` for now); the tarball is the distribution
  unit until a registry decision is made.
- It does not decide product behavior from environment variables.
