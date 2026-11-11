"""Offline tests for reviewed release-tag preparation."""
import contextlib
import io
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

import prepare_release as release


class PrepareReleaseTests(unittest.TestCase):
    commit = "a" * 40

    def call(self, request, version="1.2.3", expected=None, actual=None,
             package="1.2.3", manifest="1.2.3", lock="1.2.3"):
        return release.prepare(version, expected or self.commit, actual or self.commit,
                               package, manifest, lock, request)

    def test_missing_tag_is_created_and_both_verification_builds_are_requested(self):
        calls = []
        def request(method, path, data):
            calls.append((method, path, data))
            if method == "GET":
                return None
            if path == "git/refs":
                return {"object": {"type": "commit", "sha": self.commit}}
            return None
        self.assertEqual(self.call(request), "v1.2.3")
        self.assertEqual(calls, [
            ("GET", "git/ref/tags/v1.2.3", None),
            ("POST", "git/refs", {"ref": "refs/tags/v1.2.3", "sha": self.commit}),
            ("POST", "actions/workflows/release.yml/dispatches", {"ref": "v1.2.3"}),
            ("POST", "actions/workflows/microsoft-store.yml/dispatches", {"ref": "v1.2.3"}),
        ])

    def test_existing_correct_tag_is_not_recreated(self):
        calls = []
        def request(method, path, data):
            calls.append((method, path, data))
            return {"object": {"type": "commit", "sha": self.commit}} if method == "GET" else None
        self.call(request)
        self.assertFalse(any(path == "git/refs" for _, path, _ in calls))
        self.assertEqual(len(calls), 3)

    def test_wrong_or_annotated_tag_never_dispatches(self):
        for obj in ({"type": "commit", "sha": "b" * 40},
                    {"type": "tag", "sha": self.commit}, {}, None):
            calls = []
            def request(method, path, data):
                calls.append((method, path))
                return {"object": obj} if obj is not None else {"object": {}}
            with self.assertRaises(RuntimeError):
                self.call(request)
            self.assertEqual(calls, [("GET", "git/ref/tags/v1.2.3")])

    def test_invalid_request_fails_before_network(self):
        def forbidden(*_):
            self.fail("network should not be called")
        for kwargs in ({"version": "1.2.3; echo unsafe"}, {"version": "01.2.3"},
                       {"expected": "bad"}, {"actual": "b" * 40},
                       {"package": "1.2.4"}, {"manifest": "1.2.4"}, {"lock": "1.2.4"}):
            with self.subTest(kwargs=kwargs), self.assertRaises(RuntimeError):
                self.call(forbidden, **kwargs)

    def test_dispatch_accepts_empty_or_json_success_responses(self):
        class Response:
            def __init__(self, data):
                self.data = data
            def __enter__(self):
                return self
            def __exit__(self, *_):
                return False
            def read(self):
                return self.data
        for payload, expected in ((b"", None), (b'{"workflow_run_id":7}', {"workflow_run_id": 7})):
            with self.subTest(payload=payload), patch.object(release, "urlopen", return_value=Response(payload)) as network:
                actual = release.github_request("POST", "actions/workflows/release.yml/dispatches",
                                                {"ref": "v1.2.3"}, "test-token")
                self.assertEqual(actual, expected)
                request = network.call_args.args[0]
                self.assertEqual(request.method, "POST")
                self.assertEqual(request.full_url,
                                 "https://api.github.com/repos/yonathanarbel/LawPDF/actions/workflows/release.yml/dispatches")
                self.assertIn(b'"ref": "v1.2.3"', request.data)
                self.assertEqual(request.get_header("X-github-api-version"), "2026-03-10")
                self.assertEqual(network.call_args.kwargs["timeout"], 20)

    def test_missing_tag_is_the_only_http_error_treated_as_absence(self):
        def error(status):
            return HTTPError("https://api.github.com", status, "error", {}, io.BytesIO())
        with patch.object(release, "urlopen", side_effect=error(404)):
            self.assertIsNone(release.github_request("GET", "git/ref/tags/v1.2.3", None, "token"))
        for method, status in (("GET", 403), ("GET", 500), ("POST", 404)):
            with self.subTest(method=method, status=status), patch.object(release, "urlopen", side_effect=error(status)):
                with self.assertRaises(HTTPError):
                    release.github_request(method, "git/refs", {}, "token")

    def test_main_checks_repository_ref_and_actual_version_files_before_dispatch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text('[package]\nname="lawpdf"\nversion="1.2.3"\n')
            (root / "Cargo.lock").write_text('[[package]]\nname="lawpdf"\nversion="1.2.3"\n')
            (root / "release-manifest.json").write_text('{"product":{"version":"1.2.3"}}')
            environment = {"GITHUB_REPOSITORY": release.REPOSITORY, "GITHUB_REF": "refs/heads/main",
                           "GITHUB_SHA": self.commit, "GH_TOKEN": "test-token"}
            arguments = ["prepare_release.py", "--version", "1.2.3", "--expected-commit", self.commit]
            calls = []
            def request(method, path, data, token):
                calls.append((method, path, data, token))
                return {"object": {"type": "commit", "sha": self.commit}} if method == "GET" else None
            with patch.object(release, "ROOT", root), patch.object(sys, "argv", arguments), \
                 patch.dict(os.environ, environment, clear=True), patch.object(release, "github_request", side_effect=request), \
                 contextlib.redirect_stdout(io.StringIO()):
                release.main()
            self.assertEqual(len(calls), 3)
            self.assertTrue(all(call[3] == "test-token" for call in calls))
            (root / "Cargo.lock").write_text('[[package]]\nname="lawpdf"\nversion="1.2.4"\n')
            with patch.object(release, "ROOT", root), patch.object(sys, "argv", arguments), \
                 patch.dict(os.environ, environment, clear=True), patch.object(release, "github_request") as network:
                with self.assertRaises(RuntimeError):
                    release.main()
                network.assert_not_called()
            environment["GITHUB_REF"] = "refs/heads/other"
            with patch.object(release, "ROOT", root), patch.object(sys, "argv", arguments), \
                 patch.dict(os.environ, environment, clear=True), patch.object(release, "github_request") as network:
                with self.assertRaises(RuntimeError):
                    release.main()
                network.assert_not_called()

    def test_unexpected_created_tag_stops_before_dispatch(self):
        calls = []
        def request(method, path, data):
            calls.append((method, path))
            return None if method == "GET" else {"object": {"type": "commit", "sha": "b" * 40}}
        with self.assertRaises(RuntimeError):
            self.call(request)
        self.assertEqual(calls, [("GET", "git/ref/tags/v1.2.3"), ("POST", "git/refs")])


if __name__ == "__main__":
    unittest.main()
