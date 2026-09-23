#!/usr/bin/env python3
"""Export only the approved build, runtime, and release-QA source files."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    destination = args.destination.resolve()
    if destination == root or root in destination.parents:
        raise SystemExit("Export outside the Box checkout.")
    if destination.exists() and any(destination.iterdir()):
        raise SystemExit("The destination must be empty; no files were replaced.")
    files = set(map(Path, [
        ".gitattributes", ".gitignore", "AGENTS.md", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.rs",
        "LICENSE", "README.md", "THIRD_PARTY_NOTICES.md", "THIRD_PARTY_RUST_LICENSES.csv", "release-manifest.json",
        "docs/PRIVACY.md", "docs/CODE_SIGNING.md", "docs/RELEASING.md", "docs/PRODUCTION_PLAN.md", "docs/PRODUCTION_VALIDATION.md", "docs/ACCESSIBILITY_FOLLOWUP.md",
        "packaging/update-public-key.hex", "packaging/windows/LawPDF.iss",
        "android/README.md", "android/build.gradle", "android/gradle.properties", "android/settings.gradle",
        "android/build.ps1", "android/build.sh", "android/gradlew", "android/gradlew.bat",
        "android/gradle/wrapper/gradle-wrapper.jar", "android/gradle/wrapper/gradle-wrapper.properties",
        "android/app/build.gradle", "android/app/proguard-rules.pro",
        "tools/md_verify.py", "tools/test_md_verify.py", "tools/verify_release_manifest.py",
        "tools/make_update_manifest.py", "tools/package_rust_notices.py", "tools/export_public_source.py",
        "vendor/libpdfium.dylib", "vendor/pdfium.dll", "vendor/fonts/EBGaramond.ttf",
        "vendor/fonts/FrankRuhlLibre-Black.ttf", "docs/READING_LAYOUT.md",
        "assets/lawpdf.ico", "assets/lawpdf.png",
    ]))
    for pattern in ["src/**/*.rs", "android/app/src/**/*.java", "android/app/src/**/*.xml",
                    "tests/markdown_fixtures/*", "tests/fixtures/*", "third_party/**/*",
                    "scripts/package-*.sh", "scripts/package-*.ps1", "scripts/fetch-catboost-windows.ps1",
                    "scripts/update-signing.swift", "scripts/verify-windows-install.ps1", "scripts/verify-android-persistence.py",
                    ".github/workflows/*.yml"]:
        files.update(path.relative_to(root) for path in root.glob(pattern) if path.is_file())
    # Only exact file exceptions in the reviewed promotion allowlist qualify.
    # Being tracked by Git is not authorization to publish research artifacts.
    for rule in (root / ".gitignore").read_text(encoding="utf-8").splitlines():
        if rule.startswith("!/profile-models/") and not rule.endswith("/"):
            if any(character in rule for character in "*?["):
                raise SystemExit("Model publication requires an exact file allowlist: " + rule)
            files.add(Path(rule[2:]))
    manifest = json.loads((root / "release-manifest.json").read_text(encoding="utf-8"))
    for asset in manifest["runtime_assets"].values():
        if isinstance(asset, dict) and isinstance(asset.get("path"), str):
            files.add(Path(asset["path"]))
    files.discard(Path("src/profile_dataset.rs"))
    records = []
    for relative in sorted(files):
        if relative.is_absolute() or ".." in relative.parts:
            raise SystemExit("Unsafe source path: " + str(relative))
        source = root / relative
        if source.is_symlink() or not source.is_file():
            raise SystemExit("Missing or linked source file: " + str(relative))
        output = destination / relative
        output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, output)
        digest = hashlib.sha256(output.read_bytes()).hexdigest()
        records.append({"path": relative.as_posix(), "sha256": digest, "bytes": output.stat().st_size})
    (destination / "PUBLIC-SOURCE-MANIFEST.json").write_text(json.dumps(records, indent=2) + "\n", encoding="utf-8")
    print("Exported", len(records), "approved source/runtime files to", destination)


if __name__ == "__main__":
    main()
