# Versions and platform

## This is an alpha

Circular is not released as stable. Command names and flags, SDK exports, configuration
keys, actor configuration and port names, diagnostic codes, and the formats the daemon
stores can all change from one alpha or beta release to the next.

Today, a corrected name, configuration key or command spelling replaces the old one, and the
old spelling is refused. For example, `journal.retain` and `journal.retain_derived` are
rejected regardless of their values. For `journal.retain`, startup fails with
`ConfigRejected: daemon config <path>: unknown config key journal.retain. Remove journal.retain from config.toml.`
For `journal.retain_derived`, startup fails with
`ConfigRejected: daemon config <path>: unknown config key journal.retain_derived`.

A state directory made with one alpha version may not open in the next.
Pin the version you built against, and keep your pipeline program, which you can deploy
again into a fresh state directory.

## The pieces

An installation is two commands and the desktop app, plus the source tree they were built
from (the SDK packages, the templates and the documents):

| | |
|---|---|
| `circular-daemon` | the engine |
| `circular` | the command line: a launcher that runs the CLI in the installation's `src/sdk/typescript` on the Node runtime inside `Circular.app` |
| `Circular.app` | the desktop app |

`scripts/install.sh` installs one release, the tag its URL names. It puts all of it in one
directory per build, `~/.local/opt/circular/<version>-<commit>/`, and does not change that
directory after placing it. `~/.local/opt/circular/current` selects one of them, and
`~/.local/bin/circular` and `~/Applications/Circular.app` are links through `current`.

To update, run the one-line command of the newer release. It installs that release into its
own directory beside the ones already there, then points `current` and the two links at it.
The older installations stay; the installer lists them and prints the command that removes
one. When the same build is already installed, the installer checks it and selects it
instead of building it again. The installer does not open or change a state directory, and
does not start, stop or register a daemon: a daemon that is already running keeps running
the version it started with.

Ask each piece what it is:

```sh
circular --version                  # CLI version and the commit it was built from
circular version --state <dir>      # the same, plus the installed daemon executable's
circular-daemon --version           # engine build identity
```

No query publishes the version of a daemon that is already running, so `circular version`
reports the installed executable and says so rather than claiming the running build.

## Platform

macOS on Apple Silicon. The daemon's location rules, its logging paths, and the LaunchAgent
registration are macOS-specific, and the desktop UI is built for that platform. The
installed `circular` command runs on the Node runtime inside `Circular.app`, so it needs no
Node of its own. Node.js 22.12 or newer must be on your `PATH` for the installer; it does not
install Node. The SDK package declares Node.js 22 or newer for programs you run yourself.

## Experimental and unsupported parts

Some published parts are experimental or not supported in this alpha. The reference pages
mark them, and the README's *Known issues* section lists what you can hit and what to do
instead.
