#!/usr/bin/env python3
import contextlib
import datetime as dt
import importlib.machinery
import importlib.util
import io
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import patch


loader = importlib.machinery.SourceFileLoader("dojo_update", str(Path(__file__).with_name("dojo-update")))
spec = importlib.util.spec_from_loader(loader.name, loader)
updater = importlib.util.module_from_spec(spec)
loader.exec_module(updater)


class UpdateTests(unittest.TestCase):
    def run_cli(self, args, result=0):
        output = io.StringIO()
        with contextlib.redirect_stdout(output), patch.object(updater.shutil, "which", return_value="/bin/tool"), \
             patch.object(updater.subprocess, "run", return_value=subprocess.CompletedProcess([], result)) as run:
            code = updater.main(args)
        return code, output.getvalue(), run

    def test_dry_run_has_no_side_effects_and_prints_age_policy(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"XDG_STATE_HOME": temporary}):
            with patch.object(updater.Path, "exists", return_value=True):
                code, output, run = self.run_cli(["--dry-run", "--only", "nix,codex,claude,t3code,nvim"])
            self.assertEqual(code, 0)
            self.assertIn("--buffer-days 3", output)
            self.assertIn("codex update", output)
            self.assertIn("claude update", output)
            self.assertIn("install-t3code.py", output)
            self.assertIn("nvim: held", output)
            self.assertIn("nix-home: planned", output)
            self.assertEqual(list(Path(temporary).iterdir()), [])
            run.assert_not_called()

    def test_input_failure_blocks_activation_and_continues_independent_tool(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"XDG_STATE_HOME": temporary}):
            code, output, run = self.run_cli(["--only", "nix,codex"], result=1)
            self.assertEqual(code, 1)
            self.assertIn("nix-inputs: failed", output)
            self.assertIn("nix-lock: blocked", output)
            self.assertIn("nix-home: blocked", output)
            self.assertIn("codex: failed", output)
            self.assertEqual([call.args[0][1:] for call in run.call_args_list], [
                [str(Path(updater.__file__).resolve().parent / "refresh-update-inputs.py"), "--repo",
                 str(Path(updater.__file__).resolve().parent.parent), "--buffer-days", "3"], ["update"]])

    def test_successful_nix_activation_uses_local_machine(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"XDG_STATE_HOME": temporary}):
            code, output, run = self.run_cli(["--only", "nix", "--keep-pins"])
            self.assertEqual(code, 0)
            self.assertIn("nix-home: ok", output)
            self.assertEqual([call.args[0] for call in run.call_args_list], [
                ["devenv", "update"], ["devenv", "machines", "deploy", socket.gethostname()]])

    def test_optional_groups_and_zero_buffer(self):
        code, output, _ = self.run_cli(["--dry-run", "--only", "nvim,dojo", "--buffer-days", "0"])
        self.assertEqual(code, 0)
        self.assertIn("'+Lazy! update'", output)
        self.assertIn("cargo build --release --bin dojo", output)
        self.assertIn("dojo-install: planned", output)

    def test_snapshot_preserves_options_replaces_all_mirrors(self):
        actual = updater.archive_config("[options]\nSigLevel = Required DatabaseOptional\n[core]\nInclude = /etc/pacman.d/mirrorlist\nCacheServer = https://cache/\n[extra]\nServer = https://latest/\n", "2026/10/09")
        self.assertEqual(actual, "[options]\nSigLevel = Required DatabaseOptional\n[core]\nServer = https://archive.archlinux.org/repos/2026/10/09/$repo/os/$arch\n[extra]\nServer = https://archive.archlinux.org/repos/2026/10/09/$repo/os/$arch\n")

    def test_unknown_repository_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "no supported dated snapshot"):
            updater.archive_config("[options]\n[custom]\nServer = https://latest/\n", "2026/10/09")

    def test_arch_snapshot_at_least_three_full_days_old(self):
        runner = updater.Runner(Path.cwd(), True)
        with patch.object(updater.shutil, "which", return_value="/bin/tool"), \
             patch.object(Path, "read_text", return_value="[options]\n[core]\nInclude = /mirrorlist\n"), \
             contextlib.redirect_stdout(io.StringIO()) as output:
            updater.update_arch(runner, 3, dt.datetime(2026, 10, 13, 12, tzinfo=dt.timezone.utc), "archive")
        self.assertIn("<snapshot-2026/10/09>", output.getvalue())
        self.assertEqual(runner.results, [("arch", "planned", "")])

    def test_omarchy_stable_replaces_every_official_mirror(self):
        actual = updater.omarchy_stable_config("[options]\nSigLevel = Required DatabaseOptional\n[core]\nInclude = /etc/pacman.d/mirrorlist\n[extra]\nServer = https://latest/\n")
        self.assertEqual(actual, "[options]\nSigLevel = Required DatabaseOptional\n[core]\nServer = https://stable-mirror.omarchy.org/$repo/os/$arch\n[extra]\nServer = https://stable-mirror.omarchy.org/$repo/os/$arch\n")
        with self.assertRaisesRegex(ValueError, "no supported dated snapshot"):
            updater.omarchy_stable_config("[options]\n[custom]\nServer = https://latest/\n")

    def test_arch_defaults_to_archive_and_omarchy_stable_is_opt_in(self):
        with patch.object(Path, "read_text", return_value="[options]\n[core]\nInclude = /mirrorlist\n"):
            _, output, _ = self.run_cli(["--dry-run", "--only", "arch"])
            self.assertIn("<snapshot-", output)
            _, output, _ = self.run_cli(["--dry-run", "--only", "arch", "--arch-source", "omarchy-stable"])
            self.assertIn("<omarchy-stable>", output)

    def test_npm_skips_hosts_without_a_global_prefix_and_pins_the_prefix(self):
        with tempfile.TemporaryDirectory() as home, patch.dict(os.environ, {"HOME": home}, clear=False), \
             patch.dict(os.environ, {"NPM_CONFIG_PREFIX": ""}):
            _, output, run = self.run_cli(["--only", "npm"])
            self.assertIn("npm: skipped", output)
            run.assert_not_called()
            (Path(home) / ".local/npm").mkdir(parents=True)
            _, _, run = self.run_cli(["--only", "npm"])
            self.assertEqual(run.call_args.args[0][:3], ["env", f"NPM_CONFIG_PREFIX={home}/.local/npm", "npm"])

    def test_t3code_is_installed_by_the_repository_script(self):
        code, _, run = self.run_cli(["--only", "t3code"])
        self.assertEqual(code, 0)
        self.assertTrue(run.call_args.args[0][1].endswith("scripts/install-t3code.py"))

    def test_concurrent_run_rejected(self):
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, {"XDG_STATE_HOME": temporary}), \
             updater.update_lock(False), contextlib.redirect_stderr(io.StringIO()) as error:
            code, _, _ = self.run_cli(["--only", "codex"])
        self.assertEqual(code, 1)
        self.assertIn("another dojo-update is running", error.getvalue())


if __name__ == "__main__":
    unittest.main()
