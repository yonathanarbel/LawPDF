"""Verify or publish a built LawPDF release authorized by a maintainer record.

No private signing key is used here. The detached signature was produced on the
maintainer's Mac. Every package and its provenance must verify before publication.
"""
import argparse
import base64
import re
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile

REPO = "yonathanarbel/LawPDF"
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
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    require(re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", args.version), "Invalid version")
    version = args.version
    tag = f"v{version}"
    record = json.loads((ROOT / f"packaging/releases/{tag}.json").read_bytes())
    require(record["version"] == version, "Wrong maintainer record version")
    commit = record["commit"]
    release_id = record["release_id"]
    manifest_hash = record["manifest_sha256"]
    require(re.fullmatch(r"[a-f0-9]{40}", commit), "Invalid source commit")
    require(re.fullmatch(r"[a-f0-9]{64}", manifest_hash), "Invalid manifest digest")
    release = api(f"releases/{release_id}")
    require(release["tag_name"] == tag, "Unexpected release tag")
    require(release["draft"], "Refusing to alter an already published release")
    require(api(f"git/ref/tags/{tag}")["object"]["sha"] == commit, "Tag moved")
    require(len(set(record["required_runs"])) >= 2, "Missing release and desktop build evidence")
    verified_workflows = set()
    release_run_id = None
    for run_id in record["required_runs"]:
        run = api(f"actions/runs/{run_id}")
        require(run["head_sha"] == commit and run["conclusion"] == "success",
                f"Required build evidence is not successful: {run_id}")
        verified_workflows.add(run["name"])
        if run["name"] == "Release":
            release_run_id = run_id
    require({"Release", "Desktop verification"} <= verified_workflows, "Missing required workflow results")
    notes = (ROOT / f"docs/releases/{tag}.md").read_text(encoding="utf-8")
    require("Professor Yonathan Arbel" in notes and "WITHOUT WARRANTY" in notes,
            "Release notes are missing authorship or warranty notice")
    require("SOURCE_AND_BUILD_EVIDENCE" not in notes, "Release notes still contain a placeholder")
    work = Path(tempfile.mkdtemp(prefix="lawpdf-publication-", dir=os.getenv("RUNNER_TEMP")))
    print(f"Verifying release in {work}", flush=True)
    command("gh", "release", "download", tag, "--repo", REPO, "--dir", str(work),
            "--pattern", "LawPDF*", "--pattern", "UPDATE-MANIFEST.json",
            "--pattern", "SHA256SUMS.txt")
    manifest_path = work / "UPDATE-MANIFEST.json"
    require(sha(manifest_path) == manifest_hash, "Signed manifest bytes changed")
    manifest = json.loads(manifest_path.read_bytes())
    require(manifest["repository"] == REPO and manifest["version"] == version
            and manifest["commit"] == commit and manifest["schema"] == 1
            and manifest["purpose"] == "lawpdf-update-v1", "Wrong manifest identity")
    require(len(manifest["assets"]) == 3
            and {a["name"] for a in manifest["assets"]} == NAMES, "Wrong packages")
    for item in manifest["assets"]:
        package = work / item["name"]
        require(package.stat().st_size == item["bytes"]
                and sha(package) == item["sha256"], f"Package mismatch: {item['name']}")
        command("gh", "attestation", "verify", str(package), "--repo", REPO,
                "--signer-workflow", f"{REPO}/.github/workflows/release.yml",
                "--source-ref", f"refs/tags/{tag}", "--source-digest", commit)
    evidence_dir = work / "windows-install-evidence"
    command("gh", "run", "download", str(release_run_id), "--repo", REPO,
            "--name", "LawPDF-Windows-installation-evidence", "--dir", str(evidence_dir))
    installation = json.loads((evidence_dir / "installation.json").read_text(encoding="utf-8-sig"))
    require(installation["installer_sha256"] == sha(work / "LawPDFSetup-x64.exe"),
            "Installation evidence refers to another installer")
    for field in ("product_version", "file_version", "runtime_app_version"):
        require(installation[field] in (version, version + ".0"), "Installed version mismatch")
    require(installation["runtime_exit_code"] == 0 and installation["runtime_requirements_met"] is True
            and installation["start_menu_shortcuts"], "Installed runtime or shortcuts failed")
    with zipfile.ZipFile(work / "LawPDF-windows-portable-x64.zip") as archive:
        executables = [name for name in archive.namelist() if name.replace("\\", "/").split("/")[-1].lower() == "lawpdf.exe"]
        require(len(executables) == 1, "Portable package must contain one LawPDF executable")
        require(hashlib.sha256(archive.read(executables[0])).hexdigest() == installation["executable_sha256"],
                "Installed executable differs from the verified portable package")
    evidence_path = work / "RELEASE-VERIFICATION.json"
    evidence_path.write_text(json.dumps({"schema": "lawpdf-release-verification-v1",
        "version": version, "commit": commit, "required_runs": record["required_runs"],
        "verified_packages": manifest["assets"], "windows_installation": installation}, indent=2) + "\n")
    # Verify the existing checksum file too; never replace the built packages.
    subprocess.run(["sha256sum", "--check", "SHA256SUMS.txt"], cwd=work, check=True)
    if not args.publish:
        print("Package, provenance and checksum verification passed.", flush=True)
        print("VERIFIED_MANIFEST_BASE64=" + base64.b64encode(manifest_path.read_bytes()).decode("ascii"), flush=True)
        return
    key_path = work / "key.der"
    key_path.write_bytes(bytes.fromhex("302a300506032b6570032100" + PUBLIC_KEY))
    signature_path = work / "UPDATE-MANIFEST.sig"
    shutil.copyfile(ROOT / f"packaging/releases/{tag}.sig", signature_path)
    signature = bytes.fromhex(signature_path.read_text().strip())
    require(len(signature) == 64, "Invalid detached signature")
    signature_binary = work / "signature.bin"
    signature_binary.write_bytes(signature)
    command("openssl", "pkeyutl", "-verify", "-pubin", "-keyform", "DER",
            "-inkey", str(key_path), "-rawin", "-in", str(manifest_path),
            "-sigfile", str(signature_binary))
    print("All package, provenance, checksum and signature checks passed.", flush=True)
    # Be idempotent if a prior run uploaded the signature but did not publish.
    release = api(f"releases/{release_id}")
    require(release["draft"], "Release changed while verifying")
    for extra_asset in (signature_path, evidence_path):
        existing = [a for a in release["assets"] if a["name"] == extra_asset.name]
        if existing:
            require(len(existing) == 1 and existing[0].get("digest") == "sha256:" + sha(extra_asset),
                    "Existing release metadata does not match")
        else:
            command("gh", "release", "upload", tag, str(extra_asset), "--repo", REPO)
    command("gh", "release", "edit", tag, "--repo", REPO, "--verify-tag",
            "--title", f"LawPDF {version} — Public beta for Windows and Mac",
            "--notes-file", str(ROOT / f"docs/releases/{tag}.md"),
            "--draft=false", "--prerelease=false", "--latest")
    published = api(f"releases/{release_id}")
    require(not published["draft"] and published["published_at"], "Publication was not confirmed")
    require(api("releases/latest")["id"] == release_id, "Latest-release pointer is wrong")
    require("UPDATE-MANIFEST.sig" in {a["name"] for a in published["assets"]},
            "Public signature is missing")
    print(f"Published and verified: {published['html_url']}", flush=True)

if __name__ == "__main__":
    main()
