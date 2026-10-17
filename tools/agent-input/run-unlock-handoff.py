#!/usr/bin/env python3
"""The shell's unlock opening in a nested Niri: lock, park, unlock, with frames.

Same rig as run-lock-handoff.py (private runtime directory, buses, fake logind, fake PAM stacks, a
`record-session-actions` shell and locker). Two fixture windows stand in for the live desktop. The run locks,
checks that the shell parked one overlay window per output and counts frames while locked, unlocks with the
fingerprint stand-in, and captures the opening with grim. The captures are dated against the shell's own
`dojo-trace: unlock-frame` lines (the same method as morph-frames.py) and written as a contact sheet. The
shell is told to hold its finished opening on screen (`DOJO_SESSION_HOLD_UNLOCK_END_MS`) so the last overlay
frame can be compared with a capture of the resting desktop.

    devenv shell -- cargo build --bin dojo --bin dojo-lock --bin dojo-lock-pam \\
        --example fake_logind --features fake-pam-stack,record-session-actions
    devenv shell -- cargo build --manifest-path tools/agent-input/Cargo.toml
    devenv shell -- python3 tools/agent-input/run-unlock-handoff.py --out DIR
"""
import argparse
import importlib.util
import json
import os
import re
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
spec = importlib.util.spec_from_file_location("lock_handoff", HERE / "run-lock-handoff.py")
lock_handoff = importlib.util.module_from_spec(spec)
sys.modules["lock_handoff"] = lock_handoff
spec.loader.exec_module(lock_handoff)
wait_for = lock_handoff.wait_for
read_ppm, write_sheet = lock_handoff.read_ppm, lock_handoff.write_sheet

UNLOCK = re.compile(r"dojo-trace: unlock-frame since_begin_ms=(\d+) progress=([\d.]+) cover=([\d.]+) frame=([\d.]+) "
                    r"open_w=([\d.]+) open_h=([\d.]+) radius=([\d.]+) wallpaper=([\d.]+) surround=([\d.]+) "
                    r"clock_px=([\d.]+) has_wallpaper=(\w+) unix_ms=(\d+)")
EDGE = 5


def unlock_frames(text):
    return [dict(progress=float(m[2]), cover=float(m[3]), frame=float(m[4]), width=float(m[5]), height=float(m[6]),
                 radius=float(m[7]), wallpaper=float(m[8]), surround=float(m[9]), clock=float(m[10]), unix=int(m[12]))
            for m in UNLOCK.finditer(text)]


def measure(path):
    """(left, right) of the opening on the middle row: the first colour change from each edge."""
    width, height, pixels = read_ppm(path)
    y = height // 2
    row = [pixels[(y * width + x) * 3:(y * width + x) * 3 + 3] for x in range(width)]
    far = lambda a, b: max(abs(a[i] - b[i]) for i in range(3)) > EDGE
    left = next((x for x in range(3, width) if far(row[x], row[2])), None)
    right = next((x for x in range(width - 4, 0, -1) if far(row[x], row[width - 3])), None)
    return width, left, right


def difference(first, second):
    """Mean absolute channel difference, the largest, the share of pixels over 2, and the box they lie in."""
    (w, h, a), (_, _, b) = read_ppm(first), read_ppm(second)
    total, largest, over, box = 0, 0, 0, [w, h, -1, -1]
    for y in range(h):
        row = slice(y * w * 3, (y + 1) * w * 3)
        ra, rb = a[row], b[row]
        if ra == rb:
            continue
        for x in range(w):
            d = max(abs(ra[x * 3 + k] - rb[x * 3 + k]) for k in range(3))
            total += sum(abs(ra[x * 3 + k] - rb[x * 3 + k]) for k in range(3))
            largest = max(largest, d)
            if d > 2:
                over += 1
                box = [min(box[0], x), min(box[1], y), max(box[2], x), max(box[3], y)]
    return dict(mean_abs=round(total / (w * h * 3), 4), max=largest, share_over_2=round(over / (w * h), 5),
                box_xyxy=box if over else None, size=[w, h])


def mean_difference(first, second):
    """Fast mean absolute difference of two same-size captures."""
    (w, h, a), (_, _, b) = read_ppm(first), read_ppm(second)
    return sum(abs(x - y) for x, y in zip(a, b)) / len(a)


def stamp(text, event):
    m = re.search(rf"dojo-session: event={re.escape(event)}\b.*? unix_ms=(\d+)", text)
    return int(m.group(1)) if m else None


def locker_stamp(text, event):
    m = re.search(rf"dojo-lock: event={re.escape(event)}\b.* unix_ms=(\d+)", text)
    return int(m.group(1)) if m else None


def baseline(rig, args, result):
    """Lock and unlock with the transition off (no overlay at all) and compare the desktop before and after."""
    frames = rig.out / "frames"
    grab = lambda name: subprocess.run([rig.grim, "-t", "ppm", "-s", "1", str(frames / name)], env=rig.grim_env, check=True)
    grab("before.ppm")
    stop_launch, launch_thread = rig.locker_launcher()
    rig.leave_lock()
    wait_for(lambda: rig.locker is not None and "event=locked" in (rig.out / "locker-0.log").read_text(), "locked", timeout=20)
    time.sleep(1.0)
    (rig.pam / "finger.trigger").write_text("")
    rig.locker.wait(timeout=20)
    time.sleep(8.0)
    grab("after.ppm")
    result["baseline_no_overlay_before_vs_after"] = difference(frames / "before.ppm", frames / "after.ppm")
    stop_launch.set()
    launch_thread.join()
    print(json.dumps(result, indent=2))


def run(args):
    os.environ["DOJO_SESSION_HOLD_UNLOCK_END_MS"] = str(args.hold)
    rig = lock_handoff.Rig()
    result = {}
    if args.baseline:
        original = rig.prepare_home

        def plain_home():
            original()
            with (rig.config / "dojo/config.toml").open("a") as config:
                config.write("\n[session]\ntransition = false\n")
        rig.prepare_home = plain_home
    try:
        rig.start(args)
        out = rig.out
        frames = out / "frames"
        outputs = rig.outputs()
        result["outputs"] = {name: o["logical"] for name, o in outputs.items()}
        logical_width = next(iter(result["outputs"].values()))["width"]
        fixtures = [subprocess.Popen([str(args.fixture)], env=rig.client_env(), stdout=subprocess.DEVNULL,
                                     stderr=(out / "fixture.log").open("w"))]
        rig.processes.extend(fixtures)
        time.sleep(2.0)
        shell_log = lambda: (out / "shell.log").read_text(errors="replace")

        if args.baseline:
            return baseline(rig, args, result)
        rest = frames / "resting.ppm"
        subprocess.run([rig.grim, "-t", "ppm", "-s", "1", str(rest)], env=rig.grim_env, check=True)
        result["ready_file_before_lock"] = (rig.runtime / "dojo/overlay-ready").exists()

        stop_launch, launch_thread = rig.locker_launcher()
        rig.leave_lock()
        wait_for(lambda: rig.locker is not None and "event=locked" in (out / "locker-0.log").read_text(), "locked", timeout=20)
        wait_for(lambda: "overlay-parked" in shell_log(), "parked", timeout=20)
        time.sleep(1.0)
        ready = rig.runtime / "dojo/overlay-ready"
        result["ready_file"] = ready.read_text().strip() if ready.exists() else None
        result["shell_pid"] = rig.shell.pid
        result["layers_while_locked"] = rig.dojo_layers()
        result["outputs_count"] = len(outputs)

        # Idle: no frame from the parked overlay and none from the locker while nothing changes.
        time.sleep(2.0)
        mark_shell, mark_locker = len(shell_log()), len((out / "locker-0.log").read_text())
        ctx = lambda pid: sum(int(l.split()[1]) for l in Path(f"/proc/{pid}/status").read_text().splitlines()
                              if l.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")))
        task_ctx = lambda pid: {t.name: sum(int(l.split()[1]) for l in (t / "status").read_text().splitlines()
                                            if l.startswith(("voluntary_ctxt_switches", "nonvoluntary_ctxt_switches")))
                                for t in Path(f"/proc/{pid}/task").iterdir()}
        before = task_ctx(rig.shell.pid)
        time.sleep(args.idle)
        after = task_ctx(rig.shell.pid)
        shell_idle = shell_log()[mark_shell:]
        locker_idle = (out / "locker-0.log").read_text()[mark_locker:]
        result["idle_seconds"] = args.idle
        result["idle_shell_parked_frames"] = shell_idle.count("dojo-trace: parked-frame")
        result["idle_shell_other_trace_lines"] = shell_idle.count("dojo-trace:") - shell_idle.count("dojo-trace: parked-frame")
        result["idle_locker_frames"] = locker_idle.count("dojo-lock-trace: frame")
        result["idle_shell_context_switches"] = sum(after[t] - before.get(t, 0) for t in after)

        locker_log = "locker-0.log"
        if args.restart_locker:
            rig.locker.kill()
            rig.locker.wait()
            time.sleep(0.5)
            rig.locker_starts.append((time.time(), "restarted by the runner"))
            rig.locker = subprocess.Popen([str(args.locker), "--pam-confdir", str(rig.confdir)], env=rig.client_env(),
                                          stdout=subprocess.DEVNULL, stderr=(out / "locker-1.log").open("w"))
            locker_log = "locker-1.log"
            wait_for(lambda: "event=locked" in (out / locker_log).read_text(), "relocked", timeout=20)
            time.sleep(1.0)
            result["after_locker_restart"] = {"layers": rig.dojo_layers(), "ready_file": ready.exists(),
                                              "marker": (rig.runtime / "dojo/locked-at").exists(),
                                              "shell_alive": rig.shell.poll() is None}
        # Unlock.
        capture = lock_handoff.Capturer(rig.grim, rig.grim_env, frames, "unlock", args.interval, args.shot_scale)
        time.sleep(0.3)
        sampler_stop, samples = threading.Event(), []

        def sample():
            while not sampler_stop.is_set():
                try:
                    samples.append((round(time.time() * 1000), len(rig.dojo_layers())))
                except Exception:
                    pass
                time.sleep(0.05)
        threading.Thread(target=sample, daemon=True).start()
        touched = time.time()
        (rig.pam / "finger.trigger").write_text("")
        code = rig.locker.wait(timeout=20)
        wait_for(lambda: "unlock-end-held" in shell_log() or "unlock-done" in shell_log(), "opening end", timeout=10)
        result["layers_at_end_before_grab"] = rig.layers()
        held = frames / "held-end.ppm"
        sleeper = threading.Timer(3.0, lambda: subprocess.run([rig.grim, "-t", "ppm", "-s", "1", str(frames / "held-late.ppm")], env=rig.grim_env))
        sleeper.start()
        held_small = frames / "held-end-small.ppm"
        subprocess.run([rig.grim, "-t", "ppm", "-s", "1", str(held)], env=rig.grim_env, check=True)
        subprocess.run([rig.grim, "-t", "ppm", "-s", args.shot_scale, str(held_small)], env=rig.grim_env, check=True)
        result["layers_at_end_after_grab"] = [l["namespace"] for l in rig.layers()]
        kept = capture.finish()
        wait_for(lambda: "unlock-dismissed" in shell_log() or "unlock-done" in shell_log(), "dismissal", timeout=20)
        time.sleep(1.0)
        after_rest = frames / "resting-after.ppm"
        subprocess.run([rig.grim, "-t", "ppm", "-s", "1", str(after_rest)], env=rig.grim_env, check=True)
        sampler_stop.set()
        changes, last = [], None
        for at, count in samples:
            if count != last:
                changes.append((at - round(touched * 1000), count))
                last = count
        result["overlay_layer_count_changes_ms_after_touch"] = changes
        result["layers_after"] = [l["namespace"] for l in rig.layers()]
        after_rest2 = frames / "resting-after2.ppm"
        time.sleep(1.5)
        subprocess.run([rig.grim, "-t", "ppm", "-s", "1", str(after_rest2)], env=rig.grim_env, check=True)
        result["resting_after_vs_resting_after_later"] = difference(after_rest, after_rest2)
        result["ready_file_after"] = ready.exists()
        result["marker_after"] = (rig.runtime / "dojo/locked-at").exists()
        result["locker_exit_code"] = code
        result["shell_alive"] = rig.shell.poll() is None

        text, locker_text = shell_log(), (out / locker_log).read_text()
        events = {
            "touch": round(touched * 1000),
            "locker unlocked": locker_stamp(locker_text, "unlocked"),
            "locker exit": locker_stamp(locker_text, "exit"),
            "shell unlock-seen": stamp(text, "unlock-seen"),
            "shell opening-first-frame": stamp(text, "opening-first-frame"),
            "shell opening ended": stamp(text, "unlock-end-held") or stamp(text, "unlock-done"),
            "shell dismissed": stamp(text, "unlock-dismissed") or stamp(text, "unlock-done"),
        }
        result["events_ms_after_touch"] = {k: (v - events["touch"] if v else None) for k, v in events.items()}
        greeting_start = locker_stamp(locker_text, "report method=Fingerprint") or None
        result["shell_probe_line"] = re.findall(r"dojo-lock: event=shell-overlay.*", locker_text)
        trace = unlock_frames(text)
        result["unlock_frames_logged"] = len(trace)
        zero = events["shell unlock-seen"]
        if trace:
            result["opening_logged_ms"] = trace[-1]["unix"] - trace[0]["unix"]
            gaps = [b["unix"] - a["unix"] for a, b in zip(trace, trace[1:])]
            result["unlock_frame_gap_ms"] = {"median": statistics.median(gaps), "max": max(gaps)}
            result["logged_progress_to_width"] = [(f["unix"] - zero, f["progress"], round(f["width"]), f["cover"], f["frame"]) for f in trace]

        # Date the captures. grim writes a capture some time after the compositor drew it. The first capture that equals
        # the held last frame was drawn no earlier than the last logged frame, which fixes the lag to within one cadence.
        rows = [dict(path=path, written=round(written * 1000)) for written, path, _ in kept]
        rows.sort(key=lambda r: r["written"])
        for row in rows:
            row["to_end"] = mean_difference(row["path"], held_small)
        ended = next((r for r in rows if r["to_end"] < 0.3 and r["written"] > trace[0]["unix"]), None)
        lag = (ended["written"] - trace[-1]["unix"]) if ended else 0
        result["capture_lag_ms"] = lag
        for row in rows:
            row["drawn"] = row["written"] - lag - zero
        lo, hi = -args.before, (trace[-1]["unix"] - zero) + 40
        picked, previous = [], None
        for row in rows:
            if not lo <= row["drawn"] <= hi:
                continue
            if previous is not None and mean_difference(previous["path"], row["path"]) < 0.05:
                continue
            picked.append(row)
            previous = row
        for r in picked:
            near = min(trace, key=lambda f: abs(f["unix"] - zero - r["drawn"]))
            r["progress"] = near["progress"] if abs(near["unix"] - zero - r["drawn"]) < 40 else None
        sheet = out / "sheets" / "unlock-morph.png"
        write_sheet([(f"{r['drawn']:+d}", r["path"]) for r in picked], sheet, args.columns)
        result["sheet"] = str(sheet)
        result["sheet_frames"] = [dict(drawn_ms_from_handoff=r["drawn"], logged_progress=r["progress"],
                                       to_final=round(r["to_end"], 2)) for r in picked]
        result["distinct_frames_on_sheet"] = len(picked)
        result["distinct_opening_frames"] = len([r for r in picked if r["progress"] is not None and r["progress"] < 0.98])
        sleeper.join()
        if (frames / "held-late.ppm").exists():
            result["held_early_vs_held_late"] = difference(held, frames / "held-late.ppm")
        if held.exists() and rest.exists():
            result["last_overlay_frame_vs_resting_before_lock"] = difference(held, rest)
        if held.exists() and after_rest.exists():
            result["last_overlay_frame_vs_resting_after_dismiss"] = difference(held, after_rest)
        if rest.exists() and after_rest.exists():
            result["resting_before_lock_vs_after_dismiss"] = difference(rest, after_rest)
        stop_launch.set()
        launch_thread.join()
    finally:
        rig.close()
    (rig.out / "result.json").write_text(json.dumps(result, indent=2, default=str))
    print(json.dumps(result, indent=2, default=str))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--shell", default=str(ROOT / "target/debug/dojo"))
    parser.add_argument("--locker", default=str(ROOT / "target/debug/dojo-lock"))
    parser.add_argument("--fake-logind", default=str(ROOT / "target/debug/examples/fake_logind"))
    parser.add_argument("--fixture", default=str(HERE / "target/debug/dojo-agent-input-fixture"))
    parser.add_argument("--lock-wallpaper")
    parser.add_argument("--scale", default="1.75")
    parser.add_argument("--interval", type=int, default=25)
    parser.add_argument("--shot-scale", default="0.4")
    parser.add_argument("--settle", type=float, default=4.0)
    parser.add_argument("--idle", type=float, default=5.0)
    parser.add_argument("--hold", type=int, default=4000, help="ms the shell keeps the finished opening on screen")
    parser.add_argument("--columns", type=int, default=6)
    parser.add_argument("--before", type=int, default=200, help="ms before the handoff to show")
    parser.add_argument("--wire", action="store_true")
    parser.add_argument("--restart-locker", action="store_true", help="kill the locker while locked and start another")
    parser.add_argument("--baseline", action="store_true", help="transition off: no overlay; compare the desktop before and after a lock")
    parser.add_argument("--vk-driver-files", default=next(
        (str(p) for p in (Path("/run/opengl-driver/share/vulkan/icd.d/radeon_icd.x86_64.json"),
                          Path("/usr/share/vulkan/icd.d/radeon_icd.x86_64.json")) if p.exists()), None))
    args = parser.parse_args()
    assert os.environ.get("WAYLAND_DISPLAY") and os.environ.get("XDG_RUNTIME_DIR"), "needs a host Wayland session to nest in"
    run(args)


if __name__ == "__main__":
    main()
