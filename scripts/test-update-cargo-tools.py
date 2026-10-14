#!/usr/bin/env python3
import contextlib
import datetime as dt
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("update_cargo_tools", Path(__file__).with_name("update-cargo-tools.py"))
updater = importlib.util.module_from_spec(spec)
spec.loader.exec_module(updater)


NOW = dt.datetime(2026, 10, 13, 12, tzinfo=dt.timezone.utc)


def response(versions, total=None, next_page=None):
    payload = {"versions": versions, "meta": {"total": len(versions) if total is None else total, "next_page": next_page}}
    return contextlib.closing(io.BytesIO(json.dumps(payload).encode()))


class CargoUpdateTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name)
        self.output = io.StringIO()
        self.stderr = io.StringIO()

    def write_installs(self, entries, metadata=None):
        legacy = "[v1]\n" + "".join(
            f'"{name} {version} ({source})" = {json.dumps(bins)}\n'
            for name, version, source, bins in entries
        )
        (self.home / ".crates.toml").write_text(legacy, encoding="utf-8")
        installs = {}
        for name, version, source, bins in entries:
            installs[f"{name} {version} ({source})"] = {
                "features": [], "all_features": False, "no_default_features": False,
                "bins": bins, "profile": "release", "target": "x86_64-unknown-linux-gnu",
                "rustc": "rustc\nhost: x86_64-unknown-linux-gnu\n",
            }
            installs[f"{name} {version} ({source})"].update((metadata or {}).get(name, {}))
        (self.home / ".crates2.json").write_text(json.dumps({"installs": installs}), encoding="utf-8")

    def run_update(self, buffer=3, dry_run=False, api=None):
        with patch.object(updater, "urlopen", side_effect=api), \
             contextlib.redirect_stdout(self.output), contextlib.redirect_stderr(self.stderr):
            return updater.update(self.home, buffer, dry_run, NOW)

    def test_selects_highest_old_enough_stable_non_yanked_and_preserves_features(self):
        self.write_installs([
            ("tool", "1.0.0", updater.CRATES_IO, ["tool"]),
        ], {"tool": {"features": ["fast", "json"], "no_default_features": True}})
        versions = [
            {"num": "1.3.0", "created_at": "2026-10-09T11:59:59Z", "yanked": False},
            {"num": "2.0.0", "created_at": "2026-10-10T12:00:01Z", "yanked": False},
            {"num": "9.0.0-alpha.1", "created_at": "2026-10-01T00:00:00Z", "yanked": False},
            {"num": "1.9.0", "created_at": "2026-10-01T00:00:00Z", "yanked": True},
        ]
        with patch.object(updater.subprocess, "run") as run:
            self.assertEqual(self.run_update(api=lambda request, timeout: response(versions)), 0)
        run.assert_called_once_with([
            "cargo", "install", "tool", "--version", "=1.3.0", "--locked",
            "--features", "fast,json", "--no-default-features", "--bin", "tool",
        ], check=True)

    def test_dry_run_does_not_install_and_keeps_installed_newer(self):
        self.write_installs([
            ("newer", "3.0.0", updater.CRATES_IO, ["newer"]),
            ("upgrade", "1.0.0", updater.CRATES_IO, ["upgrade"]),
        ])
        versions = [
            {"num": "2.0.0", "created_at": "2026-10-01T00:00:00Z", "yanked": False},
        ]
        original_files = {path.name: path.read_bytes() for path in self.home.iterdir()}
        with patch.object(updater.subprocess, "run") as run:
            self.assertEqual(self.run_update(dry_run=True, api=lambda request, timeout: response(versions)), 0)
        run.assert_not_called()
        self.assertEqual({path.name: path.read_bytes() for path in self.home.iterdir()}, original_files)
        self.assertIn("newer: current (3.0.0)", self.output.getvalue())
        self.assertIn("upgrade: planned", self.output.getvalue())

    def test_unknown_source_is_manual_and_not_queried(self):
        self.write_installs([("tool", "1.0.0", "git+https://example.test/tool", ["tool"])])
        with patch.object(updater, "urlopen", side_effect=AssertionError("must not query")):
            self.assertEqual(self.run_update(), 0)
        self.assertIn("manual: tool 1.0.0 comes from git+", self.output.getvalue())

    def test_install_failure_is_reported_and_other_crates_continue(self):
        self.write_installs([
            ("bad", "1.0.0", updater.CRATES_IO, ["bad"]),
            ("good", "1.0.0", updater.CRATES_IO, ["good"]),
        ])
        versions = [{"num": "2.0.0", "created_at": "2026-10-01T00:00:00Z", "yanked": False}]
        results = [subprocess.CalledProcessError(1, "cargo"), subprocess.CompletedProcess([], 0)]
        with patch.object(updater.subprocess, "run", side_effect=results) as run:
            self.assertEqual(self.run_update(api=lambda request, timeout: response(versions)), 1)
        self.assertEqual(run.call_count, 2)
        self.assertIn("bad: failed", self.stderr.getvalue())
        self.assertIn("good: updated to 2.0.0", self.output.getvalue())

    def test_unsafe_feature_metadata_is_held(self):
        self.write_installs([("tool", "1.0.0", updater.CRATES_IO, ["tool"])], {
            "tool": {"features": [3]},
        })
        with patch.object(updater, "urlopen", side_effect=AssertionError("must not query")):
            self.assertEqual(self.run_update(), 0)
        self.assertIn("manual: tool 1.0.0: feature metadata is malformed", self.output.getvalue())

    def test_publication_cutoff_and_zero_buffer_validation(self):
        self.write_installs([("tool", "1.0.0", updater.CRATES_IO, ["tool"])])
        versions = [
            {"num": "2.0.0", "created_at": "2026-10-10T12:00:01Z", "yanked": False},
        ]
        with patch.object(updater.subprocess, "run") as run:
            self.assertEqual(self.run_update(api=lambda request, timeout: response(versions)), 0)
        run.assert_not_called()
        with contextlib.redirect_stderr(self.stderr), self.assertRaises(SystemExit) as error:
            updater.main(["--buffer-days", "-1"])
        self.assertEqual(error.exception.code, 2)
        with contextlib.redirect_stderr(self.stderr), self.assertRaises(SystemExit) as error:
            updater.main(["--buffer-days", "36501"])
        self.assertEqual(error.exception.code, 2)

    def test_finds_highest_eligible_version_after_first_api_page(self):
        self.write_installs([("tool", "1.0.0", updater.CRATES_IO, ["tool"])])
        recent = [
            {"num": f"3.0.{index}", "created_at": "2026-10-12T00:00:00Z", "yanked": False}
            for index in range(100)
        ]
        older = [
            {"num": "2.2.0", "created_at": "2026-10-09T00:00:00Z", "yanked": False},
            {"num": "2.1.0", "created_at": "2026-10-08T00:00:00Z", "yanked": False},
        ]
        calls = []

        def api(request, timeout):
            calls.append(request.full_url)
            if len(calls) == 1:
                return response(recent, total=102, next_page="?per_page=100&seek=opaque-cursor")
            return response(older, total=102)

        with patch.object(updater.subprocess, "run") as run:
            self.assertEqual(self.run_update(api=api), 0)
        self.assertEqual(len(calls), 2)
        self.assertTrue(calls[0].endswith("?per_page=100"))
        self.assertTrue(calls[1].endswith("?per_page=100&seek=opaque-cursor"))
        run.assert_called_once_with([
            "cargo", "install", "tool", "--version", "=2.2.0", "--locked", "--bin", "tool",
        ], check=True)

    def test_incomplete_pagination_and_unknown_yanked_status_fail_closed(self):
        self.write_installs([("tool", "1.0.0", updater.CRATES_IO, ["tool"])])
        eligible = [{"num": "2.0.0", "created_at": "2026-10-01T00:00:00Z", "yanked": False}]
        with patch.object(updater.subprocess, "run") as run:
            self.assertEqual(self.run_update(api=lambda request, timeout: response(
                eligible, total=2
            )), 1)
        run.assert_not_called()
        self.assertIn("ended pagination before all versions were read", self.stderr.getvalue())

        self.output = io.StringIO()
        self.stderr = io.StringIO()
        with patch.object(updater.subprocess, "run") as run:
            self.assertEqual(self.run_update(api=lambda request, timeout: response([
                {"num": "2.0.0", "created_at": "2026-10-01T00:00:00Z"},
            ])), 1)
        run.assert_not_called()
        self.assertIn("invalid yanked metadata", self.stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
