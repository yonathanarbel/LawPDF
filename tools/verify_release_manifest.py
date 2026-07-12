#!/usr/bin/env python3
"""Verify the public LawPDF release manifest and packaged runtime assets."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import re
import sys
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--manifest", type=Path, default=Path("release-manifest.json"))
    parser.add_argument("--platform", choices=("host", "macos", "windows"), default="host")
    parser.add_argument("--require-platform-library", action="store_true")
    return parser.parse_args()


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def host_platform() -> str:
    return "windows" if platform.system().lower().startswith("win") else "macos"


def main() -> int:
    args = parse_args()
    root = args.root.resolve()
    manifest_path = args.manifest if args.manifest.is_absolute() else root / args.manifest
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    errors: list[str] = []
    checked: list[str] = []

    if manifest.get("schema_version") != "lawpdf-public-release-manifest-v1":
        errors.append("unexpected release manifest schema")

    cargo_text = (root / "Cargo.toml").read_text(encoding="utf-8")
    version_match = re.search(r'^version\s*=\s*"([^"]+)"', cargo_text, re.MULTILINE)
    cargo_version = version_match.group(1) if version_match else None
    if cargo_version != manifest["product"]["version"]:
        errors.append(
            f"Cargo version {cargo_version!r} != manifest version "
            f"{manifest['product']['version']!r}"
        )

    runtime_assets = manifest["runtime_assets"]
    for name in ("native_model", "context_model"):
        asset = runtime_assets[name]
        path = root / asset["path"]
        if not path.is_file():
            errors.append(f"missing {name}: {path}")
        elif sha256(path) != asset["sha256"]:
            errors.append(f"{name} sha256 mismatch")
        else:
            checked.append(asset["path"])

    selected_platform = host_platform() if args.platform == "host" else args.platform
    library = runtime_assets["platform_libraries"][selected_platform]
    library_path = root / library["path"]
    if library_path.is_file():
        if sha256(library_path) != library["sha256"]:
            errors.append(f"{selected_platform} library sha256 mismatch")
        else:
            checked.append(library["path"])
    elif args.require_platform_library:
        errors.append(f"missing required {selected_platform} library: {library_path}")

    contract_key = {"windows": "windows-x86_64", "macos": "macos-arm64"}[selected_platform]
    contract = manifest.get("package_contracts", {}).get(contract_key)
    if not contract:
        errors.append(f"missing package contract: {contract_key}")
    elif not (root / contract["builder"]).is_file():
        errors.append(f"missing package builder: {contract['builder']}")
    elif "--require-native" not in contract.get("runtime_verification", "") or (
        "--require-context" not in contract.get("runtime_verification", "")
    ):
        errors.append(f"incomplete packaged runtime verification: {contract_key}")

    report = {
        "schema_version": "lawpdf-public-release-verification-v1",
        "status": "ok" if not errors else "error",
        "platform": selected_platform,
        "checked": checked,
        "errors": errors,
    }
    print(json.dumps(report, indent=2))
    return 0 if not errors else 1


if __name__ == "__main__":
    sys.exit(main())
