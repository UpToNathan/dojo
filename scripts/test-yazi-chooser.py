#!/usr/bin/env python3
"""Run with python3 scripts/test-yazi-chooser.py; no GUI or deployment."""
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("chooser", Path(__file__).with_name("yazi-chooser.py"))
chooser = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chooser)


class ChooserTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.file = self.root / 'space $ and "quote".txt'
        self.file.touch()
        self.second = self.root / "second.txt"
        self.second.touch()
        self.folder = self.root / "a directory"
        self.folder.mkdir()
        self.out = self.root / "portal-out"
        self.out.write_text("stale result")
        self.fake = self.root / "kitty"
        self.fake.write_text('''#!/usr/bin/env python3
import os, pathlib, sys
assert sys.argv[3] == "/fake/yazi"
assert len(sys.argv) == 6
config = pathlib.Path(os.environ["YAZI_CONFIG_HOME"])
assert "quit --no-cwd-file" in (config / "keymap.toml").read_text()
assert (config / "plugins/dojo-chooser.yazi/main.lua").exists()
with open(os.environ["CALLS"], "a") as f: f.write("call\\n")
if os.environ.get("RESULT"):
    pathlib.Path(sys.argv[4].split("=", 1)[1]).write_text(os.environ["RESULT"])
sys.exit(int(os.environ.get("STATUS", "0")))
''')
        self.fake.chmod(0o755)
        self.env = dict(DOJO_KITTY=str(self.fake), DOJO_YAZI="/fake/yazi",
                        DOJO_YAZI_CHOOSER_PLUGIN=str(Path(__file__).with_name("yazi-chooser.lua").resolve()),
                        CALLS=str(self.root / "calls"), XDG_CONFIG_HOME=str(self.root / "config"))

    def run_picker(self, result="", multiple=0, directory=0, save=0, status=0):
        with patch.dict(os.environ, dict(self.env, RESULT=result, STATUS=str(status))):
            chooser.main([str(multiple), str(directory), str(save),
                          str(self.root / "new file.txt") if save else str(self.folder), str(self.out), "0"])
        self.assertEqual((self.root / "calls").read_text(), "call\n")
        return self.out.read_text()

    def test_cancel_is_empty(self):
        self.assertEqual(self.run_picker(), "")

    def test_failure_discards_partial_output(self):
        self.assertEqual(self.run_picker(str(self.file), status=1), "")

    def test_spaces_and_shell_characters(self):
        self.assertEqual(self.run_picker(str(self.file)), str(self.file) + "\n")

    def test_multiple(self):
        result = f"{self.file}\n{self.second}\n"
        self.assertEqual(self.run_picker(result, multiple=1), result)

    def test_single_truncates_selection(self):
        self.assertEqual(self.run_picker(f"{self.file}\n{self.second}"), str(self.file) + "\n")

    def test_directory(self):
        self.assertEqual(self.run_picker(str(self.folder), directory=1), str(self.folder) + "\n")

    def test_directory_cancel_does_not_select_cwd(self):
        self.assertEqual(self.run_picker(directory=1), "")

    def test_save_nonexisting(self):
        dest = self.folder / "new file.txt"
        self.assertEqual(self.run_picker(str(dest), save=1), str(dest) + "\n")
        self.assertFalse(dest.exists())

    def test_save_cancel(self):
        self.assertEqual(self.run_picker(save=1), "")

    def test_invalid_relative_result(self):
        self.assertEqual(self.run_picker("relative.txt"), "")


if __name__ == "__main__":
    unittest.main()
