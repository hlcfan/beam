"""Smoke-test the real installer on a disposable Windows CI runner."""

import os
from pathlib import Path
import subprocess
import sys
import time
import winreg


def main():
    installer = Path(sys.argv[1]).resolve()
    destination = Path(os.environ['LOCALAPPDATA']) / 'Programs/Beam'
    shortcut = Path(os.environ['APPDATA']) / 'Microsoft/Windows/Start Menu/Programs/Beam.lnk'
    key = r'Software\Microsoft\Windows\CurrentVersion\Uninstall\Beam'
    # Never replace an existing installation when invoking this test manually.
    if destination.exists() or shortcut.exists():
        raise RuntimeError('Run this test on a disposable runner without Beam installed.')
    for _ in range(2):
        subprocess.run([str(installer), '/S'], check=True, timeout=120)
        assert (destination / 'beam.exe').is_file()
        assert (destination / 'Uninstall.exe').is_file()
        assert shortcut.is_file()
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key, 0,
                            winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as entry:
            assert winreg.QueryValueEx(entry, 'InstallLocation')[0] == str(destination)
    # Unknown files must survive uninstall, as must workspace data outside this directory.
    retained = destination / 'keep.txt'
    retained.write_text('retain')
    subprocess.run([str(destination / 'Uninstall.exe'), '/S'], check=True, timeout=120)
    # NSIS runs the uninstaller from a temporary copy, so wait for its completion.
    deadline = time.monotonic() + 30
    while (destination / 'Uninstall.exe').exists() and time.monotonic() < deadline:
        time.sleep(0.2)
    assert not (destination / 'beam.exe').exists()
    assert not (destination / 'Uninstall.exe').exists()
    assert not shortcut.exists()
    assert retained.read_text() == 'retain'
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key, 0,
                            winreg.KEY_READ | winreg.KEY_WOW64_64KEY):
            raise AssertionError('Uninstall registration remains')
    except FileNotFoundError:
        pass
    retained.unlink()
    destination.rmdir()


if __name__ == '__main__':
    main()
