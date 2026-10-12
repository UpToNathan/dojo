#!/usr/bin/env python3
"""Verify tray GPU allocations during redraws and icon lifecycle changes.

Build first: devenv shell -- cargo build --example tray_gpu_probe
Run: devenv shell -- python3 scripts/check-tray-gpu-memory.py
"""
import json
import os
from pathlib import Path
import subprocess


def main():
    root = Path(__file__).resolve().parents[1]
    session = dict(
        line.split("=", 1)
        for line in subprocess.check_output(
            ["systemctl", "--user", "show-environment"], text=True
        ).splitlines()
        if "=" in line
    )
    environment = os.environ.copy()
    environment.update(
        {key: session[key] for key in ("WAYLAND_DISPLAY", "NIRI_SOCKET", "XDG_RUNTIME_DIR")}
    )
    results = {}
    for mode in ("reuse", "recreate", "cache", "churn"):
        used = int(Path("/sys/class/drm/card1/device/mem_info_vram_used").read_text())
        if used > 4 * 1024**3:
            raise SystemExit("Probe stopped: GPU allocation already exceeds 4 GiB.")
        process = subprocess.run(
            [str(root / "target/debug/examples/tray_gpu_probe"), mode],
            env=environment,
            capture_output=True,
            text=True,
            timeout=45,
            check=True,
        )
        result = json.loads(process.stdout.strip())
        results[mode] = result
        print(json.dumps(result), flush=True)
    assert results["reuse"]["growth_KiB"] < 16 * 1024, results
    assert results["recreate"]["growth_KiB"] > 64 * 1024, results
    assert results["cache"]["growth_KiB"] < 16 * 1024, results
    assert results["churn"]["growth_KiB"] < 32 * 1024, results
    print("PASS: tray cache bounds GPU allocations across redraws, replacements, hiding, and removal.")


if __name__ == "__main__":
    main()
