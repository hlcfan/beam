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
        base_version = version.split("-", 1)[0].split("+", 1)[0]
        build_version = f"{base_version}-nightly.{run_id}.{attempt}"
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive")
    parser.add_argument("--require-key", action="store_true")
    args = parser.parse_args()
    if args.require_key and not re.fullmatch(
        r"[0-9a-fA-F]{64}", os.environ.get("BEAM_UPDATE_PUBLIC_KEY", "")
    ):
        parser.error("Set the BEAM_UPDATE_PUBLIC_KEY repository variable to the public key hex.")
    with Path("Cargo.toml").open("rb") as manifest:
        version = tomllib.load(manifest)["package"]["version"]
    try:
        metadata = release_metadata(version, os.environ, args.archive)
    except (KeyError, ValueError) as error:
        parser.error(str(error))
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        for key, value in metadata.items():
            output.write(f"{key}={value}\n")
    with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as output:
        output.write(f"BEAM_BUILD_VERSION={metadata['build_version']}\n")


if __name__ == "__main__":
    main()
