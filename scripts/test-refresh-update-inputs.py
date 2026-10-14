#!/usr/bin/env python3
import contextlib
import datetime
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError
from urllib.parse import parse_qs, urlparse


spec = importlib.util.spec_from_file_location("refresh_inputs", Path(__file__).with_name("refresh-update-inputs.py"))
updater = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updater)

OLD = "1" * 40
NEW = "2" * 40
NOW = datetime.datetime(2026, 10, 13, 16, 0, tzinfo=datetime.timezone.utc)
SOURCE = (
    "# keep this comment\r\ninputs:\r\n"
    f"  devenv:\r\n    url: github:cachix/devenv/{OLD}?dir=src/modules # keep query\r\n"
    f"  home-manager:\r\n    url: 'github:nix-community/home-manager/{OLD}' # quote\r\n"
    "    inputs:\r\n      nixpkgs:\r\n        follows: nixpkgs\r\n"
    "nixpkgs:\r\n  permitted_unfree_packages:\r\n    - 1password\r\n"
).encode()


class RefreshTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.source = self.repo / "devenv.yaml"
        self.source.write_bytes(SOURCE)
        self.source.chmod(0o640)
        self.state = self.root / "state"
        self.environment = patch.dict(os.environ, {"XDG_STATE_HOME": str(self.state)})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.output = io.StringIO()

    def api(self, request, timeout):
        result = [{"sha": NEW, "commit": {"committer": {"date": "2026-10-10T16:00:00Z"}}}]
        return contextlib.closing(io.BytesIO(json.dumps(result).encode()))

    def run_command(self, *options, api=None):
        with patch.object(updater, "urlopen", side_effect=api or self.api), patch.object(updater, "utc_now", return_value=NOW), contextlib.redirect_stdout(self.output), contextlib.redirect_stderr(self.output):
            return updater.main(["--repo", str(self.repo), *options])

    def test_updates_all_pins_preserves_format_and_backs_up_original(self):
        self.assertEqual(self.run_command(), 0)
        self.assertEqual(self.source.read_bytes(), SOURCE.replace(OLD.encode(), NEW.encode()))
        backups = list((self.state / "dojo-update").glob("devenv.yaml-*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), SOURCE)
        self.assertEqual(self.source.stat().st_mode & 0o777, 0o640)
        self.assertEqual(list(self.repo.iterdir()), [self.source])

    def test_dry_run_fetches_and_writes_nothing(self):
        self.assertEqual(self.run_command("--dry-run"), 0)
        self.assertEqual(self.source.read_bytes(), SOURCE)
        self.assertFalse(self.state.exists())
        self.assertIn(f"{OLD} -> {NEW}", self.output.getvalue())

    def test_api_failure_prevents_partial_update(self):
        def fail(request, timeout):
            if "nix-community" in request.full_url:
                raise HTTPError(request.full_url, 403, "rate limit", {}, None)
            return self.api(request, timeout)
        self.assertEqual(self.run_command(api=fail), 1)
        self.assertEqual(self.source.read_bytes(), SOURCE)
        self.assertEqual(list((self.state / "dojo-update").glob("devenv.yaml-*")), [])

    def test_invalid_api_revision_prevents_update(self):
        def invalid(request, timeout):
            result = [{"sha": "branch-name"}]
            return contextlib.closing(io.BytesIO(json.dumps(result).encode()))
        self.assertEqual(self.run_command(api=invalid), 1)
        self.assertEqual(self.source.read_bytes(), SOURCE)

    def test_concurrent_source_edit_is_preserved(self):
        edited = SOURCE + b"# edited while fetching\n"
        def edit(request, timeout):
            self.source.write_bytes(edited)
            return self.api(request, timeout)
        self.assertEqual(self.run_command(api=edit), 1)
        self.assertEqual(self.source.read_bytes(), edited)
        self.assertIn("refusing to overwrite", self.output.getvalue())

    def test_unchanged_pins_keep_file_identity_and_need_no_backup(self):
        self.source.write_bytes(SOURCE.replace(OLD.encode(), NEW.encode()))
        inode = self.source.stat().st_ino
        self.assertEqual(self.run_command(), 0)
        self.assertEqual(self.source.stat().st_ino, inode)
        self.assertEqual(list((self.state / "dojo-update").glob("devenv.yaml-*")), [])
        self.assertIn("All input revisions are unchanged", self.output.getvalue())

    def test_invalid_pin_is_rejected_before_fetching(self):
        self.source.write_bytes(SOURCE.replace(OLD.encode(), b"main"))
        with patch.object(updater, "urlopen", side_effect=AssertionError("must not fetch")):
            with contextlib.redirect_stderr(self.output):
                self.assertEqual(updater.main(["--repo", str(self.repo), "--dry-run"]), 1)
        self.assertIn("explicit 40-character", self.output.getvalue())

    def test_backup_directory_inside_repo_is_rejected(self):
        with patch.dict(os.environ, {"XDG_STATE_HOME": str(self.repo / "state")}):
            self.assertEqual(self.run_command(), 1)
        self.assertEqual(list(self.repo.iterdir()), [self.source])

    def test_default_branch_query_has_three_day_cutoff(self):
        endpoints = []
        def record(request, timeout):
            endpoints.append(request.full_url)
            return self.api(request, timeout)
        self.assertEqual(self.run_command("--dry-run", api=record), 0)
        self.assertEqual(urlparse(endpoints[0]).path, "/repos/cachix/devenv/commits")
        self.assertEqual(parse_qs(urlparse(endpoints[0]).query), {"per_page": ["1"], "until": ["2026-10-10T16:00:00Z"]})

    def test_custom_buffer_changes_cutoff(self):
        endpoints = []
        def record(request, timeout):
            endpoints.append(request.full_url)
            return self.api(request, timeout)
        self.assertEqual(self.run_command("--dry-run", "--buffer-days", "1", api=record), 0)
        self.assertEqual(parse_qs(urlparse(endpoints[0]).query)["until"], ["2026-10-12T16:00:00Z"])

    def test_exempt_inputs_receive_head_without_cutoff(self):
        self.source.write_bytes((f"inputs:\n  pi:\n    url: github:works/pi/{OLD}\n  opencode:\n    url: github:anomalyco/opencode/{OLD}\n").encode())
        endpoints = []
        def record(request, timeout):
            endpoints.append(request.full_url)
            return contextlib.closing(io.BytesIO(json.dumps([{"sha": NEW}]).encode()))
        self.assertEqual(self.run_command(api=record), 0)
        self.assertEqual(self.source.read_text(), f"inputs:\n  pi:\n    url: github:works/pi/{NEW}\n  opencode:\n    url: github:anomalyco/opencode/{NEW}\n")
        self.assertEqual([parse_qs(urlparse(endpoint).query) for endpoint in endpoints], [{"per_page": ["1"]}, {"per_page": ["1"]}])

    def test_empty_eligible_commits_prevents_update(self):
        self.assertEqual(self.run_command(api=lambda request, timeout: contextlib.closing(io.BytesIO(b"[]"))), 1)
        self.assertEqual(self.source.read_bytes(), SOURCE)

    def test_commit_newer_than_cutoff_prevents_update(self):
        def recent(request, timeout):
            result = [{"sha": NEW, "commit": {"committer": {"date": "2026-10-10T16:00:01Z"}}}]
            return contextlib.closing(io.BytesIO(json.dumps(result).encode()))
        self.assertEqual(self.run_command(api=recent), 1)
        self.assertEqual(self.source.read_bytes(), SOURCE)

    def test_missing_commit_date_prevents_buffered_update(self):
        result = json.dumps([{"sha": NEW}]).encode()
        self.assertEqual(self.run_command(api=lambda request, timeout: contextlib.closing(io.BytesIO(result))), 1)
        self.assertEqual(self.source.read_bytes(), SOURCE)

    def test_negative_buffer_is_rejected(self):
        with contextlib.redirect_stderr(self.output), self.assertRaises(SystemExit) as error:
            updater.main(["--repo", str(self.repo), "--buffer-days", "-1"])
        self.assertEqual(error.exception.code, 2)
        self.assertFalse(self.state.exists())

    def test_second_updater_refuses_active_lock(self):
        state = self.state / "dojo-update"
        state.mkdir(parents=True)
        with (state / "inputs.lock").open("a") as lock:
            updater.fcntl.flock(lock, updater.fcntl.LOCK_EX | updater.fcntl.LOCK_NB)
            self.assertEqual(self.run_command(), 1)
        self.assertEqual(self.source.read_bytes(), SOURCE)
        self.assertIn("Another input refresh is running", self.output.getvalue())


if __name__ == "__main__":
    unittest.main()
