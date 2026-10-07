#!/usr/bin/env python3
"""Build the prebuilt daemon asset that a release publishes (macOS, Apple Silicon).

Run from a clean checkout of the published repository at the release tag:

    python3 scripts/build-artifact.py

It writes two files to dist/:

    circular-daemon-<target>.tar.gz          the asset
    circular-daemon-<target>.tar.gz.sha256   its SHA-256, in `shasum -a 256` format

The asset holds one directory, circular-daemon-<target>/, with the part of the image that
is built from Rust and the specification: bin/circular-daemon, self.json (the build's
identity and compatibility axes, projected by scripts/artifact-self.rs from the libraries
the daemon was linked with) and docs/ (the offline document bundle). The three travel
together.

Nothing else is built here. The CLI and Circular.app are built on the user's machine by
scripts/install.sh from the same source; install.sh also runs this script itself when a
release has no asset for the machine. Requires Rust, the Xcode command-line tools and
Python 3.9 or newer (the macOS system python3). No signing or notarization happens here.

The identity comes from git when the checkout has .git, otherwise from BUILD_INFO at the
workspace root. A tree with uncommitted or untracked changes is refused before anything is
built: install.sh refuses an asset built from one, so such an asset would install nowhere.
Existing outputs are never overwritten. A failure prints one line in install.sh's form,
`build-artifact: <REASON>: <sentence>`, with the reason word and exit code install.sh gives
the same fact (ASSET_SOURCE_MISMATCH 4, LOCATION_OCCUPIED 5, BUILD_FAILED 6).
docs/index.json is the file list
of the unchanged reference Markdown pages (the approved first bundle projection, not a
diagnostic-query index or a catalog/spec value copy).
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
# The one executable this asset carries. The `circular` launcher and Circular.app are
# installed beside it by scripts/install.sh.
BINARIES = ("circular-daemon",)

class Refused(Exception):
    """A failure with the reason word and exit code install.sh's table gives the same fact."""

    def __init__(self, code, reason, sentence):
        super().__init__(sentence)
        self.code = code
        self.reason = reason

def asset_name(target):
    """The file name a release publishes for one target; install.sh spells the same name."""
    return f"circular-daemon-{target}"

def run(args, *, cwd=ROOT, **kwargs):
    return subprocess.run([str(arg) for arg in args], cwd=cwd, check=True,
                          text=True, **kwargs)

def output(args, **kwargs):
    return run(args, stdout=subprocess.PIPE, **kwargs).stdout.strip()

def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")

def build_info(root):
    """Preserve archive provenance; never borrow an enclosing checkout's identity."""
    path = root / "BUILD_INFO"
    if (root / ".git").exists():
        info = {
            "git_sha": output(["git", "rev-parse", "HEAD"], cwd=root),
            "git_dirty": str(bool(output(["git", "status", "--porcelain"], cwd=root))).lower(),
            "commit_date": output(["git", "show", "-s", "--format=%cs", "HEAD"], cwd=root),
        }
    else:
        info = dict(line.split("=", 1) for line in path.read_text().splitlines() if "=" in line)
    if (not re.fullmatch(r"[0-9a-f]{12,40}", info.get("git_sha", ""))
            or info.get("git_dirty") not in ("true", "false")
            or not re.fullmatch(r"\d{4}-\d{2}-\d{2}", info.get("commit_date", ""))):
        raise ValueError("BUILD_INFO requires git_sha, git_dirty, commit_date provenance")
    if info["git_dirty"] == "true":
        where = ("the checkout has uncommitted or untracked files (git status lists them)"
                 if (root / ".git").exists() else "BUILD_INFO says the source has uncommitted changes")
        raise Refused(4, "ASSET_SOURCE_MISMATCH",
                      f"{where}, and the installer refuses a daemon asset built from such a tree; "
                      "build from a clean checkout of the commit")
    # Rewriting also invalidates transport's build identity for dirty-only changes.
    path.write_text("".join(f"{key}={info[key]}\n" for key in ("git_sha", "git_dirty", "commit_date")))
    return info

def build_image(stage, work, info, target):
    log = ROOT / "target/artifact-build.log"
    log.parent.mkdir(parents=True, exist_ok=True)
    print(f"Building the daemon with cargo (release); its whole output goes to {log}", flush=True)
    # Cargo's messages (JSON) and its progress lines both go to the log. A successful build
    # prints nothing more; a failed one prints its errors and names the log.
    args = ["cargo", "build", "--locked", "--release", "--target", target, "-p", "engine",
            "--message-format=json"]
    for binary in BINARIES:
        args += ["--bin", binary]
    with log.open("w") as stream:
        failed = subprocess.run([str(arg) for arg in args], cwd=ROOT, stdout=stream,
                                stderr=subprocess.STDOUT).returncode
    lines = log.read_text().splitlines()
    artifacts = [json.loads(line) for line in lines if line.startswith("{")]
    if failed:
        for item in artifacts:
            if item.get("reason") == "compiler-message" and item["message"]["level"] == "error":
                print(item["message"]["rendered"], file=sys.stderr)
        for line in [line for line in lines if not line.startswith("{")][-20:]:
            print(line, file=sys.stderr)
        raise ValueError(f"cargo build failed (exit {failed}); its whole output is {log}")
    print(next((line.strip() for line in reversed(lines) if line.strip().startswith("Finished")),
               "cargo build finished"), flush=True)
    libraries = {}
    # Cargo states where every library it compiled lives; nothing here is spelled by hand.
    # Both directories are needed: the cross-compiled rlibs and the host proc-macros.
    search = set()
    executables = {}
    for item in artifacts:
        if item.get("reason") != "compiler-artifact":
            continue
        name = item["target"]["name"]
        if item.get("executable"):
            executables[name] = Path(item["executable"])
        for filename in item["filenames"]:
            if Path(filename).suffix in (".rlib", ".rmeta", ".dylib", ".so"):
                search.add(Path(filename).parent)
            if filename.endswith(".rlib") and f"/{target}/" in filename:
                libraries[name] = Path(filename)
    (stage / "bin").mkdir()
    for binary in BINARIES:
        shutil.copy2(executables[binary], stage / "bin" / binary)

    # Link a build-time projection to the exact image libraries, without a new crate.
    helper = work / "artifact-self"
    args = ["rustc", "--edition=2024", "--target", target, ROOT / "scripts/artifact-self.rs",
            "-o", helper]
    for alias, name in (("circular_core", "circular_core"), ("circular_transport", "transport"),
                        ("serde_json", "serde_json")):
        args += ["--extern", f"{alias}={libraries[name]}"]
    # The cross-compiled directory answers first; the host one only carries proc-macros.
    for directory in sorted(search, key=lambda path: (f"/{target}/" not in str(path), str(path))):
        args += ["-L", f"dependency={directory}"]
    run(args)
    description = json.loads(output([helper]))
    identity = description["identity"]
    expected = {"git_sha": info["git_sha"][:12], "git_dirty": info["git_dirty"] == "true",
                "commit_date": info["commit_date"], "target": target}
    if any(identity[key] != value for key, value in expected.items()):
        raise ValueError("compiled BuildIdentity disagrees with BUILD_INFO")
    # Read the linked image's deployment requirement, never the host OS version.
    minimums = []
    for binary in BINARIES:
        load_commands = output(["otool", "-l", stage / "bin" / binary])
        versions = re.findall(r"\bminos\s+(\d+(?:\.\d+)+)", load_commands)
        if not versions:
            versions = re.findall(r"cmd LC_VERSION_MIN_MACOSX\s+cmdsize \d+\s+version (\d+(?:\.\d+)+)", load_commands)
        if not versions:
            raise ValueError(f"cannot read minimum macOS version from {binary}")
        minimums.extend(versions)
    identity["minimum_os_version"] = max(minimums, key=lambda v: tuple(map(int, v.split("."))))
    return description

def build_bundle(root, stage):
    docs = stage / "docs"
    docs.mkdir()
    pages = []
    # One enumeration produces both projections; retain all Markdown, including Retired.
    for source in sorted((root / "reference").rglob("*.md")):
        key = source.relative_to(root / "reference").as_posix()
        destination = docs / "pages" / key
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        pages.append(f"pages/{key}")
    if not pages:
        raise ValueError("empty reference declaration set")
    write_json(docs / "index.json", pages)

def validate_asset(asset, description, pages):
    """No publication with a missing or partial component."""
    for name in BINARIES:
        path = asset / "bin" / name
        if not path.is_file() or not os.access(path, os.X_OK):
            raise ValueError(f"missing executable: bin/{name}")
    docs = asset / "docs"
    actual = json.loads((docs / "index.json").read_text())
    if not pages or actual != pages or len(set(actual)) != len(actual):
        raise ValueError("bundle index differs from generated pages")
    if sorted(p.relative_to(docs).as_posix() for p in (docs / "pages").rglob("*.md")) != sorted(pages):
        raise ValueError("bundle pages differ from index")
    if json.loads((asset / "self.json").read_text()) != description:
        raise ValueError("self differs from compiled compatibility projection")
    if "identity" not in description or len(description) < 2:
        raise ValueError("self has no identity or compatibility axes")
    # The installer copies these files into another tree; a link would point outside it.
    for path in asset.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"link in the asset: {path.relative_to(asset)}")

def smoke(asset, description):
    version = output([asset / "bin/circular-daemon", "--version"], timeout=30)
    identity = description["identity"]
    expected = (f"circular-daemon {identity['version']} ({identity['git_sha']}"
                f"{'-dirty' if identity['git_dirty'] else ''} {identity['commit_date']} {identity['target']})")
    if version.splitlines()[0] != expected:
        raise ValueError("daemon --version disagrees with self identity")
    print(version, flush=True)

def anonymous(member):
    """The archive names no account of the machine that built it."""
    member.uid = member.gid = 0
    member.uname = member.gname = ""
    return member

def pack(asset, archive):
    with tarfile.open(archive, "w:gz") as stream:
        stream.add(asset, arcname=asset.name, filter=anonymous)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum = archive.with_name(archive.name + ".sha256")
    checksum.write_text(f"{digest}  {archive.name}\n")
    return checksum

def build_asset():
    # First: a tree the installer would refuse is refused before anything is built.
    info = build_info(ROOT)
    target = next(line[len("host: "):] for line in output(["rustc", "-vV"]).splitlines()
                  if line.startswith("host: "))
    name = asset_name(target)
    dist = ROOT / "dist"
    dist.mkdir(exist_ok=True)
    archive = dist / f"{name}.tar.gz"
    for existing in (archive, archive.with_name(archive.name + ".sha256")):
        if existing.exists():
            raise Refused(5, "LOCATION_OCCUPIED",
                          f"{existing} already exists; remove it or use a fresh checkout")
    with tempfile.TemporaryDirectory(prefix=".asset-", dir=dist) as temporary:
        work = Path(temporary)
        stage = work / name
        stage.mkdir()
        description = build_image(stage, work, info, target)
        build_bundle(ROOT, stage)
        write_json(stage / "self.json", description)
        pages = json.loads((stage / "docs/index.json").read_text())
        validate_asset(stage, description, pages)
        smoke(stage, description)
        partial = work / archive.name
        checksum = pack(stage, partial)
        partial.rename(archive)
        checksum.rename(dist / checksum.name)
    print(f"Asset: {archive}", flush=True)
    print(f"SHA-256: {dist / checksum.name}", flush=True)
    return archive

if __name__ == "__main__":
    argparse.ArgumentParser(description=__doc__,
                            formatter_class=argparse.RawDescriptionHelpFormatter).parse_args()
    try:
        build_asset()
    except Refused as refused:
        print(f"build-artifact: {refused.reason}: {refused}", file=sys.stderr)
        sys.exit(refused.code)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"build-artifact: BUILD_FAILED: {error}", file=sys.stderr)
        sys.exit(6)
