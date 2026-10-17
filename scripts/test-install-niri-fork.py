import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class NiriInstallationTests(unittest.TestCase):
    def test_broker_pin_is_required_exported_rooted_and_versioned(self):
        broker = os.environ.get("DOJO_BROWSER_CONTROL_BROKER_EXE")
        mcp = os.environ.get("DOJO_BROWSER_CONTROL_MCP_EXE")
        if not broker:
            self.skipTest("Set DOJO_BROWSER_CONTROL_BROKER_EXE to a Nix-built broker")
        installer = Path(__file__).with_name("install-niri-fork.sh")
        with tempfile.TemporaryDirectory(prefix="dojo-niri-installer-test-") as temporary:
            fixture = Path(temporary)
            scripts = fixture / "repo/scripts"
            scripts.mkdir(parents=True)
            shutil.copy2(installer, scripts / installer.name)
            shutil.copy2(Path(__file__).with_name("dojo-agent"), scripts / "dojo-agent")
            (scripts / "dojo-niri-session").write_text("fixture session wrapper\n")
            session_config = fixture / "repo/session-config"
            session_config.mkdir()
            (session_config / "90-dojo-niri.conf").write_text("fixture service config\n")
            binary = fixture / "repo/vendor/niri/target/release/niri"
            binary.parent.mkdir(parents=True)
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(0o755)
            commands = fixture / "commands"
            commands.mkdir()
            for name in ["ldd", "systemctl"]:
                path = commands / name
                path.write_text("#!/bin/sh\nexit 0\n")
                path.chmod(0o755)
            nix_store = commands / "nix-store"
            nix_store.write_text('#!/bin/sh\nln -s "$5" "$2"\n')
            nix_store.chmod(0o755)
            fixture_home = fixture / "home"
            fixture_home.mkdir()
            environment = {
                **os.environ,
                "HOME": str(fixture_home),
                "XDG_DATA_HOME": str(fixture / "data"),
                "XDG_CONFIG_HOME": str(fixture / "config"),
                "PATH": str(commands) + ":" + os.environ["PATH"],
                "LD_LIBRARY_PATH": "/nix/store/example/lib",
            }
            environment.pop("DOJO_BROWSER_CONTROL_BROKER_EXE", None)
            environment.pop("DOJO_BROWSER_CONTROL_MCP_EXE", None)
            command = ["bash", str(scripts / installer.name)]
            missing = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn("must pin", missing.stderr)
            self.assertFalse((fixture / "data").exists())
            environment["DOJO_BROWSER_CONTROL_BROKER_EXE"] = str(binary)
            mutable = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertNotEqual(mutable.returncode, 0)
            environment["DOJO_BROWSER_CONTROL_BROKER_EXE"] = broker
            environment["DOJO_BROWSER_CONTROL_MCP_EXE"] = str(binary)
            invalid_mcp = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertNotEqual(invalid_mcp.returncode, 0)
            self.assertIn("DOJO_BROWSER_CONTROL_MCP_EXE must pin", invalid_mcp.stderr)
            if mcp:
                environment["DOJO_BROWSER_CONTROL_MCP_EXE"] = mcp
            else:
                environment.pop("DOJO_BROWSER_CONTROL_MCP_EXE")
            environment["DOJO_PERCEPTION_INSTALLATION"] = str(binary)
            invalid_perception = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertNotEqual(invalid_perception.returncode, 0)
            self.assertIn("DOJO_PERCEPTION_INSTALLATION must pin", invalid_perception.stderr)
            environment.pop("DOJO_PERCEPTION_INSTALLATION")
            success = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertEqual(success.returncode, 0, success.stderr)
            self.assertEqual((fixture_home / ".local/bin/dojo-agent").resolve(), scripts / "dojo-agent")
            installed = (fixture / "data/dojo/niri/current").resolve()
            self.assertIn("export DOJO_BROWSER_CONTROL_BROKER_EXE=" + broker, (installed / "environment.sh").read_text())
            self.assertTrue((installed / "roots" / Path(broker).parts[3]).is_symlink())
            if mcp:
                self.assertIn("export DOJO_BROWSER_CONTROL_MCP_EXE=" + mcp, (installed / "environment.sh").read_text())
                self.assertTrue((installed / "roots" / Path(mcp).parts[3]).is_symlink())
            environment["LD_LIBRARY_PATH"] = "/nix/store/new-runtime/lib"
            updated = subprocess.run(command, env=environment, capture_output=True, text=True)
            self.assertEqual(updated.returncode, 0, updated.stderr)
            self.assertNotEqual((fixture / "data/dojo/niri/current").resolve(), installed)
            self.assertEqual((fixture / "data/dojo/niri/previous").resolve(), installed)
            self.assertNotIn("DOJO_PERCEPTION_INSTALLATION", (fixture / "data/dojo/niri/current/environment.sh").read_text())
            perception = os.environ.get("DOJO_PERCEPTION_INSTALLATION")
            if perception:
                environment["DOJO_PERCEPTION_INSTALLATION"] = perception
                staged = subprocess.run(command, env=environment, capture_output=True, text=True)
                self.assertEqual(staged.returncode, 0, staged.stderr)
                current = (fixture / "data/dojo/niri/current").resolve()
                self.assertIn("export DOJO_PERCEPTION_INSTALLATION=" + perception, (current / "environment.sh").read_text())
                self.assertTrue((current / "roots" / Path(perception).parts[3]).is_symlink())


if __name__ == "__main__":
    unittest.main()
