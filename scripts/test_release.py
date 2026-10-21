"""Check the release signing contract shared by CI and the application updater."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from sign_release import sign_release


class ReleaseTests(unittest.TestCase):
    def test_missing_signing_configuration_never_changes_the_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "beam.zip"
            archive.write_bytes(b"unsigned archive")
            with patch.dict("os.environ", {}, clear=True):
                with self.assertRaises(ValueError):
                    sign_release(archive)
            self.assertEqual(archive.read_bytes(), b"unsigned archive")
            self.assertFalse(archive.with_suffix(".zip.sha256").exists())


if __name__ == "__main__":
    unittest.main()
