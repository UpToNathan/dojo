#!/usr/bin/env python3
import contextlib
import importlib.machinery
import importlib.util
import io
from pathlib import Path
import socket
import tempfile
import unittest
from unittest.mock import patch


loader = importlib.machinery.SourceFileLoader("restore_baseline", str(Path(__file__).with_name("restore-system-baseline")))
spec = importlib.util.spec_from_loader(loader.name, loader)
restore = importlib.util.module_from_spec(spec)
loader.exec_module(restore)


class RestoreTests(unittest.TestCase):
    def fixture(self, repo):
        (repo / "system-config/packages").mkdir(parents=True)
        (repo / "system-config/services").mkdir(parents=True)
        (repo / "system-config/packages/official-explicit.txt").write_text("base\nbluez\nbase\n")
        (repo / "system-config/packages/required.txt").write_text("# setup dependencies\nfirewalld\n")
        (repo / "system-config/services/system-enabled.txt").write_text("bluetooth.service\n")

    def test_plan_includes_required_firewall_and_tracked_installer(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            self.fixture(repo)
            commands = restore.restore_plan(repo, "/snapshot.conf")
        self.assertEqual(commands, [
            ["sudo", "pacman", "--config", "/snapshot.conf", "-Syyu", "--needed", "base", "bluez", "firewalld"],
            ["bash", str(repo / "scripts/install-system-config.sh")],
            ["devenv", "machines", "deploy", socket.gethostname()],
            ["sudo", "systemctl", "enable", "bluetooth.service"],
        ])

    def test_preview_has_default_buffer_and_never_runs_commands(self):
        with contextlib.redirect_stdout(io.StringIO()) as output, \
             patch.object(restore.subprocess, "run") as run, \
             patch.object(restore.tempfile, "NamedTemporaryFile") as temporary:
            code = restore.main([])
        self.assertEqual(code, 0)
        self.assertIn("Package version buffer: 3 days", output.getvalue())
        self.assertIn("<snapshot-", output.getvalue())
        self.assertIn("firewalld", output.getvalue())
        run.assert_not_called()
        temporary.assert_not_called()

    def test_invalid_manifest_cannot_inject_command_flags(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            self.fixture(repo)
            (repo / "system-config/packages/required.txt").write_text("--noconfirm\n")
            with self.assertRaisesRegex(ValueError, "invalid entry"):
                restore.restore_plan(repo)


if __name__ == "__main__":
    unittest.main()
