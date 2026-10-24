#!/usr/bin/env python3
"""Check that the launcher's hidden layer surface releases and retakes the keyboard.

Runs in the nested Niri that run-nested.sh started, never the live session. Needs `wtype` on PATH
(or DOJO_WTYPE) and the agent-input fixture, whose two ordinary windows record keyboard focus.

    DOJO_NESTED_AGENT_INPUT=0 nix shell nixpkgs#wtype -c devenv shell -- \\
        tools/agent-input/run-nested.sh --launcher-reuse

Per cycle: open the launcher, see the fixture lose the keyboard, press Escape (which only works
if the launcher holds the keyboard), see the layer surface unmap and the fixture get the keyboard
back, then type into the fixture to prove it. Finally the output scale changes while the launcher
is hidden and the next opening must still draw its panel.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from run import Peer, events, screenshot, wait_for

PANEL_COLOUR = ("30", "30", "46")
TOP_BAR_HEIGHT = 24


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True)
    parser.add_argument("--cycles", type=int, default=4)
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
    query = Peer(os.environ["NIRI_SOCKET"])
    binary = os.environ.get("DOJO_BINARY", str(root / "target/debug/dojo"))
    fixture = os.environ.get("DOJO_FIXTURE", str(Path(__file__).parent / "target/debug/dojo-agent-input-fixture"))
    wtype = os.environ.get("DOJO_WTYPE", "wtype")
    pixel_diff = os.environ.get("DOJO_PIXEL_DIFF_BINARY", str(Path(__file__).parent / "target/debug/pixel-diff"))
    fixture_log = directory / "fixture.jsonl"
    checks = []

    def check(name, ok, detail=""):
        checks.append({"check": name, "ok": bool(ok), "detail": detail})
        print(("PASS " if ok else "FAIL ") + name + (f" ({detail})" if detail else ""), flush=True)

    def layers():
        return {item["namespace"]: item for item in query.request("Layers")["Layers"]}

    def fixture_events():
        return events(fixture_log)

    def count(kind, window=None):
        return sum(1 for e in fixture_events() if e.get("event") == kind and (window is None or e.get("window") == window))

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
            # The shell's opening overlay holds the keyboard until its greeting ends.
            wait_for(lambda: not any(n.startswith("dojo-session") for n in layers()), "opening overlay gone")
            recorder = subprocess.Popen([fixture], env=env, stdout=recorder_log)
            wait_for(lambda: count("keyboard_enter") >= 1, "fixture keyboard focus")
            time.sleep(0.5)
            output_name, output = next(iter(query.request("Outputs")["Outputs"].items()))
            scale = output["logical"]["scale"]
            width = output["logical"]["width"]

            for cycle in range(args.cycles):
                enters, leaves = count("keyboard_enter"), count("keyboard_leave")
                subprocess.run([binary, "--launcher"], env=env, timeout=10, check=True)
                wait_for(lambda: "dojo-launcher" in layers(), f"cycle {cycle}: launcher mapped")
                try:
                    wait_for(lambda: count("keyboard_leave") > leaves, f"cycle {cycle}: fixture loses the keyboard")
                except AssertionError:
                    path = screenshot(query, str(directory / f"failed-{cycle}.png"), show_pointer=False)
                    print("DEBUG panel pixels", panel_pixels(path, scale, width), "layers", layers().get("dojo-launcher"), flush=True)
                    raise
                interactivity = layers()["dojo-launcher"]["keyboard_interactivity"]
                layer = layers()["dojo-launcher"]["layer"]
                time.sleep(0.4)  # let the reveal finish and the launcher take keyboard focus
                key("Escape")
                wait_for(lambda: count("keyboard_enter") > enters, f"cycle {cycle}: fixture gets the keyboard back")
                still_listed = "dojo-launcher" in layers()
                before = count("key")
                key("a")
                try:
                    wait_for(lambda: count("key") > before, f"cycle {cycle}: typing reaches the fixture")
                    typed = True
                except AssertionError:
                    typed = False
                check(f"cycle {cycle}: open takes the keyboard, Escape unmaps and returns it, typing reaches the fixture",
                      typed and layer == "Overlay", f"layer={layer}, keyboard_interactivity={interactivity}; unmapped layer still listed by Niri: {still_listed}")
                time.sleep(0.3)

            # The same, closing through the toggle command, and the shell must still be one process.
            enters = count("keyboard_enter")
            leaves = count("keyboard_leave")
            subprocess.run([binary, "--launcher"], env=env, timeout=10, check=True)
            wait_for(lambda: count("keyboard_leave") > leaves, "toggle open")
            time.sleep(0.4)
            subprocess.run([binary, "--launcher"], env=env, timeout=10, check=True)
            wait_for(lambda: count("keyboard_enter") > enters, "keyboard back after toggle close")
            check("toggle closes by unmapping and returns the keyboard", True)

            # Output scale changes while the surface is unmapped.
            for new_scale in (1.5, 2.0, scale):
                query.request({"Output": {"output": output_name, "action": {"Scale": {"scale": {"Specific": float(new_scale)}}}}})
                time.sleep(0.5)
                logical = query.request("Outputs")["Outputs"][output_name]["logical"]
                leaves = count("keyboard_leave")
                enters = count("keyboard_enter")
                subprocess.run([binary, "--launcher"], env=env, timeout=10, check=True)
                wait_for(lambda: count("keyboard_leave") > leaves, f"scale {new_scale}: launcher mapped")
                time.sleep(0.5)
                path = screenshot(query, str(directory / f"scale-{new_scale}.png"), show_pointer=False)
                matching = panel_pixels(path, logical["scale"], logical["width"])
                full = (round(4 * logical["scale"]) * 2) ** 2
                check(f"panel drawn after output scale changed to {new_scale} while hidden",
                      matching >= 0.9 * full, f"{matching}/{full} panel pixels")
                os.unlink(path)
                key("Escape")
                wait_for(lambda: count("keyboard_enter") > enters, f"scale {new_scale}: unmapped")
                time.sleep(0.3)
            check("shell process survived every cycle", shell.poll() is None)
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
