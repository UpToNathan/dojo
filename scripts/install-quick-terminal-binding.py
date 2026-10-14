#!/usr/bin/env python3
"""Activate only the maintained quick-terminal binding, preserving the current Niri configuration."""
import os
from pathlib import Path
import subprocess
import time

home = Path.home()
config = Path(os.environ.get("XDG_CONFIG_HOME", home / ".config")) / "niri/config.kdl"
source = config.read_text()
old = 'spawn "kitten" "quick-access-terminal"'
new = f'spawn "{home}/.local/bin/dojo-desktop" "--terminal"'
if old not in source:
    if new not in source:
        raise SystemExit("Quick terminal binding was customized; no configuration changed")
    raise SystemExit("Quick terminal binding is already active")
if source.count(old) != 1:
    raise SystemExit("Expected one quick terminal binding; no configuration changed")
root = Path(os.environ.get("XDG_STATE_HOME", home / ".local/state")) / "dojo/quick-terminal"
root.mkdir(parents=True, exist_ok=True)
generated = root / "niri.kdl"
generated.write_text(source.replace(old, new))
pid = subprocess.check_output(["pgrep", "-xo", "niri"], text=True).strip()
binary = Path(f"/proc/{pid}/exe").resolve()
subprocess.run([str(binary), "validate", "--config", str(generated)], check=True)
backup = config.with_name(f"config.kdl.backup-{time.time_ns()}")
config.rename(backup)
config.symlink_to(generated)
subprocess.run([str(binary), "msg", "action", "load-config-file"], check=True)
print(f"Activated quick terminal binding; previous configuration is {backup}")
