import importlib.util
import json
from pathlib import Path
import tempfile
import tomllib
import unittest

spec = importlib.util.spec_from_file_location("installer", Path(__file__).with_name("install-browser-control-config.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class ConfigurationTests(unittest.TestCase):
    def test_merge_preserves_unrelated_servers_and_settings_and_is_idempotent(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            codex = home / ".codex/config.toml"
            codex.parent.mkdir()
            source = '# preferences\nmodel = "local"\n[mcp_servers.dojo-desktop]\ncommand = "dojo-agent"\n'
            codex.write_text(source)
            claude = home / ".claude.json"
            claude.write_text('{"preferences":{"theme":"dark"},"mcpServers":{"other":{"command":"other"}}}')
            zed = home / ".config/zed/settings.json"
            zed.parent.mkdir(parents=True)
            zed.write_text('{"theme":"Dojo"}')
            command = "/nix/store/example/bin/browser-control-mcp"
            self.assertEqual(installer.install(home, command, {}), ["Codex", "Claude Code"])
            self.assertIn(source.strip(), codex.read_text())
            self.assertEqual(tomllib.loads(codex.read_text())["mcp_servers"]["browser_control"]["command"], command)
            self.assertEqual(json.loads(claude.read_text())["preferences"], {"theme": "dark"})
            self.assertEqual(json.loads(claude.read_text())["mcpServers"]["other"], {"command": "other"})
            self.assertEqual(json.loads(zed.read_text())["theme"], "Dojo")
            backups = list(home.rglob("*.dojo-browser-backup-*"))
            self.assertEqual(len(backups), 2)
            self.assertTrue(all(path.stat().st_mode & 0o777 == 0o600 for path in backups))
            self.assertEqual(installer.install(home, command, {}), [])
            self.assertEqual(len(list(home.rglob("*.dojo-browser-backup-*"))), 2)

    def test_updates_owned_entry_and_preserves_following_table(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "config.toml"
            path.write_text('[mcp_servers.browser_control]\ncommand="/nix/store/old/bin/browser-control-mcp"\n[features]\nweb=true\n')
            installer.install_codex(path, "/nix/store/new/bin/browser-control-mcp")
            value = tomllib.loads(path.read_text())
            self.assertEqual(value["features"], {"web": True})
            self.assertEqual(value["mcp_servers"]["browser_control"]["command"], "/nix/store/new/bin/browser-control-mcp")

    def test_refuses_unowned_and_declarative_configs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "config.toml"
            path.write_text('[mcp_servers.browser_control]\ncommand="custom-controller"\n')
            before = path.read_bytes()
            with self.assertRaisesRegex(ValueError, "Preserving"):
                installer.install_codex(path, "/nix/store/new/bin/browser-control-mcp")
            self.assertEqual(path.read_bytes(), before)
            source = root / "managed.json"
            source.write_text("{}")
            link = root / "settings.json"
            link.symlink_to(source)
            with self.assertRaisesRegex(ValueError, "Declarative"):
                installer.install_json(link, "/nix/store/new/bin/browser-control-mcp", "context_servers")
            self.assertEqual(source.read_text(), "{}")


if __name__ == "__main__":
    unittest.main()
