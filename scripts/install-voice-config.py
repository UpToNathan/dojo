#!/usr/bin/env python3
"""Add Dojo's desktop MCP to Codex without replacing existing settings."""
import datetime
import os
from pathlib import Path
import shutil
import tempfile
import tomllib


def install():
    root = Path(__file__).resolve().parent.parent
    target = Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "config.toml"
    target.parent.mkdir(parents=True, exist_ok=True)
    original = target.read_text() if target.exists() else ""
    parsed = tomllib.loads(original)
    server = parsed.get("mcp_servers", {}).get("dojo-desktop")
    if server is not None:
        if server.get("command") != str(root / "target/debug/dojo-agent"):
            raise SystemExit("Existing dojo-desktop configuration differs; preserving it.")
        tools = server.get("enabled_tools")
        required_tools = ["desktop_launch", "desktop_screenshot"]
        missing_tools = [tool for tool in required_tools if tools is not None and tool not in tools]
        env_vars = server.get("env_vars", [])
        if missing_tools or "WAYLAND_DISPLAY" not in env_vars:
            start = original.index("[mcp_servers.dojo-desktop]")
            end = original.find("\n[", start + 1)
            end = len(original) if end == -1 else end
            section = original[start:end]
            import re
            if tools is not None:
                section = re.sub(r'enabled_tools\s*=\s*\[[^]]*\]',
                                 'enabled_tools = ' + repr(tools + missing_tools).replace("'", '"'), section)
            forwarded = env_vars + (["WAYLAND_DISPLAY"] if "WAYLAND_DISPLAY" not in env_vars else [])
            if "env_vars" in server:
                section = re.sub(r'env_vars\s*=\s*\[[^]]*\]', 'env_vars = ' + repr(forwarded).replace("'", '"'), section)
            else:
                section += '\nenv_vars = ["NIRI_SOCKET", "XDG_RUNTIME_DIR", "WAYLAND_DISPLAY"]\n'
            updated = original[:start] + section + original[end:]
            tomllib.loads(updated)
            timestamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
            shutil.copy2(target, target.with_name(f"config.toml.dojo-backup-{timestamp}"))
            target.write_text(updated)
        return
    addition = f'''\n[mcp_servers.dojo-desktop]
command = "{root}/target/debug/dojo-agent"
args = ["--name", "dojo-voice"]
env_vars = ["NIRI_SOCKET", "XDG_RUNTIME_DIR", "WAYLAND_DISPLAY"]
enabled_tools = ["desktop_context", "desktop_action", "desktop_release", "desktop_feedback", "desktop_launch", "desktop_screenshot"]
startup_timeout_sec = 10
tool_timeout_sec = 15
'''
    updated = original + addition
    tomllib.loads(updated)
    if target.exists():
        timestamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
        shutil.copy2(target, target.with_name(f"config.toml.dojo-backup-{timestamp}"))
    descriptor, temporary = tempfile.mkstemp(prefix=".dojo-config-", dir=target.parent)
    try:
        with os.fdopen(descriptor, "w") as output:
            output.write(updated)
        os.replace(temporary, target)
    finally:
        Path(temporary).unlink(missing_ok=True)
    print("Installed Dojo desktop MCP for Codex.")


if __name__ == "__main__":
    install()
