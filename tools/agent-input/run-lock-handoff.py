#!/usr/bin/env python3
"""Run the shell's lock closing, the locker takeover and the unlock in a nested Niri, with frames.

Never touches the live session. Everything runs against a nested Niri with a private runtime
directory, home, session bus and a stand-in system bus (a fake logind). Build first:

    devenv shell -- cargo build --bin dojo --bin dojo-lock --bin dojo-lock-pam \\
        --example fake_logind --features fake-pam-stack,record-session-actions
    devenv shell -- python3 tools/agent-input/run-lock-handoff.py --out DIR

The shell is a `record-session-actions` build, so `dojo --leave lock` records `start-locker`
instead of calling systemctl; this script starts the `fake-pam-stack` locker when that line
appears, as `systemctl --user start dojo-lock.service` would, and ignores a second start while the
locker runs. Frames come from overlapping `grim` captures at a fixed cadence and are written as
contact sheets with the offset from the closing's start printed on every tile.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import zlib

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))
from run import Peer  # noqa: E402

spec = importlib.util.spec_from_file_location("run_lock", HERE / "run-lock.py")
run_lock = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run_lock)
wait_for = run_lock.wait_for

# The system daemon, which finds /etc/dbus-1/session.conf; devenv's own build does not.
DBUS_DAEMON = next(p for p in ("/usr/bin/dbus-daemon", "/usr/sbin/dbus-daemon") if Path(p).exists())

LATENCY = []

DIGITS = {
    "0": ["111", "101", "101", "101", "111"], "1": ["010", "110", "010", "010", "111"],
    "2": ["111", "001", "111", "100", "111"], "3": ["111", "001", "111", "001", "111"],
    "4": ["101", "101", "111", "001", "001"], "5": ["111", "100", "111", "001", "111"],
    "6": ["111", "100", "111", "101", "111"], "7": ["111", "001", "001", "001", "001"],
    "8": ["111", "101", "111", "101", "111"], "9": ["111", "101", "111", "001", "111"],
    "-": ["000", "000", "111", "000", "000"], " ": ["000"] * 5,
}


def read_ppm(path):
    data = Path(path).read_bytes()
    match = re.match(rb"P6\s+(\d+)\s+(\d+)\s+255\s", data)
    width, height = int(match.group(1)), int(match.group(2))
    return width, height, data[match.end():]


def label(rows, width, text, scale=3):
    """Burns `text` into the top-left corner of a tile held as a list of bytearray rows."""
    x0, y0 = 4, 4
    glyph_w = 4 * scale
    box_w = len(text) * glyph_w + 2 * scale
    for y in range(y0 - scale, y0 + 5 * scale + scale):
        for x in range(x0 - scale, min(width, x0 + box_w)):
            if 0 <= y < len(rows) and 0 <= x < width:
                rows[y][x * 3:x * 3 + 3] = b"\x00\x00\x00"
    for index, char in enumerate(text):
        for gy, line in enumerate(DIGITS.get(char, DIGITS[" "])):
            for gx, bit in enumerate(line):
                if bit == "1":
                    for dy in range(scale):
                        for dx in range(scale):
                            x = x0 + index * glyph_w + gx * scale + dx
                            y = y0 + gy * scale + dy
                            if x < width and y < len(rows):
                                rows[y][x * 3:x * 3 + 3] = b"\xff\xe0\x40"


def write_sheet(frames, path, columns):
    """frames: list of (label text, ppm path). Written as one PNG, tiles in order."""
    tiles = []
    for text, ppm in frames:
        width, height, pixels = read_ppm(ppm)
        rows = [bytearray(pixels[y * width * 3:(y + 1) * width * 3]) for y in range(height)]
        label(rows, width, text)
        tiles.append((width, height, rows))
    width, height = tiles[0][0], tiles[0][1]
    gap = 4
    line_count = (len(tiles) + columns - 1) // columns
    sheet_w = columns * width + (columns - 1) * gap
    body = bytearray()
    for line in range(line_count):
        group = tiles[line * columns:(line + 1) * columns]
        for y in range(height):
            row = bytearray()
            for index in range(columns):
                if index:
                    row += b"\x40\x40\x40" * gap
                row += group[index][2][y] if index < len(group) else b"\x20\x20\x20" * width
            body += b"\x00" + row
        if line != line_count - 1:
            body += (b"\x00" + b"\x40\x40\x40" * sheet_w) * gap
    sheet_h = line_count * height + (line_count - 1) * gap

    def chunk(kind, data):
        return len(data).to_bytes(4, "big") + kind + data + zlib.crc32(kind + data).to_bytes(4, "big")
    ihdr = sheet_w.to_bytes(4, "big") + sheet_h.to_bytes(4, "big") + bytes([8, 2, 0, 0, 0])
    Path(path).write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
                           + chunk(b"IDAT", zlib.compress(bytes(body), 3)) + chunk(b"IEND", b""))
    return sheet_w, sheet_h


class Capturer:
    """Starts a grim every `interval` ms until stopped. Captures overlap, so the cadence holds."""

    def __init__(self, grim, env, directory, prefix, interval_ms, scale):
        self.grim, self.env, self.directory = grim, env, directory
        self.prefix, self.interval, self.scale = prefix, interval_ms / 1000, scale
        self.shots = []
        self.stop_flag = threading.Event()
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def loop(self):
        next_at = time.time()
        while not self.stop_flag.is_set():
            path = self.directory / f"{self.prefix}-{len(self.shots):04}.ppm"
            started = time.time()
            process = subprocess.Popen([self.grim, "-t", "ppm", "-s", str(self.scale), str(path)],
                                       env=self.env, stderr=subprocess.DEVNULL)
            self.shots.append((started, path, process))
            next_at += self.interval
            time.sleep(max(0, next_at - time.time()))

    def finish(self):
        self.stop_flag.set()
        self.thread.join()
        kept = []
        for started, path, process in self.shots:
            if process.wait(timeout=30) == 0 and path.exists():
                kept.append((os.stat(path).st_mtime, path, started))
        return kept


def stamps(text, prefix, event):
    """(ms since the program's mark, unix_ms) of the first `prefix: event=<event>` line."""
    match = re.search(rf"{prefix}: event={re.escape(event)}\b.* ms=(\d+) unix_ms=(\d+)", text)
    return (int(match.group(1)), int(match.group(2))) if match else None


class Rig:
    def __init__(self):
        self.processes = []

    def start(self, args):
        self.args = args
        self.out = Path(args.out).resolve()
        shutil.rmtree(self.out, ignore_errors=True)
        self.out.mkdir(parents=True)
        for name in ("frames", "sheets"):
            (self.out / name).mkdir()
        # A socket path must stay short, so the private runtime directory lives under /tmp.
        self.runtime = Path(f"/tmp/dojo-handoff-{os.getpid()}")
        self.runtime.mkdir(mode=0o700)
        self.home = self.out / "home"
        self.grim = shutil.which("grim") or str(Path(subprocess.check_output(
            ["nix", "build", "--no-link", "--print-out-paths", "nixpkgs#grim"], text=True).strip()) / "bin/grim")
        self.prepare_home()
        self.prepare_stacks()
        self.start_buses()
        self.start_nested()
        self.start_shell()

    def spawn(self, argv, env, log_name):
        process = subprocess.Popen(argv, env=env, stdout=subprocess.DEVNULL,
                                   stderr=(self.out / log_name).open("w"))
        self.processes.append(process)
        return process

    def prepare_home(self):
        self.config = self.home / "config"
        self.state = self.home / "state"
        theme = self.state / "dojo/themes/current"
        theme.mkdir(parents=True)
        (self.config / "dojo").mkdir(parents=True)
        real_state = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local/state") / "dojo"
        real_config = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config") / "dojo/config.toml"
        for entry in (real_state / "themes/current").iterdir():
            if entry.is_file():
                shutil.copy(entry, theme / entry.name)
        if (real_state / "session").is_dir():
            shutil.copytree(real_state / "session", self.state / "dojo/session")
        shutil.copy(real_config, self.config / "dojo/config.toml")
        if self.args.lock_wallpaper == "flat":
            (theme / "lock-wallpaper.png").unlink(missing_ok=True)
        elif self.args.lock_wallpaper:
            shutil.copy(self.args.lock_wallpaper, theme / "lock-wallpaper.png")
        elif not (theme / "lock-wallpaper.png").exists():
            run_lock.stand_in_wallpaper(theme / "lock-wallpaper.png")
        for name in ("data", "cache"):
            (self.home / name).mkdir()

    def prepare_stacks(self):
        self.pam = self.out / "pam"
        self.confdir = self.pam / "pam.d"
        self.confdir.mkdir(parents=True)
        (self.pam / "tally").mkdir()
        for name, body in [("check_password.sh", run_lock.CHECK_PASSWORD), ("finger.sh", run_lock.FINGER)]:
            (self.pam / name).write_text(body)
            (self.pam / name).chmod(0o755)
        (self.pam / "finger.txt").write_text(run_lock.FINGER_HINT)
        (self.confdir / "dojo-lock-password").write_text(
            f"auth required pam_exec.so quiet expose_authtok {self.pam}/check_password.sh\n")
        (self.confdir / "dojo-lock-fingerprint").write_text(
            f"auth optional pam_echo.so file={self.pam}/finger.txt\n"
            f"auth required pam_exec.so quiet {self.pam}/finger.sh {self.pam}\n")

    def start_buses(self):
        base = {k: v for k, v in os.environ.items() if k in ("PATH", "HOME", "LANG")}
        self.session_bus = f"unix:path={self.runtime}/session-bus"
        self.system_bus = f"unix:path={self.runtime}/system-bus"
        for address in (self.session_bus, self.system_bus):
            process = subprocess.Popen([DBUS_DAEMON, "--session", "--nofork", f"--address={address}"],
                                       env=base, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.processes.append(process)
        wait_for(lambda: all((self.runtime / name).exists() for name in ("session-bus", "system-bus")), "buses")
        self.logind_log = self.out / "logind.log"
        env = dict(base, DOJO_TEST_SYSTEM_BUS=self.system_bus, FAKE_LOGIND_LOG=str(self.logind_log))
        self.spawn([str(self.args.fake_logind)], env, "fake-logind.log")
        time.sleep(0.5)

    def start_nested(self):
        config = (HERE / "nested.kdl").read_text() + f'\noutput "winit" {{\n    scale {self.args.scale}\n}}\n'
        (self.out / "nested.kdl").write_text(config)
        log_path = self.out / "niri.log"
        env = {k: v for k, v in os.environ.items() if k not in ("NIRI_SOCKET", "WAYLAND_SOCKET")}
        env.update({"DOJO_AGENT_INPUT": "0", "RUST_LOG": "niri=info"})
        binary = os.environ.get("NIRI_BINARY", str(ROOT / "vendor/niri/target/debug/niri"))
        self.niri = subprocess.Popen([binary, "--config", str(self.out / "nested.kdl")], env=env,
                                     stdout=log_path.open("w"), stderr=subprocess.STDOUT)
        self.processes.append(self.niri)

        def ready():
            text = log_path.read_text(errors="replace")
            display = re.findall(r"listening on Wayland socket: (\S+)", text)
            sockets = re.findall(r"IPC listening on: (\S+)", text)
            if self.niri.poll() is not None:
                raise AssertionError(f"niri exited:\n{text}")
            return (display[-1], sockets[-1]) if display and sockets and Path(sockets[-1]).is_socket() else None
        display, socket_path = wait_for(ready, "nested niri")
        host = os.environ["WAYLAND_DISPLAY"]
        environ = dict(item.split("=", 1) for item in
                       Path(f"/proc/{self.niri.pid}/environ").read_text().split("\0") if "=" in item)
        assert environ.get("WAYLAND_DISPLAY") == host and display != host, "not a nested compositor"
        runtime_of_niri = Path(os.environ["XDG_RUNTIME_DIR"])
        self.display_path = runtime_of_niri / display
        assert self.display_path.is_socket() and display != host
        self.niri_socket = socket_path
        self.grim_env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_SOCKET", "DISPLAY", "WAYLAND_DEBUG")}
        self.grim_env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": socket_path})
        time.sleep(1.0)

    def client_env(self, **extra):
        dropped = ("WAYLAND_SOCKET", "DISPLAY", "WAYLAND_DEBUG", "NIRI_SOCKET", "RUST_LOG", "XDG_SESSION_ID",
                   "DBUS_SESSION_BUS_ADDRESS", "DBUS_SYSTEM_BUS_ADDRESS", "XDG_CURRENT_DESKTOP")
        env = {k: v for k, v in os.environ.items() if k not in dropped}
        env.update({
            "HOME": str(self.home), "XDG_RUNTIME_DIR": str(self.runtime),
            "XDG_CONFIG_HOME": str(self.config), "XDG_STATE_HOME": str(self.state),
            "XDG_DATA_HOME": str(self.home / "data"), "XDG_CACHE_HOME": str(self.home / "cache"),
            "WAYLAND_DISPLAY": str(self.display_path), "NIRI_SOCKET": self.niri_socket,
            "DBUS_SESSION_BUS_ADDRESS": self.session_bus, "DBUS_SYSTEM_BUS_ADDRESS": self.system_bus,
            "DOJO_TEST_SYSTEM_BUS": self.system_bus, "FAKE_LOGIND_LOG": str(self.logind_log),
            "DOJO_SESSION_ACTION_LOG": str(self.out / "actions.log"),
        })
        if self.args.vk_driver_files:
            env["VK_DRIVER_FILES"] = self.args.vk_driver_files
        env.update(extra)
        assert env["WAYLAND_DISPLAY"] != os.environ["WAYLAND_DISPLAY"]
        return env

    def start_shell(self):
        self.shell = self.spawn([str(self.args.shell)], self.client_env(), "shell.log")
        wait_for(lambda: (self.runtime / "dojo.sock").exists() or self.shell.poll() is not None, "control socket")
        assert self.shell.poll() is None, (self.out / "shell.log").read_text()
        time.sleep(self.args.settle)

    def layers(self):
        peer = Peer(self.niri_socket)
        try:
            return peer.request("Layers")["Layers"]
        finally:
            peer.close()

    def dojo_layers(self):
        return sorted(layer["namespace"] for layer in self.layers() if layer["namespace"].startswith("dojo-session"))

    def outputs(self):
        peer = Peer(self.niri_socket)
        try:
            return peer.request("Outputs")["Outputs"]
        finally:
            peer.close()

    def leave_lock(self):
        started = time.time()
        subprocess.run([str(self.args.shell), "--leave", "lock"], env=self.client_env(), check=True,
                       stdout=subprocess.DEVNULL)
        return started

    def locker_launcher(self):
        """Starts the locker when the shell records `start-locker`, once while it runs."""
        actions = self.out / "actions.log"
        self.locker = None
        self.locker_starts = []
        stop = threading.Event()

        def watch():
            seen = 0
            while not stop.is_set():
                lines = actions.read_text().split() if actions.exists() else []
                for _ in lines[seen:]:
                    now = time.time()
                    if self.locker is not None and self.locker.poll() is None:
                        self.locker_starts.append((now, "ignored: already running"))
                        continue
                    index = len([s for s in self.locker_starts if s[1] == "started"])
                    log = (self.out / f"locker-{index}.log").open("w")
                    extra = {"WAYLAND_DEBUG": "client"} if self.args.wire else {}
                    self.locker = subprocess.Popen(
                        [str(self.args.locker), "--pam-confdir", str(self.confdir)],
                        env=self.client_env(**extra), stdout=subprocess.DEVNULL, stderr=log)
                    self.locker_starts.append((time.time(), "started"))
                seen = len(lines)
                time.sleep(0.001)
        thread = threading.Thread(target=watch, daemon=True)
        thread.start()
        return stop, thread

    def close(self):
        for process in reversed(self.processes):
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
        for process in self.processes:
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
        shutil.rmtree(self.runtime, ignore_errors=True)


def table(kept, anchor_ms, directory, prefix):
    """Names each capture by the offset of its file's last write, which follows the compositor's copy.

    The spawn time is not used: a capture started at t may copy the output a good deal later."""
    rows = []
    for index, (written, path, started) in enumerate(kept):
        offset = round(written * 1000 - anchor_ms)
        named = directory / f"{prefix}-{index:04}-{offset:+05}.ppm"
        path.rename(named)
        rows.append((offset, named))
    LATENCY.extend(written - started for written, _, started in kept)
    return rows


def run(args):
    rig = Rig()
    result = {"scale": args.scale}
    try:
        rig.start(args)
        outputs = rig.outputs()
        result["outputs"] = {name: output["logical"] for name, output in outputs.items()}
        result["layers_before"] = rig.dojo_layers()
        stop_launch, launch_thread = rig.locker_launcher()
        frames = rig.out / "frames"

        capture = Capturer(rig.grim, rig.grim_env, frames, "lock", args.interval, args.shot_scale)
        time.sleep(0.4)
        leave_at = rig.leave_lock()
        time.sleep(0.12)
        result["layers_during_closing"] = [
            {key: layer[key] for key in ("namespace", "layer", "output") if key in layer} for layer in rig.layers()]
        wait_for(lambda: rig.locker is not None and "event=locked" in (rig.out / "locker-0.log").read_text(),
                 "locker locked", timeout=20)
        shell_log = lambda: (rig.out / "shell.log").read_text(errors="replace")
        wait_for(lambda: "event=done" in shell_log(), "shell closing done", timeout=20)
        locked_ms = stamps((rig.out / "locker-0.log").read_text(), "dojo-lock", "locked")
        time.sleep(args.after / 1000 + 0.3)
        result["layers_after_lock"] = rig.dojo_layers()
        kept = capture.finish()

        text = shell_log()
        begin = stamps(text, "dojo-session", "closing-begin occasion=lock") or stamps(text, "dojo-session", "closing-begin")
        anchor = begin[1]
        timeline = {}
        for event in ("closing-begin", "locker-start", "marker-seen", "hint-seen", "lock-confirmed",
                      "overlay-dismissed", "done"):
            found = re.search(rf"dojo-session: event={event}\b.* ms=(\d+) unix_ms=(\d+)", text)
            if found:
                timeline[event] = {"ms_after_closing_begin": int(found.group(1)), "unix_ms": int(found.group(2)) - anchor}
        locker_text = (rig.out / "locker-0.log").read_text()
        for event in ("first-render", "window-opened", "locked", "exit"):
            found = stamps(locker_text, "dojo-lock", f"{event}")
            if found:
                timeline[f"locker {event}"] = {"locker_ms": found[0], "unix_ms": found[1] - anchor}
        timeline["leave command sent"] = {"unix_ms": round(leave_at * 1000 - anchor)}
        timeline["locker process spawned"] = {"unix_ms": round(rig.locker_starts[0][0] * 1000 - anchor)}
        result["timeline_ms_from_closing_begin"] = timeline
        if args.wire:
            wire = re.findall(r"\[[\d:.]+\] .*(?:ext_session_lock_surface_v1#\d+\.configure|create_immed|set_destination|"
                              r"wl_surface#\d+\.commit|ack_configure|first-render|set_buffer_scale).*", locker_text)
            result["wire_first_lines"] = wire[:24]
        result["first_render_line"] = re.findall(r"dojo-lock: event=first-render.*", locker_text)
        result["locker_starts"] = [(round(t * 1000 - anchor), what) for t, what in rig.locker_starts]
        rows = table(kept, anchor, frames, "lock")
        result["lock_frames"] = len(rows)
        offsets = [o for o, _ in rows]
        result["lock_frame_gaps_ms"] = {"median": sorted(b - a for a, b in zip(offsets, offsets[1:]))[len(offsets) // 2],
                                        "max": max(b - a for a, b in zip(offsets, offsets[1:]))}
        marker = rig.runtime / "dojo/locked-at"
        result["marker_exists_when_locked"] = marker.exists()
        result["handoff_file_left_behind"] = (rig.runtime / "dojo/lock-handoff").exists()
        result["logind_log_after_lock"] = rig.logind_log.read_text().split() if rig.logind_log.exists() else []
        result["actions"] = (rig.out / "actions.log").read_text().split()
        result["farewell_stored"] = (rig.state / "dojo/session/lock").read_text().strip()

        # Contact sheets of the interesting window: from 150 ms before the locker start to 700 ms after the lock.
        lo = timeline["locker-start"]["unix_ms"] - 150 if "locker-start" in timeline else 0
        hi = timeline["locker locked"]["unix_ms"] + 700
        picked = [(f"{o:+d}", p) for o, p in rows if lo <= o <= hi]
        early = [(f"{o:+d}", p) for o, p in rows if o < lo]
        result["sheets"] = {}
        for name, group in (("lock-0-closing", [x for x in rows if x[0] < lo + 150]), ("lock-1-handoff", [x for x in rows if lo <= x[0] <= hi])):
            group = [(f"{o:+d}", p) for o, p in group]
            if group:
                path = rig.out / "sheets" / f"{name}.png"
                result["sheets"][name] = {"path": str(path), "frames": len(group),
                                          "size": write_sheet(group, path, args.columns)}

        # Unlock: the fingerprint stand-in succeeds; frames from the touch until well after the locker is gone.
        capture = Capturer(rig.grim, rig.grim_env, frames, "unlock", args.interval, args.shot_scale)
        time.sleep(0.4)
        touched = time.time()
        (rig.pam / "finger.trigger").write_text("")
        code = rig.locker.wait(timeout=20)
        exited = time.time()
        layer_checks = {}
        for wait_ms in (100, 500, 1500):
            time.sleep(max(0, exited + wait_ms / 1000 - time.time()))
            layer_checks[f"+{wait_ms}ms after locker exit"] = rig.dojo_layers()
        time.sleep(max(0, exited + args.after / 1000 + 1.0 - time.time()))
        kept = capture.finish()
        rows = table(kept, touched * 1000, frames, "unlock")
        result["unlock"] = {"locker_exit_code": code, "locker_exit_after_touch_ms": round((exited - touched) * 1000),
                            "dojo_session_layers": layer_checks,
                            "marker_exists_after": marker.exists(),
                            "logind_log": rig.logind_log.read_text().split(),
                            "shell_new_closing_after_unlock": shell_log().count("closing-begin") > 1,
                            "shell_alive": rig.shell.poll() is None,
                            "frames": len(rows)}
        group = [(f"{o:+d}", p) for o, p in rows if o >= 600 and o <= 2400]
        if group:
            path = rig.out / "sheets" / "unlock.png"
            result["sheets"]["unlock"] = {"path": str(path), "frames": len(group), "size": write_sheet(group, path, args.columns)}
        stop_launch.set()
        launch_thread.join()
    finally:
        rig.close()
    result["grim_spawn_to_file_written_ms"] = {
        "median": round(sorted(LATENCY)[len(LATENCY) // 2] * 1000), "max": round(max(LATENCY) * 1000)}
    (rig.out / "result.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))
    if not args.keep_frames:
        for path in (rig.out / "frames").glob("*.ppm"):
            path.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--shell", default=str(ROOT / "target/debug/dojo"))
    parser.add_argument("--locker", default=str(ROOT / "target/debug/dojo-lock"))
    parser.add_argument("--fake-logind", default=str(ROOT / "target/debug/examples/fake_logind"))
    parser.add_argument("--lock-wallpaper", help="A lock-wallpaper.png to use, or 'flat' for none. Default: the theme's, else a stand-in gradient")
    parser.add_argument("--scale", default="1.75", help="Output scale of the nested Niri")
    parser.add_argument("--interval", type=int, default=30, help="Milliseconds between capture starts")
    parser.add_argument("--shot-scale", default="0.3", help="grim -s factor, to keep each capture short")
    parser.add_argument("--after", type=int, default=700, help="Milliseconds to keep capturing after the lock")
    parser.add_argument("--settle", type=float, default=4.0, help="Seconds to let the shell start before the lock")
    parser.add_argument("--columns", type=int, default=4)
    parser.add_argument("--keep-frames", action="store_true")
    parser.add_argument("--wire", action="store_true", help="WAYLAND_DEBUG=client on the locker, first wire lines in the result")
    parser.add_argument("--vk-driver-files", default=next(
        (str(p) for p in (Path("/run/opengl-driver/share/vulkan/icd.d/radeon_icd.x86_64.json"),
                          Path("/usr/share/vulkan/icd.d/radeon_icd.x86_64.json")) if p.exists()), None))
    args = parser.parse_args()
    assert os.environ.get("WAYLAND_DISPLAY") and os.environ.get("XDG_RUNTIME_DIR"), "needs a host Wayland session to nest in"
    run(args)


if __name__ == "__main__":
    main()
