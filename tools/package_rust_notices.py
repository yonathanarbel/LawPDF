#!/usr/bin/env python3
"""Package the locked Rust dependency inventory and upstream license texts."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    source_root = Path(__file__).resolve().parents[1]
    supplement_path = source_root / "third_party/rust-extra/manifest.json"
    supplements = {name: record for record in json.loads(supplement_path.read_text(encoding="utf-8"))
                   for name in record["packages"]}
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1",
        "--filter-platform", args.target,
    ], encoding="utf-8"))
    resolved = {node["id"] for node in metadata["resolve"]["nodes"]}
    packages = sorted((package for package in metadata["packages"]
                       if package["id"] in resolved and package["source"]),
                      key=lambda package: (package["name"], package["version"]))
    args.destination.mkdir(parents=True, exist_ok=True)
    license_root = args.destination / "rust-licenses"
    license_root.mkdir(exist_ok=True)
    with (args.destination / "THIRD_PARTY_RUST_LICENSES.csv").open("w", encoding="utf-8", newline="") as output:
        writer = csv.writer(output)
        writer.writerow(["name", "version", "license", "repository", "target"])
        for package in packages:
            if not package.get("license") and not package.get("license_file"):
                raise SystemExit("Missing license metadata: " + package["name"])
            writer.writerow([package["name"], package["version"], package.get("license") or "See license file", package.get("repository") or "", args.target])
            root = Path(package["manifest_path"]).parent
            texts = {path for path in root.iterdir() if path.is_file() and
                     path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE"))}
            if package.get("license_file"):
                texts.add(root / package["license_file"])
            destination = license_root / (package["name"] + "-" + package["version"])
            destination.mkdir(exist_ok=True)
            for source in sorted(texts):
                shutil.copyfile(source, destination / source.name)
            if not texts:
                key = package["name"] + "-" + package["version"]
                supplement = supplements.get(key)
                if not supplement:
                    raise SystemExit("Missing upstream license texts: " + key)
                for record in supplement["files"]:
                    source = source_root / record["path"]
                    if hashlib.sha256(source.read_bytes()).hexdigest() != record["sha256"]:
                        raise SystemExit("Upstream license hash mismatch: " + str(source))
                    shutil.copyfile(source, destination / source.name)
                (destination / "LAWPDF-NOTICE-SOURCES.json").write_text(
                    json.dumps(supplement, indent=2) + "\n", encoding="utf-8")
    print("Packaged notices for", len(packages), "locked dependencies on", args.target)


if __name__ == "__main__":
    main()
