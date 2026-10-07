#!/bin/sh
#
# It runs the installed CLI — `src/sdk/typescript/circular.mjs` of the source tree the
# installation keeps — on the Node runtime inside the `Circular.app` that sits beside it in
# the same `bin/`, so the command needs no separately installed Node and works the same from
# a terminal and from a Finder or Dock launch, whose PATH has no node.
#
# The installer puts a symbolic link to this file on PATH (`~/.local/bin/circular`). The
# links are followed to this file first, so `bin` is the installation's own `bin/` whatever
# path the command was called by.
#
# ELECTRON_RUN_AS_NODE is Electron's own runtime switch that makes the app's executable act
# as plain Node. It selects the runtime of this one process and configures nothing in the
# product. The CLI removes it from its environment before any of its code runs, so nothing
# the CLI starts (the daemon, an agent CLI) inherits it.
#
# process.argv[1] is this launcher's own path inside the installation: the CLI finds
# `circular-daemon` beside its launcher (cli/common.mjs siblingProgram).
self=$0
while [ -L "$self" ]; do
    target=$(readlink -- "$self") || exit 1
    case $target in
        /*) self=$target ;;
        *) self=$(dirname -- "$self")/$target ;;
    esac
done
bin=$(CDPATH= cd -- "$(dirname -- "$self")" && pwd -P) || exit 1
ELECTRON_RUN_AS_NODE=1 exec "$bin/Circular.app/Contents/MacOS/Circular" --input-type=module --eval '
import path from "node:path";
import { pathToFileURL } from "node:url";
delete process.env.ELECTRON_RUN_AS_NODE;
await import(pathToFileURL(path.join(path.dirname(process.argv[1]), "../src/sdk/typescript/circular.mjs")));
' "$bin/circular" "$@"
