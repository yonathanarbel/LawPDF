"""Create the exact release payload for offline Keychain-backed signing.

Only public build artifacts and release metadata enter this file. This utility
does not access private keys or publish a release.
"""
import argparse
import hashlib
import json
import re
from pathlib import Path

ASSETS = ("LawPDFSetup-x64.exe", "LawPDF-windows-portable-x64.zip", "LawPDF-macos.zip")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", args.version):
        parser.error("Use the exact X.Y.Z package version")
    if not re.fullmatch(r"[a-fA-F0-9]{40}", args.commit):
        parser.error("Use the complete source commit SHA")
    assets = []
    for name in ASSETS:
        path = args.directory / name
        size = path.stat().st_size
        if not 0 < size <= 1024 ** 3:
            parser.error(f"Unsupported artifact size: {name}")
        digest = hashlib.sha256()
        with path.open("rb") as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b""):
                digest.update(chunk)
        assets.append({"name": name, "sha256": digest.hexdigest(), "bytes": size})
    manifest = {"schema": 1, "purpose": "lawpdf-update-v1", "repository": "yonathanarbel/LawPDF",
                "version": args.version, "commit": args.commit.lower(), "assets": assets}
    (args.directory / "UPDATE-MANIFEST.json").write_bytes((json.dumps(manifest, indent=2) + "\n").encode("utf-8"))

if __name__ == "__main__":
    main()
