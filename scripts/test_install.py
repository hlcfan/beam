"""Exercise the installer offline with a release archive and mocked curl."""

import hashlib
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class InstallTests(unittest.TestCase):
    def run_install(self, corrupt=False):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / 'home with spaces & pipes| and % dollar$'
            home.mkdir()
            payload = root / 'beam-linux-x86_64'
            payload.mkdir()
            (payload / 'beam').write_text('#!/bin/sh\necho beam\n')
            for asset in (ROOT / 'assets/linux').iterdir():
                (payload / asset.name).write_bytes(asset.read_bytes())
            (payload / 'com.hlcfan.beam.png').write_bytes(b'icon')
            archive = root / 'beam-linux-x86_64.tar.gz'
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
            env = dict(os.environ, HOME=str(home), XDG_DATA_HOME=str(home / 'data'),
                       PATH=f'{tools}:/usr/bin:/bin', FIXTURE=str(root))
            result = subprocess.run(['sh', str(ROOT / 'scripts/install.sh')], env=env,
                                    capture_output=True, text=True)
            binary = home / '.local/bin/beam'
            if corrupt:
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(binary.exists())
                return
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue(os.access(binary, os.X_OK))
            desktop = home / 'data/applications/com.hlcfan.beam.desktop'
            self.assertIn(f'Exec="{str(binary).replace("%", "%%").replace("$", chr(92) * 2 + "$")}"',
                          desktop.read_text())
            self.assertTrue((home / 'data/icons/hicolor/512x512/apps/com.hlcfan.beam.png').exists())
            # Reinstall over an existing executable.
            subprocess.run(['sh', str(ROOT / 'scripts/install.sh')], env=env,
                           check=True, capture_output=True)

    def test_install_and_reinstall(self):
        self.run_install()

    def test_checksum_failure_leaves_installation_untouched(self):
        self.run_install(corrupt=True)


if __name__ == '__main__':
    unittest.main()
