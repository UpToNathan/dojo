#!/usr/bin/env python3
"""Explicitly install or migrate Dojo's desktop MCP without replacing other settings."""
import datetime
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
import tomllib


TOOLS = [
    "desktop_context", "desktop_action", "desktop_release", "desktop_feedback",
    "desktop_launch", "desktop_screenshot", "desktop_accessibility",
    "desktop_accessibility_action",
]
FORWARDED_ENV = ["NIRI_SOCKET", "XDG_RUNTIME_DIR", "WAYLAND_DISPLAY"]
REQUIRED_TOOLS = ["desktop_launch", "desktop_screenshot", "desktop_accessibility", "desktop_accessibility_action"]


def tables(text):
    """Find actual table headers, excluding text inside multiline TOML values."""
    headers = []
    for match in re.finditer(r"(?m)^[ \t]*\[[^\n]*\][ \t]*(?:#[^\n]*)?$", text):
        try:
            tomllib.loads(text[:match.start()])
            header = tomllib.loads(match.group())
        except tomllib.TOMLDecodeError:
            continue
        headers.append((match.start(), header))
    return headers


def replace_entry(section, key, value):
    replacement = key + " = " + json.dumps(value)
    match = re.search(rf"(?m)^[ \t]*{re.escape(key)}[ \t]*=", section)
    if match is None:
        return section.rstrip("\n") + "\n" + replacement + "\n"
    end = section.find("\n", match.end())
    while True:
        end = len(section) if end == -1 else end
        try:
            parsed = tomllib.loads(section[match.start():end])
            if set(parsed) == {key}:
                return section[:match.start()] + replacement + section[end:]
        except tomllib.TOMLDecodeError:
            pass
        if end == len(section):
            raise ValueError(f"Cannot safely update Dojo's {key}; preserving configuration.")
        end = section.find("\n", end + 1)


def update_config(original, root, launcher):
    parsed = tomllib.loads(original)
    server = parsed.get("mcp_servers", {}).get("dojo-desktop")
    if server is None:
        addition = f'''\n[mcp_servers.dojo-desktop]
command = {json.dumps(str(launcher))}
args = ["--name", "dojo-voice"]
env_vars = {json.dumps(FORWARDED_ENV)}
enabled_tools = {json.dumps(TOOLS)}
startup_timeout_sec = 10
tool_timeout_sec = 15
'''
        updated = original + addition
    else:
        accepted = {str(launcher), "dojo-agent", str(root / "scripts/dojo-agent"),
                    str(root / "target/debug/dojo-agent")}
        if server.get("command") not in accepted:
            raise ValueError("Existing dojo-desktop configuration differs; preserving it.")
        headers = tables(original)
        position = next((index for index, (_, header) in enumerate(headers)
                         if header.get("mcp_servers", {}).get("dojo-desktop") == {}), None)
        if position is None:
            raise ValueError("Cannot safely locate Dojo's table; preserving configuration.")
        start = headers[position][0]
        end = headers[position + 1][0] if position + 1 < len(headers) else len(original)
        section = replace_entry(original[start:end], "command", str(launcher))
        enabled = server.get("enabled_tools")
        if enabled is not None:
            if not isinstance(enabled, list) or not all(isinstance(tool, str) for tool in enabled):
                raise ValueError("Existing enabled_tools is invalid; preserving configuration.")
            section = replace_entry(section, "enabled_tools", enabled + [tool for tool in REQUIRED_TOOLS if tool not in enabled])
        forwarded = server.get("env_vars", [])
        if not isinstance(forwarded, list) or not all(isinstance(name, str) for name in forwarded):
            raise ValueError("Existing env_vars is invalid; preserving configuration.")
        section = replace_entry(section, "env_vars", forwarded + [name for name in FORWARDED_ENV if name not in forwarded])
        updated = original[:start] + section + original[end:]
    tomllib.loads(updated)
    return updated


def install():
    root = Path(__file__).resolve().parent.parent
    launcher = Path.home() / ".local/bin/dojo-agent"
    target = Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "config.toml"
    original = target.read_text() if target.exists() else ""
    try:
        updated = update_config(original, root, launcher)
    except ValueError as error:
        raise SystemExit(str(error)) from error
    if updated == original:
        return
    target.parent.mkdir(parents=True, exist_ok=True)
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
    print("Installed Dojo desktop MCP for Codex using the stable launcher.")


if __name__ == "__main__":
    install()
