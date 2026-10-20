"""Check version and archive contracts shared by CI and the application updater."""

from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest.mock import patch

from release_metadata import prepare_build_version, release_metadata
from sign_release import sign_release


class ReleaseTests(unittest.TestCase):
    def test_nightly_prepares_cargo_version_without_changing_dependencies(self):
        manifest_text = '''# Package settings
[package]
name = "beam"
version = '0.8.0' # Keep this comment
edition = "2024"

[dependencies]
other = "0.8.0"
'''
        lock_text = '''version = 4

[[package]]
name = "other"
version = "0.8.0"

[[package]]
name = "beam"
version = "0.8.0"
dependencies = ["other"]
'''
        version = "0.8.0-nightly.123.1+g0123456789ab"
        with tempfile.TemporaryDirectory() as directory:
            manifest, lock = Path(directory) / "Cargo.toml", Path(directory) / "Cargo.lock"
            manifest.write_text(manifest_text)
            lock.write_text(lock_text)
            prepare_build_version(manifest, lock, version)
            self.assertEqual(tomllib.loads(manifest.read_text())["package"]["version"], version)
            packages = tomllib.loads(lock.read_text())["package"]
            self.assertEqual([package["version"] for package in packages], ["0.8.0", version])
            self.assertEqual(manifest.read_text(), manifest_text.replace("version = '0.8.0'", f"version = '{version}'"))
            self.assertEqual(lock.read_text(), lock_text.replace('name = "beam"\nversion = "0.8.0"', f'name = "beam"\nversion = "{version}"'))

    def test_missing_lock_entry_does_not_change_the_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest, lock = Path(directory) / "Cargo.toml", Path(directory) / "Cargo.lock"
            manifest_text = '[package]\nname = "beam"\nversion = "0.8.0"\n'
            manifest.write_text(manifest_text)
            lock.write_text('version = 4\n')
            with self.assertRaises(ValueError):
                prepare_build_version(manifest, lock, "0.8.0-nightly.123.1")
            self.assertEqual(manifest.read_text(), manifest_text)

    def test_normal_release_keeps_cargo_files_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest, lock = Path(directory) / "Cargo.toml", Path(directory) / "Cargo.lock"
            manifest_text = '[package]\nname = "beam"\nversion = "0.8.0"\n'
            lock_text = 'version = 4\n\n[[package]]\nname = "beam"\nversion = "0.8.0"\n'
            manifest.write_text(manifest_text)
            lock.write_text(lock_text)
            prepare_build_version(manifest, lock, "0.8.0")
            self.assertEqual(manifest.read_bytes(), manifest_text.encode())
            self.assertEqual(lock.read_bytes(), lock_text.encode())

    def test_stable_tags_must_match_the_compiled_version(self):
        environment = {"GITHUB_REF_TYPE": "tag", "GITHUB_REF_NAME": "v0.8.0"}
        metadata = release_metadata("0.8.0", environment, "beam-windows-x86_64.zip")
        self.assertEqual(metadata["build_version"], "0.8.0")
        self.assertEqual(metadata["archive_name"], "beam-windows-x86_64.zip")
        self.assertEqual(metadata["is_prerelease"], "false")
        with self.assertRaises(ValueError):
            release_metadata("0.9.0", environment)

    def test_nightly_reruns_use_distinct_versions_and_asset_names(self):
        environment = {"GITHUB_REF_TYPE": "branch", "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "1", "GITHUB_SHA": "0123456789abcdef0123456789abcdef01234567"}
        for archive, extension in [("beam-macos-aarch64.tar.gz", ".tar.gz"), ("beam-windows-x86_64.zip", ".zip")]:
            first = release_metadata("0.8.0", environment, archive)
            second = release_metadata("0.8.0", dict(environment, GITHUB_RUN_ATTEMPT="2"), archive)
            self.assertNotEqual(first["archive_name"], second["archive_name"])
            self.assertEqual(first["build_version"], "0.8.0-nightly.123.1+g0123456789ab")
            self.assertEqual(second["build_version"], "0.8.0-nightly.123.2+g0123456789ab")
            self.assertTrue(first["archive_name"].endswith(f"-{first['build_version']}{extension}"))
            self.assertEqual(first["release_name"], f"Nightly {first['build_version']}")
            self.assertEqual(first["release_tag"], "nightly")

    def test_nightly_requires_a_valid_commit_sha(self):
        environment = {"GITHUB_REF_TYPE": "branch", "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "1"}
        for sha in [None, "", "a" * 39, "g" * 40]:
            with self.subTest(sha=sha), self.assertRaises(ValueError):
                release_metadata("0.8.0", environment if sha is None else dict(environment, GITHUB_SHA=sha))

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
