"""Witnesses for scripts/build-artifact.py, the daemon asset a release publishes.

An asset missing its executable, its offline document bundle or its self
description is never packed. The packed asset is checked with the tools scripts/install.sh
uses on the user's machine (shasum and tar), not with the builder's own code. A tree with
changes is refused before anything is built, because install.sh refuses an asset built from
one; the refusal line is spelled here by hand, in install.sh's reason-line form.
"""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("build_artifact", Path(__file__).with_name("build-artifact.py"))
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)

def complete_asset(root):
    asset = root / "circular-daemon-aarch64-apple-darwin"
    (asset / "bin").mkdir(parents=True)
    daemon = asset / "bin/circular-daemon"
    daemon.write_text("#!/bin/sh\nexit 0\n")
    daemon.chmod(0o755)
    (root / "reference").mkdir()
    (root / "reference/retired.md").write_text("# Retired\n")
    builder.build_bundle(root, asset)
    # Test fixture only: production values come from artifact-self.rs.
    description = {"identity": {"version": "fixture"}, "fixture-axis": {"state": "undeclared"}}
    builder.write_json(asset / "self.json", description)
    # The one page written above, spelled here rather than read back from the builder.
    pages = ["pages/retired.md"]
    return asset, description, pages

# The witness repositories read no configuration of the machine running the test.
GIT_ENV = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}

def git(root, *args):
    return subprocess.run(["git", "-c", "user.name=witness", "-c", "user.email=witness@example.invalid",
                           "-c", "commit.gpgsign=false", "-C", root, *args],
                          check=True, capture_output=True, text=True, env=GIT_ENV).stdout.strip()

def committed_checkout(root):
    """A git checkout holding a copy of the builder and the repository's two ignore lines for
    what the builder writes, all committed."""
    (root / "scripts").mkdir()
    shutil.copy2(Path(__file__).with_name("build-artifact.py"), root / "scripts/build-artifact.py")
    (root / ".gitignore").write_text("/dist/\n/BUILD_INFO\n")
    git(root, "init", "-q")
    git(root, "add", ".")
    git(root, "commit", "-q", "-m", "witness")
    return git(root, "rev-parse", "HEAD")

class DaemonAssetTest(unittest.TestCase):
    def test_a_tree_with_changes_is_refused_before_anything_is_built(self):
        refusal = ("build-artifact: ASSET_SOURCE_MISMATCH: the checkout has uncommitted or untracked "
                   "files (git status lists them), and the installer refuses a daemon asset built "
                   "from such a tree; build from a clean checkout of the commit\n")
        changes = {
            "untracked file": lambda root: (root / "notes.txt").write_text("not committed\n"),
            "changed tracked file": lambda root: (root / ".gitignore").write_text("/dist/\n/BUILD_INFO\n# edited\n"),
        }
        for name, change in changes.items():
            with self.subTest(change=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                committed_checkout(root)
                change(root)
                refused = subprocess.run([sys.executable, root / "scripts/build-artifact.py"], cwd=root,
                                         capture_output=True, text=True, env=GIT_ENV)
                self.assertEqual((refused.returncode, refused.stdout, refused.stderr), (4, "", refusal))
                for written in ("dist", "BUILD_INFO", "target"):
                    self.assertFalse((root / written).exists(), written)

    def test_a_clean_checkout_passes_and_its_own_build_info_leaves_it_clean(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            head = committed_checkout(root)
            # Twice: the BUILD_INFO the first call writes must not make the next build dirty.
            for attempt in (1, 2):
                info = builder.build_info(root)
                self.assertEqual((info["git_sha"], info["git_dirty"]), (head, "false"), attempt)
            self.assertEqual((root / "BUILD_INFO").read_text().splitlines()[:2],
                             [f"git_sha={head}", "git_dirty=false"])

    def test_missing_component_never_passes_publication_gate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            complete, description, pages = complete_asset(root)
            builder.validate_asset(complete, description, pages)
            missing = ["bin", "bin/circular-daemon", "docs", "docs/index.json",
                       "docs/pages/retired.md", "self.json"]
            for index, component in enumerate(missing):
                with self.subTest(missing=component):
                    partial = root / str(index) / complete.name
                    shutil.copytree(complete, partial)
                    path = partial / component
                    if path.is_dir():
                        shutil.rmtree(path)
                    else:
                        path.unlink()
                    with self.assertRaises((ValueError, OSError)):
                        builder.validate_asset(partial, description, pages)
            linked = root / "linked" / complete.name
            shutil.copytree(complete, linked)
            (linked / "docs/outside.md").symlink_to("/etc/hosts")
            with self.assertRaises(ValueError):
                builder.validate_asset(linked, description, pages)

    def test_packed_asset_is_what_the_installer_reads(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            asset, _, _ = complete_asset(root)
            archive = root / f"{asset.name}.tar.gz"
            checksum = builder.pack(asset, archive)
            self.assertEqual(checksum.name, f"{archive.name}.sha256")
            verified = subprocess.run(["/usr/bin/shasum", "-a", "256", "-c", checksum.name],
                                      cwd=root, capture_output=True, text=True)
            self.assertEqual(verified.returncode, 0, verified.stdout + verified.stderr)
            listed = subprocess.run(["/usr/bin/tar", "-tvzf", archive], capture_output=True,
                                    text=True, check=True).stdout.splitlines()
            names = sorted(line.split()[-1].rstrip("/") for line in listed)
            self.assertEqual(names, sorted([
                asset.name, f"{asset.name}/bin", f"{asset.name}/bin/circular-daemon",
                f"{asset.name}/docs", f"{asset.name}/docs/index.json", f"{asset.name}/docs/pages",
                f"{asset.name}/docs/pages/retired.md", f"{asset.name}/self.json"]))
            # Owner columns: uid and gid 0, no account name of the machine that built it.
            self.assertEqual({tuple(line.split()[2:4]) for line in listed}, {("0", "0")})
            unpacked = root / "unpacked"
            unpacked.mkdir()
            subprocess.run(["/usr/bin/tar", "-xzf", archive, "-C", unpacked], check=True)
            self.assertTrue((unpacked / asset.name / "bin/circular-daemon").stat().st_mode & 0o111)

if __name__ == "__main__":
    unittest.main()
