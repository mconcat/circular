# The installer

## One line

```sh
curl -fsSL https://raw.githubusercontent.com/mconcat/circular/v0.1.0-alpha.1/scripts/install.sh | sh
```

That is the whole installation. The command is pinned to one release tag, and the installer
installs that release and nothing else: `install.sh` names its tag once, beside the repository
(`CIRCULAR_TAG` and `CIRCULAR_REPO` at the top), and every URL it fetches comes from those two.
It never asks GitHub for the "latest" release. The installer builds Circular on your Mac and
installs it for your user. Only the daemon comes prebuilt, from the release. Everything else is
that release's source tree, which the installation keeps: the `circular` CLI runs from its
`sdk/typescript` after `npm ci` there with the Node.js you already have, and `Circular.app` is
built from its `ui/app`.

Nothing is signed by us, and nothing needs to be. The downloaded daemon keeps the ad-hoc
signature its linker gave it. `curl` does not mark what it downloads as quarantined, and
what is built on your machine is not quarantined either, so macOS opens both without asking.
Because `Circular.app` is built here with its own name and icon, the Dock shows "Circular".

## What it needs

| What | Why |
|---|---|
| macOS on Apple Silicon | the daemon's location rules and logs are macOS-specific; Intel Macs are refused |
| Node.js 22.12 or newer, with npm | installs the CLI's dependencies and builds the app; the installed `circular` command then runs on the Node runtime inside `Circular.app`, so it works without Node on `PATH` |
| the Xcode command-line tools (`xcode-select --install`) | `git`, to fetch the source |
| network access | GitHub (the release and its source), the npm registry, and Electron's download |
| Rust (<https://rustup.rs>), only sometimes | when the release's prebuilt daemon cannot be downloaded, has no checksum or does not run on your Mac, the installer builds the daemon from the same source; without Rust it stops and says so |

The installer installs none of these for you.

## What it does

The first line says which way it installs. Each step prints one line that starts with `==>`.

1. Checks the Mac, Node.js and the Xcode command-line tools.
2. Checks that the places it writes are free (below). A file there that the installer did not
   put there stops it before anything is downloaded.
3. Downloads its tag's daemon asset,
   `releases/download/<tag>/circular-daemon-aarch64-apple-darwin.tar.gz`, and the asset's
   `.sha256`. It checks the SHA-256 and runs `circular-daemon --version` once. A SHA-256 that
   differs stops it. When the asset cannot be downloaded, its `.sha256` cannot be downloaded,
   or the daemon does not run, it prints that reason as an `install: <REASON>: …` line (the
   table below), shows what a daemon that did not run printed, and builds the daemon from
   source instead.
4. Clones its tag's source (`git clone --depth 1 --branch <tag>`) into a temporary directory.
   The daemon asset must have been built from exactly the tag's commit, from a clean tree;
   otherwise it stops with `ASSET_SOURCE_MISMATCH`.
5. If this commit is already installed, it selects that installation and stops here.
   Otherwise it moves the source tree into a new installation, as `src/`, and builds there.
   When there is no usable prebuilt daemon, it builds one with `scripts/build-artifact.py`
   and then removes the build's `target/`. Cargo's output goes to a log instead of the
   screen; a failed build prints its errors and keeps the whole log at the path the failure
   line names.
6. Runs `npm ci` in `src/sdk/typescript` and writes its `build-info.json` (the commit
   `circular --version` prints). The CLI runs from there, so it reads the public documents —
   `QUICKSTART.md`, `reference/actors/` and the rest — in the same source tree.
7. Builds the app: `npm ci` in `src/ui/app`, downloads Electron, and runs
   `npm run dist -- --dir` (electron-builder, no signing identity). `Circular.app` goes to
   `bin/`; the app's build tools (`ui/app/node_modules`) and the clone's `.git` are removed.
   `bin/circular` is the launcher (`scripts/circular-launcher.sh`).
8. Checks the new installation: every file is there, and `circular-daemon --version` and
   `circular --version` run, the latter with the `PATH` of a Finder or Dock launch.
9. Moves the new installation into place in one rename, selects it, and links the command
   and the app.

It never opens, creates or changes a state directory. It never starts, stops or registers a
daemon. The temporary directory is removed however it exits.

## Where things go

| Path | What it is |
|---|---|
| `~/.local/opt/circular/<version>-<commit>/` | one installation per build identity: `bin/` (`circular`, `circular-daemon`, `Circular.app`), `share/circular/` (`self.json`, `docs/`), `src/` (the release's source tree, with `sdk/typescript/node_modules`) |
| `~/.local/opt/circular/current` | a link to the selected installation |
| `~/.local/bin/circular` | a link to `current/bin/circular`, the command |
| `~/Applications/Circular.app` | a link to `current/bin/Circular.app`, the app |

Why these places:

- **Your home, not the system.** Nothing needs `sudo`, and the installation belongs to the
  user who runs it.
- **One directory per build identity, and a current selection.** An installation is never
  changed after it is placed. A daemon or an app that is already running keeps running the
  bytes it started from, and a new installation sits beside it until you switch those over.
- **`~/.local/opt/circular`** is the conventional install location. The daemon's own location
  rules name the same place (`crates/cli/src/shell/location.rs`), and take the installation
  `current` selects when they are not running from one. It is outside `~/Library/Application Support/Circular`,
  so nothing that is replaced sits next to the state and the app data you create.
- **`~/.local/bin`** is the usual per-user place for commands, and other command-line tools
  already put theirs there. If it is not on your `PATH`, the installer says so and prints the
  line to add.
- **`~/Applications`** is where macOS looks for one user's apps.
- **The source tree stays.** The CLI and the agent sessions it opens read the public
  documents by their place relative to `sdk/typescript`, and the templates are read from
  `sdk/typescript/templates/`. A packed npm tarball would carry neither the documents nor
  that relation.
- **`Circular.app` stays beside `circular` and `circular-daemon`.** The app runs the
  `circular` beside it to start a daemon, and `circular` runs on the app's Node runtime. The
  link in `~/Applications` points at that place; it is not a second copy.

## Updating and removing

Run a newer release's line. It installs into a new `<version>-<commit>` directory and becomes
`current`; the previous one is kept and listed at the end. Running the same release's line
again selects the installation already there. A daemon that is already running keeps running
the version it started with until you restart it. A daemon registered as a LaunchAgent keeps
the executable it was registered with — the `<version>-<commit>` directory, not `current` —
until you run `circular daemon install --state "$STATE"` again.

To remove an old installation, delete its directory. To remove Circular entirely:

```sh
rm -rf ~/.local/opt/circular ~/.local/bin/circular ~/Applications/Circular.app
```

That leaves your state directories and `~/Library/Application Support/Circular` alone.

## Reasons and exit codes

Every line the installer prints about something going wrong has one form,
`install: <REASON>: <what happened and what to do>`. `<REASON>` is a fixed word for a program to
match. A failure prints one such line and exits with the code below. Three reasons select a
source-build fallback. The installer then checks for Rust and Python before starting that build.

| Exit code | Reasons | Meaning |
|---|---|---|
| 0 | | installed, or `--help` |
| 2 | `USAGE` | an unknown argument, or an option without its path |
| 3 | `UNSUPPORTED_PLATFORM`, `NODE_REQUIRED`, `DEVELOPER_TOOLS_REQUIRED`, `RUST_REQUIRED` | this Mac lacks something the installer needs |
| 4 | `SOURCE_UNAVAILABLE`, `ASSET_UNAVAILABLE`, `ASSET_CHECKSUM_MISSING`, `ASSET_CHECKSUM_MISMATCH`, `ASSET_INVALID`, `ASSET_NOT_RUNNABLE`, `ASSET_SOURCE_MISMATCH` | the tag's source or a daemon asset could not be fetched or verified |
| 5 | `LOCATION_OCCUPIED`, `WORK_DIRECTORY_UNAVAILABLE` | a place the installer writes holds something it did not put there or cannot be created, or no work directory can be made in `$TMPDIR` |
| 6 | `BUILD_FAILED` | building the daemon, the CLI or the app failed; the build's own output is above the line |
| 7 | `IMAGE_CHECK_FAILED` | the new installation is incomplete or does not run; nothing was installed |
| 130 | | interrupted by a signal (Ctrl-C, hang-up or terminate); no reason line is printed, and the work directory and a staging directory not yet placed are removed |
| — (goes on) | `ASSET_UNAVAILABLE` | the release's daemon asset could not be downloaded (the line quotes `curl`, for example an HTTP 404) |
| — (goes on) | `ASSET_CHECKSUM_MISSING` | the release's asset has no `.sha256` that could be downloaded |
| — (goes on) | `ASSET_NOT_RUNNABLE` | the release's daemon does not run on this Mac; what it printed follows, each line starting `circular-daemon --version:` |
| — (goes on) | `APP_NOT_REGISTERED` | macOS's app registry (LaunchServices) refused the new `Circular.app`; everything is installed, and the Dock may show an earlier build's icon until you log out |

The three `ASSET_*` "goes on" reasons stop the installer instead when the asset came from `--daemon`,
because then there is no release to fall back from.

## Installing from a checkout (contributors)

```sh
scripts/install.sh --source .                          # builds the daemon with Rust too
python3 scripts/build-artifact.py                      # or build the daemon asset first,
scripts/install.sh --source . --daemon dist/circular-daemon-aarch64-apple-darwin.tar.gz
```

`--source` installs the committed `HEAD` of that checkout; uncommitted changes are not
included, and the installer says so. The version is then that commit, not a tag. `--daemon`
takes an asset with its `.sha256` beside it, and the same strict check applies: it must be built
from that same commit. Everything else is the same as the one line.

## Publishing a release

`scripts/build-artifact.py` builds the daemon asset, and nothing else:

```sh
python3 scripts/build-artifact.py
# dist/circular-daemon-aarch64-apple-darwin.tar.gz
# dist/circular-daemon-aarch64-apple-darwin.tar.gz.sha256
```

A release is made in this order:

1. Set `CIRCULAR_TAG` at the top of `scripts/install.sh` to the new tag, and commit that in the
   published repository.
2. Tag that commit with the same name.
3. In a clean checkout of that commit, run `python3 scripts/build-artifact.py`.
4. Attach both files to the GitHub release of that tag.

The installer compares the asset's commit with the source it clones for its tag and refuses a
mismatch, so the asset must come from the published repository's commit, not from another
repository's. The one-line command names the same tag in its URL. The script needs Rust, the
Xcode command-line tools and Python 3.9 or newer; the macOS `python3` is enough.

The script refuses, before it builds anything, a checkout with uncommitted or untracked files,
because the installer refuses an asset built from one. It never overwrites an existing file in
`dist/`. A refusal or failure is one line, `build-artifact: <REASON>: <what happened>`, with
the installer's word and exit code for the same fact: `ASSET_SOURCE_MISMATCH` (4) for such a
checkout, `LOCATION_OCCUPIED` (5) for an asset already in `dist/`, `BUILD_FAILED` (6) for
anything else.

The asset holds one directory, `circular-daemon-aarch64-apple-darwin/`, with
`bin/circular-daemon`, `self.json` (the build's identity and compatibility axes) and `docs/`
(the offline document bundle). The installer copies the first to `bin/` and the other two to
`share/circular/`.

## Tests

```sh
python3 -m unittest scripts/test_install.py scripts/test_build_artifact.py
```

The installer's tests run the real `install.sh` with a new `HOME` in a temporary directory. A
copy of the script whose two constants point at a local stand-in repository shows that it
fetches only its own tag's asset and source. The end-to-end case installs for real from this
checkout. It then registers the installed daemon as a LaunchAgent, and starts and stops the
installed daemon, each with its own `HOME`: a short directory under `/tmp`, because a socket
path must fit in 103 bytes. Registration does not call `launchctl`. It also asks the installed
`circular daemon status` which daemon it would start. The CLI reads the account's home
directory rather than `HOME`, so that one question uses a private state directory below your
real home, which it removes; it writes nothing else there. The end-to-end case runs only when
`dist/` holds the daemon asset of this checkout's `HEAD`.
