"""Check version and archive contracts shared by CI and the application updater."""

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from release_metadata import release_metadata
from sign_release import sign_release


class ReleaseTests(unittest.TestCase):
    def test_stable_tags_must_match_the_compiled_version(self):
        environment = {"GITHUB_REF_TYPE": "tag", "GITHUB_REF_NAME": "v0.8.0"}
        metadata = release_metadata("0.8.0", environment, "beam-windows-x86_64.zip")
        self.assertEqual(metadata["build_version"], "0.8.0")
        self.assertEqual(metadata["archive_name"], "beam-windows-x86_64.zip")
        self.assertEqual(metadata["is_prerelease"], "false")
        with self.assertRaises(ValueError):
            release_metadata("0.9.0", environment)

    def test_nightly_reruns_use_distinct_versions_and_asset_names(self):
        environment = {"GITHUB_REF_TYPE": "branch", "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "1"}
        for archive, extension in [("beam-macos-aarch64.tar.gz", ".tar.gz"), ("beam-windows-x86_64.zip", ".zip")]:
            first = release_metadata("0.8.0", environment, archive)
            second = release_metadata("0.8.0", dict(environment, GITHUB_RUN_ATTEMPT="2"), archive)
            self.assertNotEqual(first["archive_name"], second["archive_name"])
            self.assertTrue(first["archive_name"].endswith(f"-{first['build_version']}{extension}"))
            self.assertEqual(first["release_name"], f"Nightly {first['build_version']}")
            self.assertEqual(first["release_tag"], "nightly")

    def test_prerelease_tags_remain_prereleases(self):
        metadata = release_metadata("0.9.0-beta.1", {"GITHUB_REF_TYPE": "tag", "GITHUB_REF_NAME": "v0.9.0-beta.1"})
        self.assertEqual(metadata["is_prerelease"], "true")

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
