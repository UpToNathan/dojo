#!/usr/bin/env python3
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("filemanager", Path(__file__).with_name("yazi-filemanager.py"))
fm = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fm)


class FileManagerTests(unittest.TestCase):
    def test_uri_with_spaces(self):
        self.assertEqual(fm.local_path("file:///tmp/a%20b%24.txt"), "/tmp/a b$.txt")

    def test_remote_and_invalid_uris_rejected(self):
        for uri in ["https://example.org/a", "file://remote/a", "file:relative", "file:///tmp/%00"]:
            with self.subTest(uri=uri), self.assertRaises(ValueError):
                fm.local_path(uri)

    def test_launch_uses_argv_not_shell_and_deduplicates(self):
        with patch.dict(os.environ, DOJO_KITTY="/kitty", DOJO_YAZI="/yazi"), patch.object(fm.subprocess, "Popen") as popen:
            fm.launch(["file:///tmp/a%20b", "file:///tmp/a%20b"])
            popen.assert_called_once_with(["/kitty", "--title", "Yazi", "/yazi", "/tmp/a b"], start_new_session=True)

    def test_bad_uri_prevents_partial_launch(self):
        with patch.object(fm.subprocess, "Popen") as popen, self.assertRaises(ValueError):
            fm.launch(["file:///tmp/a", "https://example.org/a"])
        popen.assert_not_called()

    def test_showfolders(self):
        with tempfile.TemporaryDirectory() as tmp, patch.dict(os.environ, DOJO_KITTY="/kitty", DOJO_YAZI="/yazi"), patch.object(fm.subprocess, "Popen") as popen:
            fm.launch([Path(tmp).as_uri()], folders=True)
            self.assertEqual(popen.call_args.args[0][-1], tmp)
            with self.assertRaises(ValueError):
                fm.launch([Path(tmp, "missing").as_uri()], folders=True)


if __name__ == "__main__":
    unittest.main()
