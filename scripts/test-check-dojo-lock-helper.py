#!/usr/bin/env python3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

CHECK = Path(__file__).with_name("check-dojo-lock-helper.py")

# A stand-in loader drops "--library-path DIR" and runs the helper directly.
LOADER = '#!/bin/sh\nshift 2\nexec "$@"\n'

# --check prints ok, or the failing step on stderr, and exits 0 or 69.
HELPER = """#!/bin/sh
[ "$#" -eq 0 ] && exit 64
{behaviour}
"""
OK = 'if [ "$1" = --check ]; then echo ok; exit 0; fi\nexit 1'
NO_LIBPAM = 'if [ "$1" = --check ]; then echo "libpam unavailable: libpam.so.0: cannot open shared object file" >&2; exit 69; fi\nexit 1'
SILENT = "exit 0"


class HelperCheckTests(unittest.TestCase):
    def check(self, helper_body, usage_ok=True):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            loader = root / "loader"
            loader.write_text(LOADER)
            helper = root / "helper"
            text = HELPER.format(behaviour=helper_body)
            if not usage_ok:
                text = text.replace("exit 64", "exit 0")
            helper.write_text(text)
            for script in (loader, helper):
                script.chmod(0o755)
            empty = root / "empty"
            empty.mkdir()
            return subprocess.run(
                [
                    sys.executable, "-I", str(CHECK), str(helper),
                    "--loader", str(loader), "--library-path", str(empty),
                ],
                capture_output=True,
                text=True,
            )

    def test_accepts_a_helper_that_refuses_usage_and_answers_check_with_ok(self):
        result = self.check(OK)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("libpam loads", result.stdout)

    def test_names_libpam_when_the_helper_reports_it_unavailable(self):
        result = self.check(NO_LIBPAM)
        self.assertEqual(result.returncode, 1)
        self.assertIn("libpam cannot be loaded", result.stderr)
        self.assertIn("cannot open shared object file", result.stderr)

    def test_rejects_a_helper_that_does_not_print_ok(self):
        result = self.check(SILENT)
        self.assertEqual(result.returncode, 1)
        self.assertIn("--check failed", result.stderr)
        self.assertIn("helper --check failed", result.stderr)

    def test_rejects_a_helper_that_does_not_refuse_missing_arguments(self):
        result = self.check(OK, usage_ok=False)
        self.assertEqual(result.returncode, 1)
        self.assertIn("expected 64", result.stderr)


if __name__ == "__main__":
    unittest.main()
