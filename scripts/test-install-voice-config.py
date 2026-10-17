import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest


spec = importlib.util.spec_from_file_location("voice_config", Path(__file__).with_name("install-voice-config.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class VoiceConfigTests(unittest.TestCase):
    root = Path("/fixture/Dojo")
    launcher = Path("/fixture/home/.local/bin/dojo-agent")

    def update(self, original):
        return module.update_config(original, self.root, self.launcher)

    def test_new_setup_uses_stable_launcher_and_native_tools(self):
        original = '# Retain this setting exactly.\nmodel = "fixture"\n'
        updated = self.update(original)
        self.assertTrue(updated.startswith(original))
        server = tomllib.loads(updated)["mcp_servers"]["dojo-desktop"]
        self.assertEqual(server["command"], str(self.launcher))
        self.assertIn("desktop_accessibility", server["enabled_tools"])
        self.assertIn("desktop_accessibility_action", server["enabled_tools"])
        self.assertEqual(self.update(updated), updated)

    def test_migrates_own_legacy_table_preserving_tools_and_unrelated_config(self):
        before = '# exact prefix\nmodel = "fixture"\n'
        after = '\n[mcp_servers.unrelated]\ncommand = "keep this"\n'
        original = before + '''[mcp_servers."dojo-desktop"]
command = "/fixture/Dojo/target/debug/dojo-agent"
args = ["--name", "existing session"]
enabled_tools = [
  "custom_tool", # retain enabled tool membership
  "desktop_context",
]
env_vars = ["KEEP_ME"]
enabled = false
''' + after
        updated = self.update(original)
        self.assertTrue(updated.startswith(before))
        self.assertTrue(updated.endswith(after))
        server = tomllib.loads(updated)["mcp_servers"]["dojo-desktop"]
        self.assertEqual(server["command"], str(self.launcher))
        self.assertEqual(server["args"], ["--name", "existing session"])
        self.assertEqual(server["enabled_tools"][:2], ["custom_tool", "desktop_context"])
        self.assertEqual(len(server["enabled_tools"]), len(set(server["enabled_tools"])))
        self.assertNotIn("desktop_action", server["enabled_tools"])
        self.assertEqual(server["env_vars"], ["KEEP_ME", *module.FORWARDED_ENV])
        self.assertFalse(server["enabled"])
        self.assertEqual(self.update(updated), updated)

    def test_absent_tool_allowlist_remains_unrestricted(self):
        updated = self.update('[mcp_servers.dojo-desktop]\ncommand = "dojo-agent"\n')
        self.assertNotIn("enabled_tools", tomllib.loads(updated)["mcp_servers"]["dojo-desktop"])

    def test_unrelated_server_command_is_refused(self):
        original = '[mcp_servers.dojo-desktop]\ncommand = "/another/agent"\n'
        with self.assertRaisesRegex(ValueError, "preserving"):
            self.update(original)

    def test_header_text_in_multiline_string_is_not_an_edit_target(self):
        original = 'description = """\n[mcp_servers.dojo-desktop]\nexample only\n"""\n'
        updated = self.update(original)
        self.assertTrue(updated.startswith(original))
        self.assertEqual(tomllib.loads(updated)["mcp_servers"]["dojo-desktop"]["command"], str(self.launcher))

    def test_explicit_install_migrates_only_a_temporary_config_and_keeps_backup(self):
        with tempfile.TemporaryDirectory(prefix="dojo-voice-config-") as temporary:
            fixture = Path(temporary)
            target = fixture / "codex/config.toml"
            target.parent.mkdir()
            root = Path(module.__file__).resolve().parent.parent
            original = ('model = "fixture"\n[mcp_servers.dojo-desktop]\ncommand = "'
                        + str(root / "target/debug/dojo-agent") + '"\nenabled_tools = ["keep_tool"]\n')
            target.write_text(original)
            environment = {**os.environ, "HOME": str(fixture / "home"), "CODEX_HOME": str(target.parent)}
            command = [sys.executable, module.__file__]
            first = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertEqual(first.returncode, 0, first.stderr)
            server = tomllib.loads(target.read_text())["mcp_servers"]["dojo-desktop"]
            self.assertEqual(server["command"], str(fixture / "home/.local/bin/dojo-agent"))
            self.assertIn("keep_tool", server["enabled_tools"])
            self.assertIn("desktop_accessibility_action", server["enabled_tools"])
            backups = list(target.parent.glob("config.toml.dojo-backup-*"))
            self.assertEqual(len(backups), 1)
            self.assertEqual(backups[0].read_text(), original)
            second = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertEqual(second.returncode, 0, second.stderr)
            self.assertEqual(len(list(target.parent.glob("config.toml.dojo-backup-*"))), 1)


if __name__ == "__main__":
    unittest.main()
