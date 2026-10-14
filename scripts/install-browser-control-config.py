#!/usr/bin/env python3
"""Merge the packaged Browser Control MCP into installed agents' user settings."""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
import tomllib


SERVER = "browser_control"


def owned(command):
    return isinstance(command, str) and (
        command == "browser-control-mcp"
        or command.endswith("/bin/browser-control-mcp")
    )


def replace_file(path, original, updated):
    if original == updated:
        return False
    if path.is_symlink():
        raise ValueError(f"Declarative configuration must be edited at its source: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        timestamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
        backup = path.with_name(f"{path.name}.dojo-browser-backup-{timestamp}")
        shutil.copy2(path, backup)
        backup.chmod(0o600)
    descriptor, temporary = tempfile.mkstemp(prefix=".dojo-browser-config-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as output:
            output.write(updated)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)
    return True


def install_codex(path, command):
    original = path.read_text() if path.exists() else ""
    parsed = tomllib.loads(original)
    old = parsed.get("mcp_servers", {}).get(SERVER)
    desired = {"command": command, "args": [], "startup_timeout_sec": 15, "tool_timeout_sec": 120}
    if old == desired:
        return False
    if old is not None and not owned(old.get("command")):
        raise ValueError(f"Preserving existing {SERVER} entry: {path}")
    section = (
        f"[mcp_servers.{SERVER}]\ncommand = {json.dumps(command)}\n"
        "args = []\nstartup_timeout_sec = 15\ntool_timeout_sec = 120\n"
    )
    if old is None:
        updated = original.rstrip() + "\n\n" + section
    else:
        header = re.search(r'^\[mcp_servers\.(?:browser_control|"browser_control")\]\s*$', original, re.M)
        if header is None:
            raise ValueError(f"Unsupported inline MCP table; preserving {path}")
        next_header = re.search(r'^\[', original[header.end():], re.M)
        end = header.end() + next_header.start() if next_header else len(original)
        updated = original[:header.start()] + section + "\n" + original[end:]
    if tomllib.loads(updated).get("mcp_servers", {}).get(SERVER) != desired:
        raise ValueError(f"Failed to construct MCP entry: {path}")
    return replace_file(path, original, updated)


def install_json(path, command, key, typed=False):
    original = path.read_text() if path.exists() else ""
    parsed = json.loads(original) if original.strip() else {}
    servers = parsed.setdefault(key, {})
    old = servers.get(SERVER)
    desired = {"command": command, "args": []}
    if typed:
        desired["type"] = "stdio"
    if old == desired:
        return False
    if old is not None and not owned(old.get("command")):
        raise ValueError(f"Preserving existing {SERVER} entry: {path}")
    servers[SERVER] = desired
    return replace_file(path, original, json.dumps(parsed, indent=2) + "\n")


def install(home, command, environ=None):
    environ = os.environ if environ is None else environ
    changed = []
    codex_home = Path(environ.get("CODEX_HOME", str(home / ".codex")))
    if codex_home.exists():
        if install_codex(codex_home / "config.toml", command):
            changed.append("Codex")
    claude = home / ".claude.json"
    if claude.exists() or (home / ".claude").exists():
        if install_json(claude, command, "mcpServers", typed=True):
            changed.append("Claude Code")
    config_home = Path(environ.get("XDG_CONFIG_HOME", str(home / ".config")))
    zed = config_home / "zed"
    if zed.exists():
        if install_json(zed / "settings.json", command, "context_servers"):
            changed.append("Zed")
    return changed


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--command", required=True)
    args = parser.parse_args()
    command = str(Path(args.command).resolve(strict=True))
    if not os.access(command, os.X_OK):
        parser.error("MCP command must be executable")
    for agent in install(Path.home(), command):
        print(f"Installed Browser Control MCP for {agent}.")
