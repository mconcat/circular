"""Witnesses for scripts/install.sh, the one-line installer.

Every case runs the real installer with a fresh HOME. The refusal cases stop it before
anything is fetched or built and check that it said why, with which exit code, and that it
left HOME as it found it. The release cases run a copy of the script whose two constants
(CIRCULAR_REPO, CIRCULAR_TAG) point at a local stand-in repository with tagged releases, the
way the one-line command's copy points at the published one. The end-to-end case installs for
real from this checkout and a daemon asset built from it (python3 scripts/build-artifact.py);
it runs only when dist/ holds that asset for this checkout's HEAD commit, because building the
app takes minutes.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
INSTALLER = ROOT / 'scripts/install.sh'
ASSET = 'circular-daemon-aarch64-apple-darwin'
# A Finder or Dock launch has this PATH, with no node and no cargo on it.
GUI_PATH = '/usr/bin:/bin:/usr/sbin:/sbin'
RUNS = b'#!/bin/sh\n[ "$1" = --version ] && { echo "circular-daemon fixture"; exit 0; }\nexit 91\n'
DOES_NOT_RUN = b'#!/bin/sh\necho "dyld: fixture library not loaded" >&2\nexit 134\n'

def snapshot(root):
    return {str(p.relative_to(root)): (p.lstat().st_mode,
            os.readlink(p) if p.is_symlink() else p.read_bytes() if p.is_file() else None)
            for p in root.rglob('*')}

def listing(root):
    """Link targets, and modes, sizes and modification times of the rest: cheap for an image."""
    return {str(p.relative_to(root)): os.readlink(p) if p.is_symlink()
            else (p.lstat().st_mode, p.lstat().st_size, p.lstat().st_mtime_ns)
            for p in root.rglob('*')}

def git(*args, cwd):
    return subprocess.run(['git', '-c', 'user.name=fixture', '-c', 'user.email=fixture@invalid',
                           *args], cwd=cwd, check=True, capture_output=True, text=True).stdout.strip()

def fake_asset(directory, *, git_sha, git_dirty=False, daemon=RUNS, checksum=None, sha_file=True):
    """A daemon asset in the shape build-artifact.py writes, with a stand-in daemon."""
    directory.mkdir(parents=True, exist_ok=True)
    archive = directory / f'{ASSET}.tar.gz'
    members = {
        'bin/circular-daemon': (daemon, 0o755),
        'self.json': (json.dumps({'identity': {'version': '0.0.0', 'git_sha': git_sha,
                                               'git_dirty': git_dirty}}).encode(), 0o644),
        'docs/index.json': (b'["pages/fixture.md"]', 0o644),
        'docs/pages/fixture.md': (b'# Fixture\n', 0o644),
    }
    with tarfile.open(archive, 'w:gz') as stream:
        for name, (data, mode) in members.items():
            info = tarfile.TarInfo(f'{ASSET}/{name}')
            info.size, info.mode = len(data), mode
            stream.addfile(info, io.BytesIO(data))
    if sha_file:
        digest = checksum or hashlib.sha256(archive.read_bytes()).hexdigest()
        (directory / f'{ASSET}.tar.gz.sha256').write_text(f'{digest}  {archive.name}\n')
    return archive

class InstallerTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='circular-installer-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.home = self.root / 'home with spaces'
        self.home.mkdir()
        self.state = self.home / '.circular/state'
        self.state.mkdir(parents=True)
        (self.state / 'existing-record').write_bytes(b'preserve this state\0')
        self.cwd = self.root / 'working-directory'
        self.cwd.mkdir()
        # Child-process isolation only; the installer reads no setting from the environment.
        self.environment = dict(os.environ, HOME=str(self.home), TMPDIR=str(self.root))
        self.installer = INSTALLER

    def invoke(self, *args, expected, reason=None, environment=None, home_unchanged=None):
        before = snapshot(self.home)
        result = subprocess.run(['sh', str(self.installer), *args], cwd=self.cwd,
                                env=environment or self.environment, stdin=subprocess.DEVNULL,
                                capture_output=True, text=True, timeout=1200)
        self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
        if reason is not None:
            self.assertIn(f'install: {reason}: ', result.stderr)
        if home_unchanged if home_unchanged is not None else expected != 0:
            self.assertEqual(snapshot(self.home), before)
        self.assertEqual(list(self.cwd.iterdir()), [])
        # The work directory and any staging directory are removed on every exit.
        self.assertEqual(list(self.root.glob('circular-install.*')), [])
        self.assertEqual(list(self.home.glob('.local/opt/circular/.stage-*')), [])
        return result

    def small_checkout(self, name='checkout'):
        """A git checkout with one commit: enough to reach the identity check, nothing more."""
        checkout = self.root / name
        checkout.mkdir()
        (checkout / 'README').write_text('fixture\n')
        git('init', '-q', cwd=checkout)
        git('add', 'README', cwd=checkout)
        git('commit', '-qm', 'fixture', cwd=checkout)
        return checkout, git('rev-parse', 'HEAD', cwd=checkout)

    def released_copy(self, repository, tag):
        """This script as a release would publish it: its two constants name one repository and tag."""
        text = INSTALLER.read_text()
        text, repositories = re.subn(r"^CIRCULAR_REPO='[^']*'$", f"CIRCULAR_REPO='file://{repository}'",
                                     text, flags=re.M)
        text, tags = re.subn(r"^CIRCULAR_TAG='[^']*'$", f"CIRCULAR_TAG='{tag}'", text, flags=re.M)
        self.assertEqual((repositories, tags), (1, 1), 'the repository and the tag are each named once')
        copy = self.root / f'install-{tag}.sh'
        copy.write_text(text)
        self.installer = copy

    def without_cargo(self):
        tools = self.root / 'tools'
        tools.mkdir(exist_ok=True)
        for name in ('node', 'npm'):
            if not (tools / name).exists():
                (tools / name).symlink_to(shutil.which(name))
        return dict(self.environment, PATH=f'{tools}:{GUI_PATH}')

    def test_arguments(self):
        result = self.invoke('--help', expected=0)
        self.assertIn('curl -fsSL', result.stdout)
        self.invoke('--prefix', str(self.root), expected=2, reason='USAGE')
        self.invoke('--source', expected=2, reason='USAGE')
        self.invoke('--daemon', '', expected=2, reason='USAGE')

    def test_node_22_is_required(self):
        tools = self.root / 'tools'
        tools.mkdir()
        without_node = dict(self.environment, PATH=f'{tools}:{GUI_PATH}')
        self.invoke(expected=3, reason='NODE_REQUIRED', environment=without_node)
        node = tools / 'node'
        node.write_text('#!/bin/sh\nexit 1\n')
        node.chmod(0o755)
        self.invoke(expected=3, reason='NODE_REQUIRED', environment=without_node)

    def test_occupied_locations_are_refused_before_anything_is_fetched(self):
        command = self.home / '.local/bin/circular'
        command.parent.mkdir(parents=True)
        command.write_text('#!/bin/sh\necho the user wrote this\n')
        self.invoke(expected=5, reason='LOCATION_OCCUPIED')
        command.unlink()
        command.symlink_to('/usr/bin/true')
        self.invoke(expected=5, reason='LOCATION_OCCUPIED')
        command.unlink()
        app = self.home / 'Applications/Circular.app/Contents'
        app.mkdir(parents=True)
        self.invoke(expected=5, reason='LOCATION_OCCUPIED')
        shutil.rmtree(app.parent)
        current = self.home / '.local/opt/circular/current'
        current.mkdir(parents=True)
        self.invoke(expected=5, reason='LOCATION_OCCUPIED')

    def test_a_given_daemon_asset_is_verified_before_the_source_is_fetched(self):
        checkout, commit = self.small_checkout()
        wrong = fake_asset(self.root / 'wrong', git_sha=commit[:12], checksum='0' * 64)
        self.invoke('--source', str(checkout), '--daemon', str(wrong),
                    expected=4, reason='ASSET_CHECKSUM_MISMATCH')
        unlisted = fake_asset(self.root / 'unlisted', git_sha=commit[:12], sha_file=False)
        self.invoke('--source', str(checkout), '--daemon', str(unlisted),
                    expected=4, reason='ASSET_CHECKSUM_MISSING')
        self.invoke('--source', str(checkout), '--daemon', str(self.root / 'absent.tar.gz'),
                    expected=4, reason='ASSET_UNAVAILABLE')
        broken = fake_asset(self.root / 'broken', git_sha=commit[:12], daemon=DOES_NOT_RUN)
        result = self.invoke('--source', str(checkout), '--daemon', str(broken),
                             expected=4, reason='ASSET_NOT_RUNNABLE')
        # What the daemon said is kept, not discarded.
        self.assertIn('circular-daemon --version: dyld: fixture library not loaded', result.stderr)
        self.assertNotIn('Taking the source', result.stdout)

    def test_daemon_asset_and_source_must_be_one_clean_commit(self):
        checkout, commit = self.small_checkout()
        other = fake_asset(self.root / 'other', git_sha='0' * 12)
        result = self.invoke('--source', str(checkout), '--daemon', str(other),
                             expected=4, reason='ASSET_SOURCE_MISMATCH')
        self.assertIn(commit[:12], result.stderr)
        dirty = fake_asset(self.root / 'dirty', git_sha=commit[:12], git_dirty=True)
        self.invoke('--source', str(checkout), '--daemon', str(dirty),
                    expected=4, reason='ASSET_SOURCE_MISMATCH')
        plain = self.root / 'not-a-checkout'
        plain.mkdir()
        self.invoke('--source', str(plain), expected=4, reason='SOURCE_UNAVAILABLE')

    def test_without_a_prebuilt_daemon_rust_is_required(self):
        checkout, _ = self.small_checkout()
        result = self.invoke('--source', str(checkout), expected=3, reason='RUST_REQUIRED',
                             environment=self.without_cargo())
        self.assertIn('https://rustup.rs', result.stderr)

    def test_a_released_script_installs_only_its_own_tag(self):
        # A stand-in published repository: two tagged commits, and a release directory per
        # tag laid out as GitHub serves it (releases/download/<tag>/<asset>).
        repository, first = self.small_checkout('published')
        git('tag', 'v0.1.0-fixture.1', cwd=repository)
        (repository / 'README').write_text('fixture, second release\n')
        git('commit', '-qam', 'second', cwd=repository)
        second = git('rev-parse', 'HEAD', cwd=repository)
        git('tag', 'v0.1.0-fixture.2', cwd=repository)
        releases = repository / 'releases/download'
        fake_asset(releases / 'v0.1.0-fixture.1', git_sha=first[:12])
        # The second release's asset was built from the first release's commit.
        fake_asset(releases / 'v0.1.0-fixture.2', git_sha=first[:12])

        self.released_copy(repository, 'v0.1.0-fixture.1')
        result = self.invoke(expected=6, reason='BUILD_FAILED', home_unchanged=False)
        own = hashlib.sha256((releases / f'v0.1.0-fixture.1/{ASSET}.tar.gz').read_bytes()).hexdigest()
        self.assertIn('installing Circular v0.1.0-fixture.1', result.stdout)
        self.assertIn(f'SHA-256 {own}', result.stdout)
        self.assertIn('Downloading the source of v0.1.0-fixture.1', result.stdout)
        self.assertIn(f'commit {first[:12]}', result.stdout)
        self.assertNotIn('fixture.2', result.stdout + result.stderr)
        self.assertNotIn('latest', result.stdout + result.stderr)
        # It passed the commit check and failed only where the stand-in source has no SDK.
        self.assertIn('npm ci failed in sdk/typescript', result.stderr)
        self.assertEqual([p.name for p in (self.home / '.local/opt/circular').iterdir()], [])

        self.released_copy(repository, 'v0.1.0-fixture.2')
        result = self.invoke(expected=4, reason='ASSET_SOURCE_MISMATCH')
        self.assertIn(f'built from commit {first[:12]} and the source is commit {second[:12]}',
                      result.stderr)

    def test_a_release_daemon_that_cannot_be_used_is_a_coded_fall_back(self):
        repository, commit = self.small_checkout('published')
        releases = repository / 'releases/download'
        cases = {
            # No release directory at all: curl cannot fetch the asset.
            'v0.1.0-fixture.absent': ('ASSET_UNAVAILABLE', None),
            'v0.1.0-fixture.unlisted': ('ASSET_CHECKSUM_MISSING', {'sha_file': False}),
            'v0.1.0-fixture.broken': ('ASSET_NOT_RUNNABLE', {'daemon': DOES_NOT_RUN}),
        }
        for tag, (reason, asset) in cases.items():
            with self.subTest(tag=tag):
                git('tag', tag, cwd=repository)
                if asset is not None:
                    fake_asset(releases / tag, git_sha=commit[:12], **asset)
                self.released_copy(repository, tag)
                # Without Rust the fall back cannot finish; the coded line comes first.
                result = self.invoke(expected=3, reason='RUST_REQUIRED', environment=self.without_cargo())
                lines = result.stderr.splitlines()
                self.assertTrue(lines[0].startswith(f'install: {reason}: '), result.stderr)
                self.assertTrue(lines[0].endswith('the daemon will be built from source'), lines[0])
                self.assertIn(f'commit {commit[:12]}', result.stdout)
                if reason == 'ASSET_NOT_RUNNABLE':
                    self.assertIn('circular-daemon --version: dyld: fixture library not loaded',
                                  result.stderr)

    def test_end_to_end_from_this_checkout(self):
        asset = ROOT / 'dist' / f'{ASSET}.tar.gz'
        head = git('rev-parse', 'HEAD', cwd=ROOT)
        if not asset.is_file():
            self.skipTest('no daemon asset in dist/; run python3 scripts/build-artifact.py')
        with tarfile.open(asset) as stream:
            identity = json.load(stream.extractfile(f'{ASSET}/self.json'))['identity']
        if identity['git_sha'] != head[:12]:
            self.skipTest(f'dist/ holds the asset of {identity["git_sha"]}, not of HEAD {head[:12]}')
        self.invoke('--source', str(ROOT), '--daemon', str(asset), expected=0)
        installations = self.home / '.local/opt/circular'
        identity_directory = f'{identity["version"]}-{head[:12]}'
        self.assertEqual(os.readlink(installations / 'current'), identity_directory)
        self.assertEqual(sorted(p.name for p in installations.iterdir()),
                         sorted(['current', identity_directory]))
        image = installations / identity_directory
        command = self.home / '.local/bin/circular'
        self.assertEqual(os.readlink(command), str(installations / 'current/bin/circular'))
        self.assertEqual(os.readlink(self.home / 'Applications/Circular.app'),
                         str(installations / 'current/bin/Circular.app'))
        self.assertEqual((image / 'bin/circular').read_bytes(),
                         (ROOT / 'scripts/circular-launcher.sh').read_bytes())
        # The installation keeps the source tree, not a clone's history or the app's build tools.
        self.assertEqual(sorted(p.name for p in image.iterdir()), ['bin', 'share', 'src'])
        self.assertFalse((image / 'src/.git').exists())
        self.assertFalse((image / 'src/ui/app/node_modules').exists())
        # From the SDK checkout, the public documents sit where the CLI looks for them.
        sdk = image / 'src/sdk/typescript'
        found = subprocess.run(['node', '--input-type=module', '-e', '''
const m = await import(process.argv[1]);
const agents = m.publicInstruction("AGENTS.md");
const rows = m.referencedDocuments(agents.path && (await import("node:path")).dirname(agents.path), "");
console.log(JSON.stringify({ origin: agents.origin, rows }));
''', str(sdk / 'chat/session-instructions.mjs')], capture_output=True, text=True, check=True)
        documents = json.loads(found.stdout)
        self.assertEqual(documents['origin'], 'checkout')
        actors = next(row['path'] for row in documents['rows'] if row['name'] == 'reference/actors/')
        self.assertTrue(actors and Path(actors).resolve().is_relative_to(image / 'src'), documents)
        self.assertTrue(any(Path(actors).glob('*.md')), actors)
        # The command on PATH runs with the PATH of a Finder or Dock launch: no node on it.
        gui = dict(self.environment, PATH=GUI_PATH)
        result = subprocess.run([str(command), '--version'], cwd=self.cwd, env=gui,
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(head[:8], result.stdout)
        result = subprocess.run([str(image / 'bin/circular-daemon'), '--version'],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f'({head[:12]} ', result.stdout)
        # The installation carries every template of the committed source tree, not a subset.
        committed = git('ls-files', 'sdk/typescript/templates/*/deploy.mjs', cwd=ROOT).splitlines()
        result = subprocess.run([str(command), 'template', 'list'], cwd=self.cwd, env=gui,
                                capture_output=True, text=True, timeout=60)
        self.assertEqual([line.split('\t')[0] for line in result.stdout.splitlines()],
                         sorted(Path(path).parent.name for path in committed))
        self.assertIn('hermes-dashboard', result.stdout)
        # Built or unpacked on this machine: nothing carries a quarantine attribute.
        for program in ('bin/circular-daemon', 'bin/Circular.app/Contents/MacOS/Circular'):
            attributes = subprocess.run(['xattr', str(image / program)], capture_output=True,
                                        text=True, check=True).stdout.split()
            self.assertNotIn('com.apple.quarantine', attributes, program)
        # No state directory was created or changed.
        self.assertEqual((self.state / 'existing-record').read_bytes(), b'preserve this state\0')
        self.assertEqual(sorted(p.name for p in self.state.iterdir()), ['existing-record'])
        self.assertFalse((self.home / 'Library/Application Support/Circular').exists())

        self.installed_command_finds_the_installed_daemon(command, installations, image, head)

        # Installing the same commit again selects the image already there.
        before = listing(installations)
        result = self.invoke('--source', str(ROOT), '--daemon', str(asset), expected=0)
        self.assertIn('is already installed', result.stdout)
        self.assertEqual(listing(installations), before)

        self.registration_names_the_installed_identity(installations, image)

    def installed_command_finds_the_installed_daemon(self, command, installations, image, head):
        """The installed `circular` names the daemon beside it, and that daemon starts.

        The CLI takes the account's home directory (os.userInfo), not HOME: it accepts a state
        only below the real home, and `circular daemon start` writes its operating log in the
        real ~/Library/Logs. So no daemon is started through the CLI here. `circular daemon
        status` asks the same function `start` uses which program it would run, and writes
        nothing; its state is a private directory below the real home, removed afterwards. The
        installed daemon is then started from the current selection with an injected HOME, a
        short directory under /tmp (a socket path fits in 103 bytes), and answers through the
        installed SDK's own transport.
        """
        state = Path(tempfile.mkdtemp(prefix='.circular-a12-e2e-', dir=Path.home())).resolve()
        self.addCleanup(shutil.rmtree, state, True)
        os.chmod(state, 0o700)
        result = subprocess.run([str(command), 'daemon', 'status', '--state', str(state), '--json'],
                                cwd=self.cwd, env=dict(self.environment, PATH=GUI_PATH),
                                capture_output=True, text=True, timeout=120)
        # `daemon status` exits 1 when no daemon answers for the state; none does here.
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(value['installedDaemon'], str(image.resolve() / 'bin/circular-daemon'))
        self.assertIn(f'({head[:12]} ', value['installedDaemonVersion'])
        self.assertEqual((value['running'], value['answering']), (False, False))
        self.assertFalse(Path(value['log']).exists(), value['log'])
        self.assertEqual(os.listdir(state), [])

        home = Path(tempfile.mkdtemp(prefix='a12s-', dir='/tmp')).resolve()
        self.addCleanup(shutil.rmtree, home, True)
        running = home / 's'
        running.mkdir(mode=0o700)
        environment = dict(self.environment, HOME=str(home))
        with open(home / 'daemon.out', 'wb') as out:
            daemon = subprocess.Popen([str(installations / 'current/bin/circular-daemon'),
                                       '--state', str(running)], cwd=self.cwd, env=environment,
                                      stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT)
        self.addCleanup(lambda: daemon.poll() is None and (daemon.kill(), daemon.wait()))
        ask = ('const { healthOf } = await import(process.argv[1]);\n'
               'console.log(JSON.stringify(await healthOf(process.argv[2])));\n')
        health = {'answering': False}
        deadline = time.monotonic() + 180
        while not health['answering'] and daemon.poll() is None and time.monotonic() < deadline:
            asked = subprocess.run(['node', '--input-type=module', '-e', ask,
                                    str(image / 'src/sdk/typescript/cli/daemon.mjs'), str(running)],
                                   cwd=self.cwd, env=environment, capture_output=True, text=True,
                                   check=True, timeout=60)
            health = json.loads(asked.stdout)
            if not health['answering']:
                time.sleep(0.2)
        said = (home / 'daemon.out').read_text(errors='replace')
        self.assertTrue(health['answering'], f'{health}\n{said}')
        daemon.terminate()
        self.assertEqual(daemon.wait(timeout=120), 0, said)

    def registration_names_the_installed_identity(self, installations, image):
        """The installed daemon registers itself as a LaunchAgent in an injected HOME: no launchctl.

        The state's socket path must fit in 103 bytes, so this HOME is a short directory of its
        own under /tmp, as crates/engine/tests/launch_agent_registration.rs does.
        """
        home = Path(tempfile.mkdtemp(prefix='a12r-', dir='/tmp')).resolve()
        self.addCleanup(shutil.rmtree, home, True)
        environment = dict(self.environment, HOME=str(home))
        plists = home / 'Library/LaunchAgents'
        state = home / 's'
        managed = home / 'Library/Application Support/Circular'

        def register(action):
            return subprocess.run([str(installations / 'current/bin/circular-daemon'), action,
                                   '--state', str(state)], cwd=self.cwd, env=environment,
                                  capture_output=True, text=True, timeout=60)

        for moved in (False, True):
            if moved:
                # A second installation selected in place of the first.
                target = image.with_name(image.name + ' moved')
                image.rename(target)
                image = target
                (installations / 'current').unlink()
                (installations / 'current').symlink_to(image.name)
            result = register('install')
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            names = [plist.name for plist in plists.glob('*.plist')]
            self.assertEqual(len(names), 1, names)
            self.assertTrue(names[0].startswith('dev.circular.daemon.'), names)
            text = (plists / names[0]).read_text()
            self.assertIn(str(image.resolve() / 'bin/circular-daemon'), text)
            self.assertNotIn('/current/', text)
            self.assertIn(str(state.resolve()), text)
            self.assertEqual(state.stat().st_mode & 0o7777, 0o700)
            self.assertFalse(managed.exists())
        # Unregistering works after the installation is gone, from another copy of the daemon.
        other = self.root / 'other-daemon'
        shutil.copy2(image / 'bin/circular-daemon', other)
        shutil.rmtree(image)
        result = subprocess.run([str(other), 'uninstall', '--state', str(state)], cwd=self.cwd,
                                env=environment, capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(list(plists.glob('*.plist')), [])

if __name__ == '__main__':
    unittest.main()
