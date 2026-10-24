#!/usr/bin/env python3
"""Open every launcher command's view in the nested Niri and check the shell survives using it.

Runs in the nested Niri that run-nested.sh started, never the live session. Needs `wtype` on PATH
(or DOJO_WTYPE) and the agent-input fixture.

    nix shell nixpkgs#wtype -c devenv shell -- tools/agent-input/run-nested.sh --launcher-views

For each command key the driver runs `dojo --launcher-view <key>`, waits for the layer surface,
checks the panel was drawn, types a character into the view's search box, presses Escape to leave
the view (the launcher stays open on the root list) and Escape again to close it, and checks the
fixture gets the keyboard back. The shell must still be one process at the end and must not have
logged a panic. Set DOJO_VIEWS_SCREENSHOTS to a directory to keep a screenshot of each view.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from run import Peer, events, screenshot, wait_for

PANEL_COLOUR = ("30", "30", "46")
TOP_BAR_HEIGHT = 24
VIEWS = ["emoji", "clipboard", "windows", "snippets", "color", "processes", "services"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    directory = Path(args.log).parent
    runtime = directory / "shell-runtime"
    runtime.mkdir(mode=0o700)
    env = dict(os.environ)
    env["WAYLAND_DISPLAY"] = str(Path(os.environ["XDG_RUNTIME_DIR"]) / os.environ["WAYLAND_DISPLAY"])
    env["XDG_RUNTIME_DIR"] = str(runtime)
    env["XDG_STATE_HOME"] = str(directory / "shell-state")
    env["XDG_CONFIG_HOME"] = str(directory / "shell-config")
    env["XDG_DATA_HOME"] = str(directory / "shell-data")
    env.pop("DBUS_SESSION_BUS_ADDRESS", None)
    # The shell gets its own config directory, which hides the user's fonts, and the emoji face is
    # one of them; link the real fontconfig in so colour emoji can be seen.
    real_fontconfig = Path(os.environ.get("HOME", "")) / ".config/fontconfig"
    if real_fontconfig.is_dir():
        (directory / "shell-config").mkdir(exist_ok=True)
        (directory / "shell-config/fontconfig").symlink_to(real_fontconfig)
    query = Peer(os.environ["NIRI_SOCKET"])
    binary = os.environ.get("DOJO_BINARY", str(root / "target/debug/dojo"))
    fixture = os.environ.get("DOJO_FIXTURE", str(Path(__file__).parent / "target/debug/dojo-agent-input-fixture"))
    wtype = os.environ.get("DOJO_WTYPE", "wtype")
    pixel_diff = os.environ.get("DOJO_PIXEL_DIFF_BINARY", str(Path(__file__).parent / "target/debug/pixel-diff"))
    keep = os.environ.get("DOJO_VIEWS_SCREENSHOTS")
    fixture_log = directory / "fixture.jsonl"
    checks = []

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": detail})
        print(("PASS " if ok else "FAIL ") + name + (f" ({detail})" if detail else ""), flush=True)

    def layers():
        return {item["namespace"]: item for item in query.request("Layers")["Layers"]}

    def count(kind):
        return sum(1 for e in events(fixture_log) if e.get("event") == kind)

    def key(name):
        subprocess.run([wtype, "-k", name], env=env, check=True, timeout=10)

    def panel_pixels(path, scale, width):
        out = subprocess.check_output(
            [pixel_diff, "--color", path, str(round(width / 2 * scale)), str(round((TOP_BAR_HEIGHT + 8) * scale)),
             str(round(4 * scale)), *PANEL_COLOUR], text=True)
        return json.loads(out)["matching_pixels"]

    shell = recorder = None
    try:
        with open(directory / "shell.log", "w") as shell_log, open(fixture_log, "w") as recorder_log:
            shell = subprocess.Popen([binary, "--enable"], env=env, stdout=shell_log, stderr=shell_log)
            wait_for(lambda: any(n.startswith("dojo-frame-") for n in layers()), "shell frame")
            wait_for(lambda: not any(n.startswith("dojo-session") for n in layers()), "opening overlay gone")
            recorder = subprocess.Popen([fixture], env=env, stdout=recorder_log)
            wait_for(lambda: count("keyboard_enter") >= 1, "fixture keyboard focus")
            time.sleep(0.5)
            _, output = next(iter(query.request("Outputs")["Outputs"].items()))
            scale = output["logical"]["scale"]
            width = output["logical"]["width"]
            if keep:
                Path(keep).mkdir(parents=True, exist_ok=True)

            for name in VIEWS:
                enters, leaves = count("keyboard_enter"), count("keyboard_leave")
                subprocess.run([binary, "--launcher-view", name], env=env, timeout=10, check=True)
                try:
                    wait_for(lambda: "dojo-launcher" in layers(), f"{name}: launcher mapped")
                    wait_for(lambda: count("keyboard_leave") > leaves, f"{name}: launcher takes the keyboard")
                    opened = True
                except AssertionError:
                    opened = False
                time.sleep(0.8)  # the reveal, and the view's first rows
                shot = screenshot(query, str(directory / f"{name}.png"), show_pointer=False)
                drawn = panel_pixels(shot, scale, width) > 0
                if keep:
                    shutil.copy(shot, Path(keep) / f"{name}.png")
                key("a")
                time.sleep(0.4)
                shot2 = screenshot(query, str(directory / f"{name}-typed.png"), show_pointer=False)
                if keep:
                    shutil.copy(shot2, Path(keep) / f"{name}-typed.png")
                key("Escape")  # leaves the view for the root list
                time.sleep(0.4)
                still_open = "dojo-launcher" in layers()
                key("Escape")  # closes the launcher
                try:
                    wait_for(lambda: count("keyboard_enter") > enters, f"{name}: keyboard returns")
                    closed = True
                except AssertionError:
                    closed = False
                check(f"{name}: opens, draws, accepts typing, Escape backs out and Escape closes",
                      opened and drawn and still_open and closed,
                      f"opened={opened} drawn={drawn} root_after_first_escape={still_open} closed={closed}")
                check(f"{name}: the shell is still running", shell.poll() is None)
                time.sleep(0.3)
            logged = (directory / "shell.log").read_text()
            check("the shell logged no panic", "panicked" not in logged,
                  next((line for line in logged.splitlines() if "panicked" in line), ""))
    finally:
        if shell is not None:
            subprocess.run([binary, "--quit"], env=env, timeout=5, check=False)
            try:
                shell.wait(timeout=5)
            except subprocess.TimeoutExpired:
                shell.kill()
        if recorder is not None:
            recorder.terminate()
        query.close()
    Path(args.log).write_text(json.dumps(checks, indent=2))
    failed = [c for c in checks if not c["ok"]]
    print(f"{len(checks) - len(failed)} of {len(checks)} checks passed")
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()
