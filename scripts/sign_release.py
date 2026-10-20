"""Sign and verify a packaged update, then write its SHA-256 checksum."""

import argparse
import base64
import binascii
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile


def sign_release(archive):
    key_value = os.environ.get("BEAM_UPDATE_SIGNING_KEY", "")
    try:
        private_key = base64.b64decode(key_value, validate=True)
        public_key = bytes.fromhex(os.environ.get("BEAM_UPDATE_PUBLIC_KEY", ""))
    except (ValueError, binascii.Error) as error:
        raise ValueError("The update signing configuration is invalid.") from error
    if len(private_key) != 64 or len(public_key) != 32:
        raise ValueError("Configure BEAM_UPDATE_SIGNING_KEY and BEAM_UPDATE_PUBLIC_KEY before publishing.")
    kind = "tar" if archive.name.endswith(".tar.gz") else "zip"
    with tempfile.TemporaryDirectory(prefix="beam-release-signing-") as directory:
        private_path = Path(directory) / "private.key"
        public_path = Path(directory) / "public.key"
        private_path.write_bytes(private_key)
        subprocess.run(["zipsign", "gen-key", "--extract", str(private_path), str(public_path)], check=True)
        if public_path.read_bytes() != public_key:
            raise ValueError("The signing key does not match BEAM_UPDATE_PUBLIC_KEY.")
        subprocess.run(["zipsign", "sign", kind, str(archive), str(private_path)], check=True)
        subprocess.run(["zipsign", "verify", kind, str(archive), str(public_path)], check=True)
    with archive.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    archive.with_name(archive.name + ".sha256").write_text(
        f"{digest}  {archive.name}\n", encoding="utf-8"
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    try:
        sign_release(args.archive)
    except (ValueError, subprocess.CalledProcessError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
