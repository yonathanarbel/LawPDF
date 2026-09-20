"""Finalize only the already-built, maintainer-authorized LawPDF 0.2.32 release.

No private signing key is used here. The detached signature was produced on the
maintainer's Mac. Every package and its provenance must verify before publication.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REPO = "yonathanarbel/LawPDF"
TAG = "v0.2.32"
COMMIT = "1e4fe803453f3537c21d52349dd33c899f6949ca"
RELEASE_ID = 391881267
MANIFEST_HASH = "30ca503906c6fbb86c4a6308a5badd2795b514e5251cf39fa727dffc6d67668b"
PUBLIC_KEY = "dbf757b2f5ada6da73691bb4134b914d372f6526365d58ce4c36ec4b75363caf"
NAMES = {"LawPDFSetup-x64.exe", "LawPDF-windows-portable-x64.zip", "LawPDF-macos.zip"}
ROOT = Path(__file__).resolve().parent.parent

def command(*args, capture=False):
    return subprocess.run(args, check=True, text=True,
                          stdout=subprocess.PIPE if capture else None).stdout

def api(path):
    return json.loads(command("gh", "api", f"repos/{REPO}/{path}", capture=True))

def sha(path):
    with path.open("rb") as stream:
        digest = hashlib.sha256()
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

def require(condition, message):
    if not condition:
        raise RuntimeError(message)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--publish", action="store_true")
    args = parser.parse_args()
    release = api(f"releases/{RELEASE_ID}")
    require(release["tag_name"] == TAG, "Unexpected release tag")
    require(release["draft"], "Refusing to alter an already published release")
    require(api(f"git/ref/tags/{TAG}")["object"]["sha"] == COMMIT, "Tag moved")
    for run_id in (35413575923, 35413346645, 35413346644):
        run = api(f"actions/runs/{run_id}")
        require(run["head_sha"] == COMMIT and run["conclusion"] == "success",
                f"Required build evidence is not successful: {run_id}")
    notes = (ROOT / "docs/releases/v0.2.32.md").read_text(encoding="utf-8")
    require("Professor Yonathan Arbel" in notes and "WITHOUT WARRANTY" in notes,
            "Release notes are missing authorship or warranty notice")
    work = Path(tempfile.mkdtemp(prefix="lawpdf-publication-", dir=os.getenv("RUNNER_TEMP")))
    print(f"Verifying release in {work}", flush=True)
    command("gh", "release", "download", TAG, "--repo", REPO, "--dir", str(work),
            "--pattern", "LawPDF*", "--pattern", "UPDATE-MANIFEST.json",
            "--pattern", "SHA256SUMS.txt")
    manifest_path = work / "UPDATE-MANIFEST.json"
    require(sha(manifest_path) == MANIFEST_HASH, "Signed manifest bytes changed")
    manifest = json.loads(manifest_path.read_bytes())
    require(manifest["repository"] == REPO and manifest["version"] == "0.2.32"
            and manifest["commit"] == COMMIT and manifest["schema"] == 1
            and manifest["purpose"] == "lawpdf-update-v1", "Wrong manifest identity")
    require(len(manifest["assets"]) == 3
            and {a["name"] for a in manifest["assets"]} == NAMES, "Wrong packages")
    for item in manifest["assets"]:
        package = work / item["name"]
        require(package.stat().st_size == item["bytes"]
                and sha(package) == item["sha256"], f"Package mismatch: {item['name']}")
        command("gh", "attestation", "verify", str(package), "--repo", REPO,
                "--signer-workflow", f"{REPO}/.github/workflows/release.yml",
                "--source-ref", f"refs/tags/{TAG}", "--source-digest", COMMIT)
    # Verify the existing checksum file too; never replace the built packages.
    subprocess.run(["sha256sum", "--check", "SHA256SUMS.txt"], cwd=work, check=True)
    key_path = work / "key.der"
    key_path.write_bytes(bytes.fromhex("302a300506032b6570032100" + PUBLIC_KEY))
    signature_path = work / "UPDATE-MANIFEST.sig"
    shutil.copyfile(ROOT / "packaging/releases/v0.2.32.sig", signature_path)
    signature = bytes.fromhex(signature_path.read_text().strip())
    require(len(signature) == 64, "Invalid detached signature")
    signature_binary = work / "signature.bin"
    signature_binary.write_bytes(signature)
    command("openssl", "pkeyutl", "-verify", "-pubin", "-keyform", "DER",
            "-inkey", str(key_path), "-rawin", "-in", str(manifest_path),
            "-sigfile", str(signature_binary))
    print("All package, provenance, checksum and signature checks passed.", flush=True)
    if not args.publish:
        return
    # Be idempotent if a prior run uploaded the signature but did not publish.
    release = api(f"releases/{RELEASE_ID}")
    require(release["draft"], "Release changed while verifying")
    existing = [a for a in release["assets"] if a["name"] == signature_path.name]
    if existing:
        require(len(existing) == 1 and existing[0].get("digest") == "sha256:" + sha(signature_path),
                "Existing release signature does not match")
    else:
        command("gh", "release", "upload", TAG, str(signature_path), "--repo", REPO)
    command("gh", "release", "edit", TAG, "--repo", REPO, "--verify-tag",
            "--title", "LawPDF 0.2.32 — Public beta for Windows and Mac",
            "--notes-file", str(ROOT / "docs/releases/v0.2.32.md"),
            "--draft=false", "--prerelease=false", "--latest")
    published = api(f"releases/{RELEASE_ID}")
    require(not published["draft"] and published["published_at"], "Publication was not confirmed")
    require(api("releases/latest")["id"] == RELEASE_ID, "Latest-release pointer is wrong")
    require("UPDATE-MANIFEST.sig" in {a["name"] for a in published["assets"]},
            "Public signature is missing")
    print(f"Published and verified: {published['html_url']}", flush=True)

if __name__ == "__main__":
    main()
