"""Write matching build versions and asset names for release workflow jobs."""

import argparse
import os
from pathlib import Path
import re
import tomllib


def release_metadata(version, environment, archive=None):
    if environment.get("GITHUB_REF_TYPE") == "tag":
        tag = environment["GITHUB_REF_NAME"]
        if tag != f"v{version}":
            raise ValueError("The release tag must match the version in Cargo.toml.")
        build_version = version
        title = tag
        nightly = False
    else:
        run_id = environment["GITHUB_RUN_ID"]
        attempt = environment["GITHUB_RUN_ATTEMPT"]
        if not run_id.isdecimal() or not attempt.isdecimal():
            raise ValueError("Nightly builds require numeric run and attempt identifiers.")
        commit_sha = environment.get("GITHUB_SHA", "")
        if not re.fullmatch(r"[0-9a-fA-F]{40}", commit_sha):
            raise ValueError("Nightly builds require a full Git commit SHA in GITHUB_SHA.")
        base_version = version.split("-", 1)[0].split("+", 1)[0]
        build_version = f"{base_version}-nightly.{run_id}.{attempt}+g{commit_sha[:12].lower()}"
        tag = "nightly"
        title = f"Nightly {build_version}"
        nightly = True
    result = {
        "build_version": build_version,
        "release_tag": tag,
        "release_name": title,
        "is_tip": str(nightly).lower(),
        "is_prerelease": str(nightly or "-" in version).lower(),
    }
    if archive:
        if nightly:
            extension = ".tar.gz" if archive.endswith(".tar.gz") else ".zip"
            archive = f"{archive.removesuffix(extension)}-{build_version}{extension}"
        result["archive_name"] = archive
    return result


def prepare_build_version(manifest_path, lock_path, version):
    manifest = manifest_path.read_text(encoding="utf-8")
    lock = lock_path.read_text(encoding="utf-8")
    name = tomllib.loads(manifest)["package"]["name"]
    manifest = with_package_version(manifest, "[package]", name, version)
    lock = with_package_version(lock, "[[package]]", name, version)
    manifest_path.write_text(manifest, encoding="utf-8")
    lock_path.write_text(lock, encoding="utf-8")


def with_package_version(text, header, name, version):
    matches = 0

    def replace(block):
        nonlocal matches
        package = tomllib.loads(block.group())["package"]
        if isinstance(package, list):
            package = package[0]
        if package["name"] != name:
            return block.group()
        matches += 1
        updated, count = re.subn(
            r'''(?m)^(version[ \t]*=[ \t]*)(["'])[^"'\n]+\2''',
            lambda match: f"{match[1]}{match[2]}{version}{match[2]}",
            block.group(),
            count=1,
        )
        if count != 1:
            raise ValueError(f"Expected an explicit version for {name}.")
        return updated

    pattern = rf"(?ms)^{re.escape(header)}[ \t]*\n.*?(?=^\[|\Z)"
    updated = re.sub(pattern, replace, text)
    if matches != 1:
        raise ValueError(f"Expected exactly one {name} package in {header}.")
    return updated


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive")
    parser.add_argument("--require-key", action="store_true")
    parser.add_argument("--prepare-build", action="store_true")
    args = parser.parse_args()
    if args.require_key and not re.fullmatch(
        r"[0-9a-fA-F]{64}", os.environ.get("BEAM_UPDATE_PUBLIC_KEY", "")
    ):
        parser.error("Set the BEAM_UPDATE_PUBLIC_KEY repository variable to the public key hex.")
    with Path("Cargo.toml").open("rb") as manifest:
        version = tomllib.load(manifest)["package"]["version"]
    try:
        metadata = release_metadata(version, os.environ, args.archive)
        if args.prepare_build:
            prepare_build_version(Path("Cargo.toml"), Path("Cargo.lock"), metadata["build_version"])
    except (KeyError, ValueError) as error:
        parser.error(str(error))
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        for key, value in metadata.items():
            output.write(f"{key}={value}\n")


if __name__ == "__main__":
    main()
