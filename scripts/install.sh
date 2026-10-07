#!/bin/sh
# Circular installer: builds Circular on this Mac and installs it for this user.
#
#     curl -fsSL https://raw.githubusercontent.com/mconcat/circular/<tag>/scripts/install.sh | sh
#
# It installs one release: the tag named below, and nothing else. Only the daemon comes
# prebuilt, from that release's asset. The rest is the same tag's source tree, kept in the
# installation: the CLI runs from its sdk/typescript (where it finds the public documents
# beside it), after npm ci there with the Node.js already on this machine, and Circular.app
# is built from its ui/app. The asset must be built from exactly the tag's commit.
# Nothing is signed by us and nothing needs to be: the downloaded daemon keeps the ad-hoc
# signature its linker gave it, and what is built on this machine carries no quarantine.
# When the release's daemon cannot be downloaded, has no checksum or does not run here, the
# installer says which with a reason code and builds the daemon from the same source with Rust.
#
# Where it goes (scripts/INSTALL.md has the reasons):
#   ~/.local/opt/circular/<version>-<commit>/   one installation per build identity:
#                                               bin/, share/circular/, src/ (the source tree)
#   ~/.local/opt/circular/current               the selected one
#   ~/.local/bin/circular                       the command
#   ~/Applications/Circular.app                 the app
# It never opens, creates or changes a state directory, and never starts, stops or
# registers a daemon: one already running keeps running the version it started with.
#
# Contributors install from a checkout instead of a release:
#   scripts/install.sh --source <checkout> [--daemon <asset>.tar.gz]
set -eu

# The one place this installer names where Circular is published, and the one release it
# installs. A release sets the tag here in the commit the tag then points to.
CIRCULAR_REPO='https://github.com/mconcat/circular'
CIRCULAR_TAG='v0.1.0-alpha.1'

TARGET='aarch64-apple-darwin'
ASSET="circular-daemon-$TARGET"
# A Finder or Dock launch has this PATH; the installed command must work with it.
GUI_PATH='/usr/bin:/bin:/usr/sbin:/sbin'

usage() {
    cat <<'USAGE'
Usage: curl -fsSL <URL of install.sh at a release tag> | sh
       scripts/install.sh [--source <checkout>] [--daemon <asset>.tar.gz]

Builds and installs Circular for this user on an Apple Silicon Mac. Without --source it
installs the release this copy of the script names.

  --source <checkout>   build from the committed HEAD of this git checkout instead of
                        the release
  --daemon <file>       install this daemon asset (with <file>.sha256 beside it) instead
                        of downloading one; it must be built from the same commit

Requires Node.js 22 or newer and the Xcode command-line tools. When no prebuilt daemon
can be used, also Rust (https://rustup.rs).
USAGE
}

step() { printf '==> %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
# fail <exit code> <reason> <sentence>: the reason is a stable word an agent can match.
fail() {
    printf 'install: %s: %s\n' "$2" "$3" >&2
    exit "$1"
}

# A path the installer may replace is absent or a link into the installation root.
replaceable() {
    [ ! -e "$1" ] && [ ! -L "$1" ] && return 0
    [ -L "$1" ] || return 1
    case $(readlink -- "$1") in "$ROOT"/*) return 0 ;; esac
    return 1
}

# Replace a symbolic link in one rename, so it never points nowhere.
place_link() {
    node -e '
const fs = require("node:fs");
const [target, link] = process.argv.slice(1);
const temporary = `${link}.circular-install-${process.pid}`;
fs.symlinkSync(target, temporary);
fs.renameSync(temporary, link);
' "$1" "$2"
}

# The prebuilt daemon cannot be used: say why with its reason code, and build from source.
degrade() {
    fallback=$2
    printf 'install: %s: %s; the daemon will be built from source\n' "$1" "$2" >&2
}

# Verify <archive> against <archive>.sha256 (both present) and unpack it into $work/daemon.
unpack_asset() {
    expected=$(awk 'NR == 1 { print $1 }' "$1.sha256")
    actual=$(/usr/bin/shasum -a 256 "$1" | awk '{ print $1 }')
    [ -n "$expected" ] && [ "$expected" = "$actual" ] \
        || fail 4 ASSET_CHECKSUM_MISMATCH "the daemon asset's SHA-256 is $actual, its checksum file says ${expected:-nothing}"
    rm -rf "$work/daemon"
    mkdir "$work/daemon" || fail 5 WORK_DIRECTORY_UNAVAILABLE "cannot create $work/daemon"
    /usr/bin/tar -xzf "$1" -C "$work/daemon" || fail 4 ASSET_INVALID "cannot unpack $1"
    asset_dir="$work/daemon/$ASSET"
    [ -x "$asset_dir/bin/circular-daemon" ] && [ -f "$asset_dir/self.json" ] \
        && [ -f "$asset_dir/docs/index.json" ] \
        || fail 4 ASSET_INVALID "$1 does not hold $ASSET/bin/circular-daemon, self.json and docs/"
    note "SHA-256 $actual"
}

# Run the unpacked daemon once. Its output is kept in $daemon_says either way.
asset_runs() {
    daemon_says=$("$asset_dir/bin/circular-daemon" --version 2>&1 </dev/null)
}

# Show what a daemon that did not run said, instead of discarding it.
show_daemon_says() {
    printf '%s\n' "$daemon_says" | head -n 20 | sed 's/^/    circular-daemon --version: /' >&2
}

# The daemon asset in $asset_dir must be built from $commit, from a clean tree. Sets
# $built (the commit, 12 digits) and $version from its self.json.
check_identity() {
    built=$(node -e '
const { identity } = JSON.parse(require("node:fs").readFileSync(process.argv[1], "utf8"));
if (!/^[0-9a-f]{12}$/.test(identity.git_sha) || typeof identity.git_dirty !== "boolean"
    || !/^[0-9A-Za-z.+-]+$/.test(identity.version)) process.exit(1);
console.log(`${identity.git_sha}${identity.git_dirty ? "-dirty" : ""} ${identity.version}`);
' "$asset_dir/self.json") || fail 4 ASSET_INVALID 'the daemon asset has no build identity in self.json'
    version=${built#* }
    built=${built%% *}
    case $built in
        *-dirty) fail 4 ASSET_SOURCE_MISMATCH 'the daemon asset was built from a tree with uncommitted changes' ;;
    esac
    case $commit in
        "$built"*) ;;
        *) fail 4 ASSET_SOURCE_MISMATCH "the daemon asset was built from commit $built and the source is commit $short; they must be the same commit" ;;
    esac
}

# Check an installed image and run its two commands the way Finder and the Dock would.
check_image() {
    for program in bin/circular-daemon bin/circular bin/Circular.app/Contents/MacOS/Circular; do
        [ -f "$1/$program" ] && [ -x "$1/$program" ] || { note "missing $program"; return 1; }
    done
    for file in share/circular/self.json share/circular/docs/index.json src/QUICKSTART.md \
        src/sdk/typescript/circular.mjs src/sdk/typescript/build-info.json; do
        [ -f "$1/$file" ] || { note "missing $file"; return 1; }
    done
    node -e '
const fs = require("node:fs");
const path = require("node:path");
const sdk = path.join(process.argv[1], "src/sdk/typescript");
const manifest = JSON.parse(fs.readFileSync(path.join(sdk, "package.json"), "utf8"));
for (const dependency of Object.keys(manifest.dependencies ?? {})) {
  if (!fs.existsSync(path.join(sdk, "node_modules", dependency, "package.json"))) {
    console.log(`    missing dependency ${dependency} in src/sdk/typescript/node_modules`);
    process.exit(1);
  }
}
' "$1" || return 1
    daemon_version=$("$1/bin/circular-daemon" --version </dev/null 2>&1) \
        || { note "circular-daemon --version failed: $daemon_version"; return 1; }
    cli_version=$(PATH=$GUI_PATH "$1/bin/circular" --version </dev/null 2>&1) \
        || { note "circular --version failed: $cli_version"; return 1; }
    note "$(printf '%s\n' "$daemon_version" | head -n 1)"
    note "$(printf '%s\n' "$cli_version" | head -n 1)"
}

main() {
    source_dir=
    daemon_file=
    while [ "$#" -gt 0 ]; do
        case $1 in
            -h|--help) usage; exit 0 ;;
            --source|--daemon)
                [ "$#" -ge 2 ] && [ -n "$2" ] || fail 2 USAGE "$1 needs a path"
                case $1 in --source) source_dir=$2 ;; *) daemon_file=$2 ;; esac
                shift 2 ;;
            *) usage >&2; fail 2 USAGE "unknown argument: $1" ;;
        esac
    done
    if [ -n "$source_dir" ]; then
        [ -d "$source_dir" ] || fail 4 SOURCE_UNAVAILABLE "no such directory: $source_dir"
        source_dir=$(CDPATH= cd -- "$source_dir" && pwd -P) \
            || fail 4 SOURCE_UNAVAILABLE "cannot enter $source_dir"
    fi
    if [ -n "$daemon_file" ]; then
        case $daemon_file in /*) ;; *) daemon_file=$(pwd -P)/$daemon_file ;; esac
    fi

    ROOT=$HOME/.local/opt/circular
    command_link=$HOME/.local/bin/circular
    app_link=$HOME/Applications/Circular.app

    if [ -z "$source_dir" ]; then
        printf 'Circular installer: installing Circular %s. The daemon comes prebuilt with that release;\n' "$CIRCULAR_TAG"
        printf '%s\n' 'the CLI and Circular.app are built here with your Node.js, in a few minutes. Rust is' \
            'needed only when the release has no prebuilt daemon this Mac can use.'
    elif [ -z "$daemon_file" ]; then
        printf 'Circular installer: installing from %s; the daemon is built with Rust, the CLI and Circular.app with your Node.js.\n' "$source_dir"
    else
        printf 'Circular installer: installing from %s with the daemon asset %s.\n' "$source_dir" "$daemon_file"
    fi

    step 'Checking this Mac'
    [ "$(/usr/bin/uname -s)" = Darwin ] \
        || fail 3 UNSUPPORTED_PLATFORM "Circular runs on macOS; this system is $(/usr/bin/uname -s)"
    [ "$(/usr/sbin/sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ] \
        || fail 3 UNSUPPORTED_PLATFORM 'Circular runs on Apple Silicon Macs; this Mac has an Intel processor'
    note "macOS $(/usr/bin/sw_vers -productVersion), Apple Silicon"

    step 'Checking Node.js'
    command -v node >/dev/null 2>&1 \
        || fail 3 NODE_REQUIRED 'Node.js 22.12 or newer is required; install it (https://nodejs.org or your version manager) and rerun'
    # The pinned Electron declares engines.node >= 22.12.0; the app build needs it.
    node -e 'const [a, b] = process.versions.node.split(".").map(Number); process.exit(a > 22 || (a === 22 && b >= 12) ? 0 : 1)' 2>/dev/null \
        || fail 3 NODE_REQUIRED "Node.js 22.12 or newer is required; this one is $(node --version 2>/dev/null || echo 'not runnable')"
    command -v npm >/dev/null 2>&1 || fail 3 NODE_REQUIRED 'npm is required; it comes with Node.js'
    note "node $(node --version), npm $(npm --version)"

    step 'Checking the Xcode command-line tools'
    # Checked first: without them /usr/bin/git opens an installation dialog instead of running.
    /usr/bin/xcode-select -p >/dev/null 2>&1 \
        || fail 3 DEVELOPER_TOOLS_REQUIRED 'the Xcode command-line tools are required (git; the compiler for a source build): run xcode-select --install and rerun'
    git --version >/dev/null 2>&1 || fail 3 DEVELOPER_TOOLS_REQUIRED 'git is required and does not run'
    note "$(git --version)"

    step "Checking the install locations under $HOME"
    if [ -e "$ROOT" ] || [ -L "$ROOT" ]; then
        [ -d "$ROOT" ] && [ ! -L "$ROOT" ] || fail 5 LOCATION_OCCUPIED "$ROOT exists and is not a directory; move it away and rerun"
    fi
    if [ -e "$ROOT/current" ] || [ -L "$ROOT/current" ]; then
        [ -L "$ROOT/current" ] || fail 5 LOCATION_OCCUPIED "$ROOT/current is not the installer's link; move it away and rerun"
    fi
    for link in "$command_link" "$app_link"; do
        replaceable "$link" || fail 5 LOCATION_OCCUPIED "$link exists and is not a link into $ROOT; move it away and rerun"
    done

    work=$(mktemp -d "${TMPDIR:-/tmp}/circular-install.XXXXXX") \
        || fail 5 WORK_DIRECTORY_UNAVAILABLE "cannot create a work directory in ${TMPDIR:-/tmp}"
    stage=
    trap 'rm -rf "$work" ${stage:+"$stage"}' EXIT
    trap 'exit 130' HUP INT TERM

    asset_dir=
    fallback=
    if [ -n "$daemon_file" ]; then
        step "Checking the daemon asset $daemon_file"
        [ -f "$daemon_file" ] || fail 4 ASSET_UNAVAILABLE "no daemon asset at $daemon_file"
        [ -f "$daemon_file.sha256" ] \
            || fail 4 ASSET_CHECKSUM_MISSING "no checksum beside the daemon asset: $daemon_file.sha256"
        unpack_asset "$daemon_file"
        if ! asset_runs; then
            show_daemon_says
            fail 4 ASSET_NOT_RUNNABLE "$daemon_file holds a circular-daemon that does not run on this Mac; its output is above"
        fi
    elif [ -z "$source_dir" ]; then
        step "Downloading the prebuilt daemon of $CIRCULAR_TAG: $ASSET.tar.gz"
        base="$CIRCULAR_REPO/releases/download/$CIRCULAR_TAG/$ASSET.tar.gz"
        if ! said=$(/usr/bin/curl -fsSL -o "$work/$ASSET.tar.gz" "$base" 2>&1 </dev/null); then
            degrade ASSET_UNAVAILABLE "cannot download $base: $said"
        elif ! said=$(/usr/bin/curl -fsSL -o "$work/$ASSET.tar.gz.sha256" "$base.sha256" 2>&1 </dev/null); then
            degrade ASSET_CHECKSUM_MISSING "cannot download $base.sha256: $said"
        else
            unpack_asset "$work/$ASSET.tar.gz"
            if ! asset_runs; then
                asset_dir=
                degrade ASSET_NOT_RUNNABLE 'the prebuilt circular-daemon does not run on this Mac; its output follows'
                show_daemon_says
            fi
        fi
    else
        fallback='installing from a checkout without --daemon'
    fi

    if [ -n "$source_dir" ]; then
        step "Taking the source from the committed HEAD of $source_dir"
        git -C "$source_dir" rev-parse --verify -q HEAD >/dev/null 2>&1 \
            || fail 4 SOURCE_UNAVAILABLE "$source_dir is not a git checkout with a commit"
        [ -z "$(git -C "$source_dir" status --porcelain --untracked-files=no 2>/dev/null)" ] \
            || note "uncommitted changes in $source_dir are not included"
        git -c advice.detachedHead=false clone --quiet --depth 1 "file://$source_dir" "$work/source" </dev/null \
            || fail 4 SOURCE_UNAVAILABLE "cannot clone $source_dir"
    else
        step "Downloading the source of $CIRCULAR_TAG"
        git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$CIRCULAR_TAG" "$CIRCULAR_REPO" "$work/source" </dev/null \
            || fail 4 SOURCE_UNAVAILABLE "cannot clone the tag $CIRCULAR_TAG from $CIRCULAR_REPO"
    fi
    commit=$(git -C "$work/source" rev-parse HEAD) \
        || fail 4 SOURCE_UNAVAILABLE 'the fetched source has no commit'
    short=$(printf '%.12s' "$commit")
    note "commit $short"

    [ -z "$asset_dir" ] || check_identity

    id=
    for installed in "$ROOT"/*-"$short"; do
        [ -d "$installed" ] && [ ! -L "$installed" ] || continue
        step "Circular $(basename -- "$installed") is already installed; checking it"
        check_image "$installed" \
            || fail 5 LOCATION_OCCUPIED "$installed is incomplete; remove it and rerun"
        id=$(basename -- "$installed")
    done

    if [ -z "$id" ]; then
        if [ -z "$asset_dir" ]; then
            command -v cargo >/dev/null 2>&1 \
                || fail 3 RUST_REQUIRED "$fallback, and Rust is not installed: install it from https://rustup.rs and rerun"
            command -v python3 >/dev/null 2>&1 \
                || fail 3 DEVELOPER_TOOLS_REQUIRED 'python3 (from the Xcode command-line tools) is required to build the daemon'
        fi
        mkdir -p "$ROOT" || fail 5 LOCATION_OCCUPIED "cannot create $ROOT"
        stage=$(mktemp -d "$ROOT/.stage-XXXXXX") || fail 5 LOCATION_OCCUPIED "cannot create a staging directory in $ROOT"
        # The source tree becomes part of the installation; everything below builds in it.
        src=$stage/src
        mv "$work/source" "$src" && mkdir "$stage/bin" \
            || fail 6 BUILD_FAILED "cannot move the source into $stage"

        if [ -z "$asset_dir" ]; then
            step 'Building the daemon from source with Rust (a few minutes)'
            if ! (cd "$src" && python3 scripts/build-artifact.py </dev/null); then
                # The staging directory goes away on exit; the build's whole output stays.
                kept=$(mktemp "${TMPDIR:-/tmp}/circular-daemon-build.XXXXXX") \
                    && cp "$src/target/artifact-build.log" "$kept" \
                    && fail 6 BUILD_FAILED "the daemon build failed; its errors are above and its whole output is in $kept"
                fail 6 BUILD_FAILED 'the daemon build failed; its output is above'
            fi
            mv "$src/dist/$ASSET.tar.gz" "$src/dist/$ASSET.tar.gz.sha256" "$work/" \
                && rm -rf "$src/target" "$src/dist" "$src/BUILD_INFO" \
                || fail 6 BUILD_FAILED 'cannot clear the daemon build out of the source tree'
            unpack_asset "$work/$ASSET.tar.gz"
            check_identity
        fi
        id=$version-$built

        step 'Installing the SDK and CLI dependencies (npm ci in sdk/typescript)'
        (cd "$src/sdk/typescript" && npm ci --no-audit --no-fund --no-update-notifier </dev/null) \
            || fail 6 BUILD_FAILED 'npm ci failed in sdk/typescript; its output is above'
        # What `circular --version` prints; the source tree keeps no .git to ask.
        (cd "$src/sdk/typescript" && node scripts/write-build-info.mjs </dev/null) \
            || fail 6 BUILD_FAILED 'cannot write sdk/typescript/build-info.json'

        step 'Installing the desktop app dependencies (npm ci in ui/app)'
        (cd "$src/ui/app" && npm ci --no-audit --no-fund --no-update-notifier </dev/null) \
            || fail 6 BUILD_FAILED 'npm ci failed in ui/app; its output is above'
        # Electron no longer downloads itself during npm ci; its package names this command.
        step 'Downloading Electron'
        (cd "$src/ui/app" && ./node_modules/.bin/install-electron </dev/null) \
            || fail 6 BUILD_FAILED 'the Electron download failed; its output is above'
        step 'Building Circular.app (electron-builder)'
        (cd "$src/ui/app" && npm run --no-update-notifier dist -- --dir --out "$work/app" </dev/null) \
            || fail 6 BUILD_FAILED 'the app build failed; its output is above'
        set -- "$work/app"/*/Circular.app
        [ "$#" -eq 1 ] && [ -d "$1" ] || fail 6 BUILD_FAILED 'the app build did not produce one Circular.app'
        # The app carries what it runs; its build tools and the clone's history are not kept.
        mv "$1" "$stage/bin/Circular.app" \
            && rm -rf "$src/ui/app/node_modules" "$src/ui/app/.npm-cache" "$src/ui/app/.electron-cache" "$src/.git" \
            && cp "$src/scripts/circular-launcher.sh" "$stage/bin/circular" \
            && chmod 755 "$stage/bin/circular" \
            && cp "$asset_dir/bin/circular-daemon" "$stage/bin/circular-daemon" \
            && mkdir -p "$stage/share/circular" \
            && cp "$asset_dir/self.json" "$stage/share/circular/self.json" \
            && cp -R "$asset_dir/docs" "$stage/share/circular/docs" \
            || fail 6 BUILD_FAILED 'cannot assemble the new installation'

        step 'Checking the new installation'
        check_image "$stage" || fail 7 IMAGE_CHECK_FAILED 'the new installation is incomplete or does not run; nothing was installed'
        [ ! -e "$ROOT/$id" ] || fail 5 LOCATION_OCCUPIED "$ROOT/$id appeared while building; rerun"
        chmod 755 "$stage" && mv "$stage" "$ROOT/$id" \
            || fail 5 LOCATION_OCCUPIED "cannot place the new installation at $ROOT/$id"
        stage=
    fi

    step "Selecting $id"
    mkdir -p "$(dirname -- "$command_link")" "$(dirname -- "$app_link")" \
        || fail 5 LOCATION_OCCUPIED "cannot create $(dirname -- "$command_link") or $(dirname -- "$app_link")"
    place_link "$id" "$ROOT/current" \
        && place_link "$ROOT/current/bin/circular" "$command_link" \
        && place_link "$ROOT/current/bin/Circular.app" "$app_link" \
        || fail 5 LOCATION_OCCUPIED "cannot link $ROOT/current, $command_link or $app_link"
    # LaunchServices keeps one icon per bundle id; a machine that ran an earlier build shows that
    # build's icon until the new bundle is registered.
    /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
        -f "$ROOT/$id/bin/Circular.app" \
        || printf 'install: APP_NOT_REGISTERED: macOS did not register %s; the Dock may show an earlier icon until you log out\n' "$ROOT/$id/bin/Circular.app" >&2

    printf '\nInstalled Circular %s in %s\n' "$id" "$ROOT/$id"
    printf '  command  %s\n' "$command_link"
    printf '  app      %s\n' "$app_link"
    printf '  guide    %s\n' "$ROOT/current/src/QUICKSTART.md"
    others=
    for installed in "$ROOT"/*; do
        [ -d "$installed" ] && [ ! -L "$installed" ] && [ "$installed" != "$ROOT/$id" ] || continue
        others="$others $(basename -- "$installed")"
    done
    if [ -n "$others" ]; then
        printf 'Also installed, and kept:%s\n' "$others"
        printf '  remove one with: rm -rf %s/<version>\n' "$ROOT"
    fi
    printf 'A daemon that is already running keeps running the version it started with.\n'
    case ":$PATH:" in
        *":$HOME/.local/bin:"*) ;;
        *) printf '\n%s is not on your PATH. Add it (zsh):\n' "$HOME/.local/bin"
           printf "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zprofile && . ~/.zprofile\n" ;;
    esac
    printf '\nNext: open %s, or run: circular --help\n' "$app_link"
}

main "$@"
