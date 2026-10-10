#!/usr/bin/env python3
"""Host integration test; requires the one-time Home Manager preset provisioning.

Uses disposable XDG state and fresh Neovim processes. It never changes the
selected desktop theme or touches buffers in an existing editor.
"""

import json
import os
from pathlib import Path
import subprocess
import tempfile

state = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state"))
original = json.loads((state / "dojo/themes/current/nvim.json").read_text())
expected = {
    "tokyo-night-night": "#1a1b26",
    "tokyo-night-storm": "#24283b",
    "tokyo-night-moon": "#222436",
    "tokyo-night-day": "#e1e2e7",
    "catppuccin-latte": "#eff1f5",
    "catppuccin-frappe": "#303446",
    "catppuccin-macchiato": "#24273a",
    "catppuccin-mocha": "#1e1e2e",
}

with tempfile.TemporaryDirectory(prefix="dojo-nvim-presets-check-") as directory:
    current = Path(directory) / "dojo/themes/current"
    current.mkdir(parents=True)
    for preset, background in expected.items():
        appearance = "light" if preset in {"tokyo-night-day", "catppuccin-latte"} else "dark"
        (current / "nvim.json").write_text(
            json.dumps(dict(original, preset=preset, appearance=appearance))
        )
        check = (
            'local h=vim.api.nvim_get_hl(0,{name="Normal",link=false});'
            'local bg=string.format("#%06x",h.bg or 0);'
            f"local expected={json.dumps(background)};local matches=true;"
            # Native plugins can round a channel when converting through HSL.
            'for offset=2,6,2 do if math.abs(tonumber(bg:sub(offset,offset+1),16)-'
            'tonumber(expected:sub(offset,offset+1),16))>1 then matches=false end end;'
            "if not matches or "
            f"vim.g.dojo_native_preset~={json.dumps(preset)} then "
            'print("FAIL "..bg.." native="..tostring(vim.g.dojo_native_preset));'
            'vim.cmd("cquit 1") end;'
            f'print("PASS {preset}")'
        )
        result = subprocess.run(
            ["nvim", "--headless", "+lua " + check, "+qa"],
            env=dict(os.environ, XDG_STATE_HOME=directory),
            capture_output=True,
            text=True,
            timeout=15,
        )
        print(result.stderr.strip())
        if result.returncode:
            raise SystemExit(result.returncode)
