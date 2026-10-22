#!/usr/bin/env python3
"""Frames of the shell's login opening in a nested Niri, with and without the greeter's marker.

The login screen fades to the frame colour and the display manager starts a session, so the shell
starts under a compositor whose background is already that colour. This runs the real shell in a
nested Niri with the theme's frame colour as its `background-color`, captures the screen from before
the shell starts, and reports what the first frames contain. With `--entry after-greeter` the
display manager's marker `$XDG_RUNTIME_DIR/dojo/after-greeter` exists, as `dojo-dm-pam` leaves it.

Never touches the live session: a private runtime directory, home, buses and a stand-in logind.

    devenv shell -- cargo build --bin dojo --example fake_logind
    NIRI_BINARY=... devenv shell -- python3 tools/agent-input/run-login-opening.py --out DIR \\
        [--entry after-greeter|cold] [--slow-wallpaper-ms N]

`--slow-wallpaper-ms` replaces the lock wallpaper with a file the shell cannot decode before that
time: the wallpaper is a named pipe that a helper fills after the delay, so the opening has to wait
behind the frame colour instead of showing the closed state without it.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import threading
import time

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("handoff", HERE / "run-lock-handoff.py")
handoff = importlib.util.module_from_spec(spec)
spec.loader.exec_module(handoff)


class LoginRig(handoff.Rig):
    def start_shell(self):
        """The shell starts later, under the capture."""


def mean_and_spread(path):
    width, height, pixels = handoff.read_ppm(path)
    step = 3 * 5
    totals, count = [0, 0, 0], 0
    low, high = [255] * 3, [0] * 3
    for i in range(0, width * height * 3 - 2, step):
        for c in range(3):
            value = pixels[i + c]
            totals[c] += value
            low[c], high[c] = min(low[c], value), max(high[c], value)
        count += 1
    mean = tuple(round(t / count) for t in totals)
    return mean, max(h - l for h, l in zip(high, low))


def palette_background(theme):
    palette = json.loads((theme / "palette.json").read_text())
    value = palette["colors"]["background"].lstrip("#")
    return "#" + value, tuple(int(value[i:i + 2], 16) for i in (0, 2, 4))


def run(args):
    out = Path(args.out).resolve()
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    # The nested Niri gets the frame colour the way the real one does: as the layout background.
    real_theme = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local/state") / "dojo/themes/current"
    background_hex, background = palette_background(real_theme)
    config_dir = out / "nested-config"
    config_dir.mkdir()
    nested = (handoff.HERE / "nested.kdl").read_text()
    assert "layout {\n" in nested
    (config_dir / "nested.kdl").write_text(
        nested.replace("layout {\n", f'layout {{\n    background-color "{background_hex}"\n', 1))
    handoff.HERE = config_dir

    rig = LoginRig()
    ns = argparse.Namespace(out=str(out / "rig"), shell=args.shell, locker="", fake_logind=args.fake_logind,
                            lock_wallpaper=None, scale=args.scale, interval=args.interval,
                            shot_scale=args.shot_scale, after=0, settle=0, columns=args.columns,
                            keep_frames=True, wire=False, vk_driver_files=args.vk_driver_files)
    result = {"entry": args.entry, "niri_background": background_hex}
    try:
        rig.start(ns)
        theme = rig.state / "dojo/themes/current"
        pipe_writer = None
        if args.slow_wallpaper_ms:
            real = theme / "lock-wallpaper.png"
            data = real.read_bytes()
            real.unlink()
            os.mkfifo(real)

            def fill():
                time.sleep(args.slow_wallpaper_ms / 1000)
                with open(real, "wb") as handle:
                    handle.write(data)
            pipe_writer = threading.Thread(target=fill, daemon=True)
        marker_dir = rig.runtime / "dojo"
        marker_dir.mkdir(exist_ok=True)
        if args.entry == "after-greeter":
            (marker_dir / "after-greeter").write_text("")
            os.chmod(marker_dir / "after-greeter", 0o600)
        frames = rig.out / "frames"
        frames.mkdir(exist_ok=True)
        capture = handoff.Capturer(rig.grim, rig.grim_env, frames, "login", args.interval, args.shot_scale)
        time.sleep(0.5)
        spawned = time.time()
        shell_env = rig.client_env()
        shell_log = rig.out / "shell.log"
        process = subprocess.Popen([args.shell], env=shell_env, stdout=subprocess.DEVNULL, stderr=shell_log.open("w"))
        rig.processes.append(process)
        if pipe_writer:
            pipe_writer.start()
        time.sleep(args.capture_seconds)
        kept = capture.finish()
        text = shell_log.read_text(errors="replace")
        result["startup"] = re.findall(r"dojo-startup: event=(login[\w-]*|start-up-queued|gpui-running) ms=(\d+)", text)
        result["marker_consumed"] = not (marker_dir / "after-greeter").exists()
        rows = []
        for written, path, started in kept:
            offset = round((written - spawned) * 1000)
            named = frames / f"login-{offset:+05}.ppm"
            path.rename(named)
            mean, spread = mean_and_spread(named)
            rows.append((offset, named, mean, spread))
        rows.sort()
        result["niri_background_rgb"] = background
        table, previous = [], None
        for offset, path, mean, spread in rows:
            if previous is None or sum(abs(a - b) for a, b in zip(mean, previous)) > 2:
                table.append({"offset_ms": offset, "mean_rgb": mean, "spread": spread})
            previous = mean
        result["distinct_frames"] = table
        shown = [row for row in rows if row[0] >= 0]
        sheet = [(f"{offset:+d}", path) for offset, path, _, _ in shown][::args.sheet_every]
        if sheet:
            handoff.write_sheet(sheet, out / "sheet.png", args.columns)
            result["sheet"] = str(out / "sheet.png")
        result["frames"] = len(rows)
    finally:
        rig.close()
    (out / "result.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))
    if not args.keep_frames:
        for path in (out / "rig" / "frames").glob("*.ppm"):
            path.unlink()


def main():
    root = HERE.parents[1]
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--entry", choices=("after-greeter", "cold"), default="after-greeter")
    parser.add_argument("--shell", default=str(root / "target/debug/dojo"))
    parser.add_argument("--fake-logind", default=str(root / "target/debug/examples/fake_logind"))
    parser.add_argument("--scale", default="1")
    parser.add_argument("--interval", type=int, default=40)
    parser.add_argument("--shot-scale", default="0.25")
    parser.add_argument("--capture-seconds", type=float, default=6.0)
    parser.add_argument("--slow-wallpaper-ms", type=int, default=0)
    parser.add_argument("--sheet-every", type=int, default=1, help="keep every Nth frame in the contact sheet")
    parser.add_argument("--columns", type=int, default=6)
    parser.add_argument("--keep-frames", action="store_true")
    parser.add_argument("--vk-driver-files", default=next(
        (str(p) for p in (Path("/run/opengl-driver/share/vulkan/icd.d/radeon_icd.x86_64.json"),
                          Path("/usr/share/vulkan/icd.d/radeon_icd.x86_64.json")) if p.exists()), None))
    args = parser.parse_args()
    assert os.environ.get("WAYLAND_DISPLAY") and os.environ.get("XDG_RUNTIME_DIR"), "needs a host Wayland session to nest in"
    run(args)


if __name__ == "__main__":
    main()
