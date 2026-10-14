#!/usr/bin/env python3
import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("capture_packages", Path(__file__).with_name("capture-system-packages.py"))
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)

RESPONSES = {
    ("pacman", "-Q"): "zlib 1.3.1-2\ncustom-cli 2:4.0-1\nbase 3-2\n",
    ("pacman", "-Qqe"): "custom-cli\nbase\n",
    ("pacman", "-Qqd"): "zlib\n",
    ("pacman", "-Qqm"): "custom-cli\n",
    ("systemctl", "list-unit-files", "--state=enabled", "--no-pager", "--no-legend"): "nix-daemon.socket enabled disabled\nNetworkManager.service enabled enabled\n",
    ("systemctl", "--user", "list-unit-files", "--state=enabled", "--no-pager", "--no-legend"): "pipewire.socket enabled enabled\nwireplumber.service enabled enabled\n",
}
EXPECTED = {
    "system-config/packages/installed.tsv": "base\t3-2\ncustom-cli\t2:4.0-1\nzlib\t1.3.1-2\n",
    "system-config/packages/official-explicit.txt": "base\n",
    "system-config/packages/dependencies.txt": "zlib\n",
    "system-config/packages/foreign.tsv": "custom-cli\t2:4.0-1\n",
    "system-config/packages/foreign-explicit.txt": "custom-cli\n",
    "system-config/services/system-enabled.txt": "NetworkManager.service\nnix-daemon.socket\n",
    "system-config/services/user-enabled.txt": "pipewire.socket\nwireplumber.service\n",
}


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name)
        self.output = io.StringIO()

    def command(self, arguments, responses=None):
        stdout = (responses or RESPONSES)[tuple(arguments)]
        if isinstance(stdout, Exception):
            raise stdout
        if isinstance(stdout, subprocess.CompletedProcess):
            return stdout
        return subprocess.CompletedProcess(arguments, 0, stdout, "")

    def run_capture(self, *options, responses=None):
        with patch.object(capture.subprocess, "run", side_effect=lambda arguments, **kwargs: self.command(arguments, responses)), contextlib.redirect_stdout(self.output), contextlib.redirect_stderr(self.output):
            return capture.main(["--repo", str(self.repo), *options])

    def test_captures_desired_explicit_packages_and_observed_inventory(self):
        self.assertEqual(self.run_capture(), 0)
        for path, expected in EXPECTED.items():
            self.assertEqual((self.repo / path).read_text(), expected)

    def test_check_reports_missing_baseline_without_writes(self):
        self.assertEqual(self.run_capture("--check"), 1)
        self.assertEqual(list(self.repo.iterdir()), [])
        self.assertIn("Drift: system-config/packages/installed.tsv", self.output.getvalue())

    def test_second_capture_preserves_file_identity(self):
        self.assertEqual(self.run_capture(), 0)
        path = self.repo / "system-config/packages/installed.tsv"
        inode = path.stat().st_ino
        self.assertEqual(self.run_capture(), 0)
        self.assertEqual(path.stat().st_ino, inode)
        self.assertEqual(self.run_capture("--check"), 0)

    def test_check_detects_version_and_service_drift(self):
        self.assertEqual(self.run_capture(), 0)
        changed = dict(RESPONSES)
        changed[("pacman", "-Q")] = "zlib 1.3.2-1\ncustom-cli 2:4.0-1\nbase 3-2\n"
        changed[("systemctl", "list-unit-files", "--state=enabled", "--no-pager", "--no-legend")] = "NetworkManager.service enabled enabled\n"
        self.assertEqual(self.run_capture("--check", responses=changed), 1)
        self.assertEqual((self.repo / "system-config/packages/installed.tsv").read_text(), EXPECTED["system-config/packages/installed.tsv"])
        self.assertIn("Drift: system-config/services/system-enabled.txt", self.output.getvalue())

    def test_last_query_failure_writes_nothing(self):
        changed = dict(RESPONSES)
        command = ("systemctl", "--user", "list-unit-files", "--state=enabled", "--no-pager", "--no-legend")
        changed[command] = subprocess.CalledProcessError(1, list(command))
        self.assertEqual(self.run_capture(responses=changed), 2)
        self.assertEqual(list(self.repo.iterdir()), [])

    def test_inconsistent_database_queries_write_nothing(self):
        changed = dict(RESPONSES)
        changed[("pacman", "-Qqd")] = "zlib\nbase\n"
        self.assertEqual(self.run_capture(responses=changed), 2)
        self.assertEqual(list(self.repo.iterdir()), [])

    def test_empty_foreign_list_is_valid(self):
        changed = dict(RESPONSES)
        changed[("pacman", "-Qqm")] = subprocess.CompletedProcess(["pacman", "-Qqm"], 1, "", "")
        self.assertEqual(self.run_capture(responses=changed), 0)
        self.assertEqual((self.repo / "system-config/packages/foreign.tsv").read_text(), "")
        self.assertEqual((self.repo / "system-config/packages/official-explicit.txt").read_text(), "base\ncustom-cli\n")

    def test_bad_unit_record_writes_nothing(self):
        changed = dict(RESPONSES)
        changed[("systemctl", "list-unit-files", "--state=enabled", "--no-pager", "--no-legend")] = "NetworkManager.service disabled enabled\n"
        self.assertEqual(self.run_capture(responses=changed), 2)
        self.assertEqual(list(self.repo.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
