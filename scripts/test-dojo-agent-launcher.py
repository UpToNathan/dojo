import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="dojo-agent-launcher-")
        self.addCleanup(self.temporary.cleanup)
        self.fixture = Path(self.temporary.name)
        self.root = self.fixture / "repo"
        scripts = self.root / "scripts"
        scripts.mkdir(parents=True)
        self.launcher = scripts / "dojo-agent"
        shutil.copy2(Path(__file__).with_name("dojo-agent"), self.launcher)
        debug = self.root / "target/debug/dojo-agent"
        debug.parent.mkdir(parents=True)
        debug.write_text('#!/bin/sh\nprintf "debug\\n"\nprintf "%s\\n" "$@"\n')
        debug.chmod(0o755)
        self.home = self.fixture / "home"
        (self.home / ".local/bin").mkdir(parents=True)
        self.link = self.home / ".local/bin/dojo-agent"
        self.link.symlink_to(self.launcher)
        self.environment = {**os.environ, "HOME": str(self.home), "XDG_DATA_HOME": str(self.fixture / "data")}
        self.environment.pop("DOJO_SEMANTIC_AGENT_EXE", None)

    def run_launcher(self):
        return subprocess.run([str(self.link), "--name", "space value"], env=self.environment,
                              capture_output=True, text=True)

    def test_symlinked_launcher_falls_back_only_without_a_pin(self):
        result = self.run_launcher()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["debug", "--name", "space value"])

    def test_invalid_or_empty_pin_never_falls_back_to_debug(self):
        for pin in ["", str(self.root / "target/debug/dojo-agent"), "/nix/store/missing-dojo-agent"]:
            self.environment["DOJO_SEMANTIC_AGENT_EXE"] = pin
            result = self.run_launcher()
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")
            self.assertIn("must pin", result.stderr)

    def test_current_installation_pin_overrides_inherited_pin_and_forwards_arguments(self):
        pin = os.environ.get("DOJO_TEST_PINNED_AGENT")
        if not pin:
            self.skipTest("Build scripts/test-fixtures/agent-launcher.nix and set DOJO_TEST_PINNED_AGENT")
        environment = self.fixture / "data/dojo/niri/current/environment.sh"
        environment.parent.mkdir(parents=True)
        environment.write_text("export DOJO_SEMANTIC_AGENT_EXE=" + shlex.quote(pin)
                               + "\nexport DOJO_LAUNCHER_FIXTURE=from-current\n")
        self.environment["DOJO_SEMANTIC_AGENT_EXE"] = "/invalid/inherited-pin"
        result = self.run_launcher()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), ["pinned", "from-current", "--name", "space value"])


if __name__ == "__main__":
    unittest.main()
