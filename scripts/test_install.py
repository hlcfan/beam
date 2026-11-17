"""Exercise the installer offline with a release archive and mocked curl."""

import hashlib
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


@unittest.skipUnless(sys.platform.startswith(('linux', 'darwin')), 'installer targets Unix')
class InstallTests(unittest.TestCase):
    def run_install(self, corrupt=False, missing_assets=False, macos=False, arch='x86_64'):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / 'home with spaces & pipes| and % dollar$'
            home.mkdir()
            payload = root / ('beam-macos-aarch64' if macos else f'beam-linux-{arch}')
            payload.mkdir()
            (payload / 'beam').write_text('#!/bin/sh\necho beam\n')
            if not missing_assets:
                for asset in (ROOT / 'assets/linux').iterdir():
                    (payload / asset.name).write_bytes(asset.read_bytes())
                (payload / 'com.hlcfan.beam.png').write_bytes(b'icon')
            if macos:
                contents = payload / 'Beam.app/Contents'
                (contents / 'MacOS').mkdir(parents=True)
                (contents / 'Info.plist').write_text('<plist/>')
                binary = contents / 'MacOS/beam'
                binary.write_text('#!/bin/sh\necho beam\n')
                binary.chmod(0o755)
                if missing_assets:
                    binary.unlink()
            archive = root / (payload.name + '.tar.gz')
            with tarfile.open(archive, 'w:gz') as output:
                output.add(payload, arcname=payload.name)
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            (root / (archive.name + '.sha256')).write_text(
                f'{"0" * 64 if corrupt else digest}  {archive.name}\n'
            )
            tools = root / 'tools'
            tools.mkdir()
            curl = tools / 'curl'
            curl.write_text('''#!/bin/sh
case "$*" in
    *url_effective*) printf 'https://github.com/hlcfan/beam/releases/tag/v1.0.0'; exit 0 ;;
esac
while [ "$#" -gt 0 ]; do
    case "$1" in
        https://*) url=$1 ;;
        -o) shift; destination=$1 ;;
    esac
    shift
done
cp "$FIXTURE/${url##*/}" "$destination"
''')
            curl.chmod(0o755)
            # Exercise the macOS checksum command contract without requiring Perl on Linux.
            shasum = tools / 'shasum'
            shasum.write_text(f'#!{sys.executable}\n' + """import hashlib
import pathlib
import sys
assert sys.argv[1:4] == ['-a', '256', '-c']
checksum = pathlib.Path(sys.argv[4]).read_text().split()
actual = hashlib.sha256(pathlib.Path(checksum[1]).read_bytes()).hexdigest()
sys.exit(0 if actual == checksum[0] else 1)
""")
            shasum.chmod(0o755)
            if sys.platform == 'darwin':
                shasum.unlink()  # Use the native checksum tool on macOS.
            xattr = tools / 'xattr'
            xattr.write_text(f'#!{sys.executable}\n' + """import os
from pathlib import Path
import sys
assert sys.argv[1] == '-cr'
assert Path(sys.argv[2], 'Contents/MacOS/beam').is_file()
with Path(os.environ['FIXTURE'], 'xattr-calls').open('a') as output:
    output.write(sys.argv[2] + '\n')
sys.exit(int(os.environ.get('XATTR_FAIL', '0')))
""".replace("'\n'", "'\\n'"))
            xattr.chmod(0o755)
            uname = tools / 'uname'
            uname.write_text('#!/bin/sh\ncase "$1" in\n-s) echo ' +
                             ('Darwin' if macos else 'Linux') + '\n;;\n-m) echo ' +
                             ('arm64' if macos else arch) + '\n;;\nesac\n')
            uname.chmod(0o755)
            env = dict(os.environ, HOME=str(home), XDG_DATA_HOME=str(home / 'data'),
                       PATH=f'{tools}:/usr/bin:/bin', FIXTURE=str(root))
            result = subprocess.run(['sh', str(ROOT / 'scripts/install.sh')], env=env,
                                    capture_output=True, text=True)
            if macos:
                app = home / 'Applications/Beam.app'
                if corrupt or missing_assets:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertFalse(app.exists())
                    return
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertTrue(os.access(app / 'Contents/MacOS/beam', os.X_OK))
                self.assertEqual(len((root / 'xattr-calls').read_text().splitlines()), 1)
                # Failure to clear attributes must leave the installed app untouched.
                failed = subprocess.run(['sh', str(ROOT / 'scripts/install.sh')],
                                        env=dict(env, XATTR_FAIL='1'), capture_output=True)
                self.assertNotEqual(failed.returncode, 0)
                self.assertTrue((app / 'Contents/MacOS/beam').exists())
                # Replacing the bundle must remove files left by the old version.
                (app / 'obsolete').write_text('old')
                subprocess.run(['sh', str(ROOT / 'scripts/install.sh')], env=env,
                               check=True, capture_output=True)
                self.assertFalse((app / 'obsolete').exists())
                self.assertEqual(list((home / 'Applications').iterdir()), [app])
                return
            binary = home / '.local/bin/beam'
            if missing_assets:
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('does not include the desktop files', result.stderr)
                self.assertFalse(binary.exists())
                return
            if corrupt:
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(binary.exists())
                return
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(os.access(binary, os.X_OK))
            desktop = home / 'data/applications/com.hlcfan.beam.desktop'
            self.assertIn('StartupWMClass=com.hlcfan.beam', desktop.read_text())
            self.assertIn(f'Exec="{str(binary).replace("%", "%%").replace("$", chr(92) * 2 + "$")}"',
                          desktop.read_text())
            self.assertTrue((home / 'data/icons/hicolor/512x512/apps/com.hlcfan.beam.png').exists())
            # Reinstall over an existing executable.
            subprocess.run(['sh', str(ROOT / 'scripts/install.sh')], env=env,
                           check=True, capture_output=True)

    def test_macos_install_and_reinstall(self):
        self.run_install(macos=True)

    def test_macos_checksum_failure(self):
        self.run_install(macos=True, corrupt=True)

    def test_macos_invalid_bundle(self):
        self.run_install(macos=True, missing_assets=True)

    @unittest.skipUnless(sys.platform.startswith('linux'), 'requires GNU install')
    def test_install_and_reinstall(self):
        self.run_install()

    @unittest.skipUnless(sys.platform.startswith('linux'), 'requires GNU install')
    def test_arm64_install_uses_aarch64_archive(self):
        self.run_install(arch='aarch64')

    @unittest.skipUnless(sys.platform.startswith('linux'), 'requires GNU install')
    def test_release_without_desktop_files_fails_clearly(self):
        self.run_install(missing_assets=True)

    @unittest.skipUnless(sys.platform.startswith('linux'), 'requires GNU install')
    def test_checksum_failure_leaves_installation_untouched(self):
        self.run_install(corrupt=True)


if __name__ == '__main__':
    unittest.main()
