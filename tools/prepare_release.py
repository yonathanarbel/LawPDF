"""Create an explicitly identified release tag and request verification builds.

This release utility only tags the supplied, already reviewed main-branch commit.
It never signs artifacts or publishes a release.
"""
import argparse
import json
import os
from pathlib import Path
import re
import tomllib
from urllib.error import HTTPError
from urllib.request import Request, urlopen

REPOSITORY = "yonathanarbel/LawPDF"
VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
COMMIT = re.compile(r"[0-9a-f]{40}")
ROOT = Path(__file__).resolve().parent.parent


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def github_request(method, path, data, token):
    body = None if data is None else json.dumps(data).encode("utf-8")
    request = Request(
        f"https://api.github.com/repos/{REPOSITORY}/{path}",
        data=body,
        method=method,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "User-Agent": "LawPDF-Release-Preparation",
            "X-GitHub-Api-Version": "2026-03-10",
        },
    )
    try:
        with urlopen(request, timeout=20) as response:
            payload = response.read()
            return json.loads(payload) if payload else None
    except HTTPError as error:
        if method == "GET" and error.code == 404:
            return None
        raise


def prepare(version, expected_commit, actual_commit, package_version, manifest_version, lock_version, request):
    require(VERSION.fullmatch(version) is not None, "Invalid release version")
    require(COMMIT.fullmatch(expected_commit) is not None, "Expected commit must be a full lowercase SHA-1")
    require(expected_commit == actual_commit, "The main-branch commit differs from the reviewed commit")
    require(version == package_version == manifest_version == lock_version,
            "Package, lockfile, and runtime manifest versions must match")
    tag = f"v{version}"
    existing = request("GET", f"git/ref/tags/{tag}", None)
    if existing is None:
        existing = request("POST", "git/refs", {"ref": f"refs/tags/{tag}", "sha": expected_commit})
    obj = existing.get("object", {}) if isinstance(existing, dict) else {}
    require(obj.get("type") == "commit" and obj.get("sha") == expected_commit,
            "Release tag is not a direct reference to the reviewed commit")
    for workflow in ("release.yml", "microsoft-store.yml"):
        request("POST", f"actions/workflows/{workflow}/dispatches", {"ref": tag})
    return tag


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--expected-commit", required=True)
    args = parser.parse_args()
    require(os.environ.get("GITHUB_REPOSITORY") == REPOSITORY, "Unexpected GitHub repository")
    require(os.environ.get("GITHUB_REF") == "refs/heads/main", "Preparation must run from main")
    token = os.environ.get("GH_TOKEN")
    require(bool(token), "Missing GitHub token")
    package = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))["package"]
    versions = [item["version"] for item in lock if item["name"] == package["name"]]
    require(len(versions) == 1, "Lockfile must contain exactly one application package")
    manifest = json.loads((ROOT / "release-manifest.json").read_text(encoding="utf-8"))["product"]["version"]
    tag = prepare(args.version, args.expected_commit, os.environ.get("GITHUB_SHA", ""),
                  package["version"], manifest, versions[0],
                  lambda method, path, data: github_request(method, path, data, token))
    print(f"Prepared {tag} at {args.expected_commit}; verification builds requested.")


if __name__ == "__main__":
    main()
