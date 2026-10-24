#!/usr/bin/env python3
"""Time launcher opening in the nested Niri that run-nested.sh started.

Never run against the live session: it starts its own shell in a private runtime directory.
Each round toggles the launcher open, polls screenshots until the panel's background colour
appears, then toggles it closed. A second phase measures the resident shell at rest, and a third
takes one screenshot per trial at a fixed delay after the command. A build that prints `WINOPEN`
timing lines (see docs/research/dojo-launcher-performance.md) adds the in-process columns; without
them only the screenshot columns are reported.

    devenv shell -- tools/agent-input/run-nested.sh --launcher-open [--rounds N]
"""
import argparse
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import time

from run import Peer, wait_for

DELAYS_MS = [20, 30, 40, 50, 60, 70, 80, 100, 120]
PANEL_COLOUR = (0x1e, 0x1e, 0x2e)  # config.toml fallback `background`
TOP_BAR_HEIGHT = 24


def stats(values):
    values = sorted(values)
    if not values:
        return None
    return {
        "n": len(values),
        "min": round(values[0], 2),
        "median": round(statistics.median(values), 2),
        "max": round(values[-1], 2),
    }


def process_numbers(pid):
    status = Path(f"/proc/{pid}/status").read_text()
    rss_kb = int(re.search(r"VmRSS:\s+(\d+)", status).group(1))
    smaps = Path(f"/proc/{pid}/smaps_rollup").read_text()
    pss_kb = int(re.search(r"^Pss:\s+(\d+)", smaps, re.M).group(1))
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    ticks = int(fields[11]) + int(fields[12])  # utime + stime
    return rss_kb, pss_kb, ticks


def drm_memory_mb(pid):
    """GPU memory the process holds, from the DRM fdinfo of its render nodes (one client once)."""
    clients = {}
    for fd in Path(f"/proc/{pid}/fdinfo").iterdir():
        try:
            text = fd.read_text()
        except OSError:
            continue
        if "drm-driver" not in text:
            continue
        fields = dict(line.split(":", 1) for line in text.splitlines() if ":" in line)
        clients[fields.get("drm-client-id", fd.name).strip()] = fields
    total = {}
    for fields in clients.values():
        for key, value in fields.items():
            if key.startswith("drm-total-"):
                number, _, unit = value.strip().partition(" ")
                kib = int(number) * {"": 1 / 1024, "KiB": 1, "MiB": 1024, "GiB": 1024 * 1024}.get(unit, 0)
                total[key[len("drm-total-"):]] = total.get(key[len("drm-total-"):], 0) + kib
    return {name: round(kib / 1024, 1) for name, kib in total.items() if kib}


def parse_log(path):
    """Group `WINOPEN` lines by the launcher open that preceded them."""
    rounds = []
    for line in Path(path).read_text().splitlines():
        if not line.startswith("WINOPEN"):
            continue
        if line.startswith("WINOPEN toggle_start"):
            rounds.append({"start_us": int(line.split("us=")[1]), "lines": []})
        elif rounds:
            rounds[-1]["lines"].append(line)
    return rounds


def millis(lines, prefix, key):
    for line in lines:
        if line.startswith(prefix):
            pattern = r"([0-9.]+)ms" if not key else rf"{re.escape(key)}=([0-9.]+)"
            match = re.search(pattern, line)
            if match:
                return float(match.group(1))
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True)
    parser.add_argument("--rounds", type=int, default=8)
    parser.add_argument("--idle-seconds", type=float, default=10)
    parser.add_argument("--trials", type=int, default=5, help="screenshots per delay")
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
    # The nested shell must not talk to the live session's services.
    env.pop("DBUS_SESSION_BUS_ADDRESS", None)
    query = Peer(os.environ["NIRI_SOCKET"])
    binary = os.environ.get("DOJO_BINARY", str(root / "target/debug/dojo"))
    pixel_diff = os.environ.get("DOJO_PIXEL_DIFF_BINARY", str(Path(__file__).parent / "target/debug/pixel-diff"))
    shell_log_path = directory / "shell.log"

    def layers():
        return {item["namespace"] for item in query.request("Layers")["Layers"]}

    def command(*flags):
        subprocess.run([binary, *flags], env=env, timeout=10, check=True)

    def panel_pixels(path, x, y, scale):
        out = subprocess.check_output(
            [pixel_diff, "--color", path, str(round(x * scale)), str(round(y * scale)), str(round(4 * scale)), *map(str, PANEL_COLOUR)],
            text=True,
        )
        return json.loads(out)["matching_pixels"]

    shell = None
    try:
        with open(shell_log_path, "w") as shell_log:
            shell = subprocess.Popen([binary, "--enable"], env=env, stdout=shell_log, stderr=shell_log)
            wait_for(lambda: any(n.startswith("dojo-frame-") for n in layers()), "shell frame")
            wait_for(lambda: not any(n.startswith("dojo-session") for n in layers()), "opening overlay gone")
            time.sleep(1.5)
            output = next(iter(query.request("Outputs")["Outputs"].values()))["logical"]
            scale = output["scale"]
            x, y = output["width"] / 2, TOP_BAR_HEIGHT + 8
            full = 4 * 4 * 4 * scale * scale  # the sampled square, in pixels
            print("output", output, flush=True)

            # Memory before the first opening, to see whether repeated openings leak.
            start_rss, start_pss, _ = process_numbers(shell.pid)
            start_gpu = drm_memory_mb(shell.pid)
            results = []
            for round_number in range(args.rounds):
                started = time.time()
                command("--launcher")
                acked = time.time()
                shots = []
                layer_seen = None
                # Poll screenshots until the panel colour fills the sampled square, then a few more.
                deadline = started + 3
                while time.time() < deadline:
                    requested = time.time()
                    if layer_seen is None and "dojo-launcher" in layers():
                        layer_seen = time.time()
                    path = str(directory / f"open-{round_number}-{len(shots)}.png")
                    query.request({"Action": {"ScreenshotScreen": {"write_to_disk": True, "show_pointer": False, "path": path}}})
                    wait_for(lambda: Path(path).exists() and Path(path).read_bytes().endswith(b"IEND\xaeB`\x82"), path)
                    shots.append((requested, time.time(), path))
                    if len(shots) > 3 and panel_pixels(shots[-1][2], x, y, scale) >= 0.9 * full:
                        break
                visible = next(((r, d) for r, d, p in shots if panel_pixels(p, x, y, scale) >= 0.9 * full), None)
                record = {
                    "round": round_number,
                    "cli_ack_ms": (acked - started) * 1e3,
                    "layer_listed_ms": None if layer_seen is None else (layer_seen - started) * 1e3,
                    "panel_capture_requested_ms": None if visible is None else (visible[0] - started) * 1e3,
                    "panel_capture_done_ms": None if visible is None else (visible[1] - started) * 1e3,
                    "toggle_start_us": int(started * 1e6),
                }
                results.append(record)
                for _, _, path in shots:
                    os.unlink(path)  # screenshots are large; only the timings matter
                command("--launcher")  # close
                # A hidden layer surface stays listed by Niri, so wait out the 150 ms closing animation.
                time.sleep(1.2)

            # Resident shell at rest.
            time.sleep(2)
            end_rss, end_pss, _ = process_numbers(shell.pid)
            growth = {
                "rounds": args.rounds,
                "rss_mb": round((end_rss - start_rss) / 1024, 1),
                "pss_mb": round((end_pss - start_pss) / 1024, 1),
                "gpu_mb_before": start_gpu,
                "gpu_mb_after": drm_memory_mb(shell.pid),
            }
            before = process_numbers(shell.pid)
            time.sleep(args.idle_seconds)
            after = process_numbers(shell.pid)
            ticks_per_second = os.sysconf("SC_CLK_TCK")
            idle = {
                "rss_mb": round(after[0] / 1024, 1),
                "pss_mb": round(after[1] / 1024, 1),
                "cpu_percent": round((after[2] - before[2]) / ticks_per_second / args.idle_seconds * 100, 3),
                "seconds": args.idle_seconds,
                "gpu_mb": drm_memory_mb(shell.pid),
            }

            # How soon after the command is the panel on screen? One screenshot per trial, taken
            # a fixed delay after the command, so the probe's own latency stays out of the answer.
            visible_by_delay = {}
            for delay_ms in DELAYS_MS:
                seen = 0
                for trial in range(args.trials):
                    started = time.time()
                    command("--launcher")
                    wait = started + delay_ms / 1e3 - time.time()
                    if wait > 0:
                        time.sleep(wait)
                    path = str(directory / "delay.png")
                    query.request({"Action": {"ScreenshotScreen": {"write_to_disk": True, "show_pointer": False, "path": path}}})
                    wait_for(lambda: Path(path).exists() and Path(path).read_bytes().endswith(b"IEND\xaeB`\x82"), path)
                    seen += panel_pixels(path, x, y, scale) >= 0.9 * full
                    os.unlink(path)
                    command("--launcher")
                    time.sleep(1.2)
                visible_by_delay[delay_ms] = f"{seen}/{args.trials}"
                print("visible", delay_ms, visible_by_delay[delay_ms], flush=True)
    finally:
        if shell is not None:
            subprocess.run([binary, "--quit"], env=env, timeout=5, check=False)
            try:
                shell.wait(timeout=5)
            except subprocess.TimeoutExpired:
                shell.kill()
        query.close()

    # Join the shell's own timing lines to each round.
    rounds = parse_log(shell_log_path)
    for record, parsed in zip(results, rounds):
        lines = parsed["lines"]
        record["open_window_ms"] = millis(lines, "WINOPEN toggle_open_window_returned", "")
        for prefix, key, name in [
            ("WINOPEN open_window", "total", "gpui_open_window_ms"),
            ("WINOPEN open_window", "Window::new", "window_new_ms"),
            ("WINOPEN open_window", "build_root", "build_root_ms"),
            ("WINOPEN open_window", "first_draw", "first_draw_ms"),
            ("WINOPEN WaylandWindow::new", "total", "wayland_window_new_ms"),
            ("WINOPEN renderer_new", "total", "renderer_new_ms"),
            ("WINOPEN renderer_new", "surface_and_context", "surface_and_context_ms"),
            ("WINOPEN renderer_new", "internal", "renderer_internal_ms"),
            ("WINOPEN core_new", "total", "core_new_ms"),
            ("WINOPEN core_new", "layouts_and_pipelines", "layouts_and_pipelines_ms"),
        ]:
            record[name] = millis(lines, prefix, key)
        first_present = next((int(l.split("us=")[1].split()[0]) for l in lines if l.startswith("WINOPEN present")), None)
        record["first_present_ms"] = None if first_present is None else (first_present - parsed["start_us"]) / 1e3
    def summarize(rows):
        return {
            key: stats([r[key] for r in rows if r.get(key) is not None])
            for key in rows[0]
            if key not in ("round", "toggle_start_us")
        } if rows else {}

    summary = summarize(results)
    # The first opening always creates the window; later ones may reuse a hidden one.
    summary_after_first = summarize(results[1:])
    report = {"rounds": results, "summary": summary, "summary_after_first": summary_after_first, "idle": idle,
              "growth_over_rounds": growth, "panel_visible_by_delay_ms": visible_by_delay}
    Path(args.log).write_text(json.dumps(report, indent=2))
    print(json.dumps({"summary_after_first": summary_after_first, "idle": idle,
                      "growth_over_rounds": growth, "panel_visible_by_delay_ms": visible_by_delay}, indent=2))


if __name__ == "__main__":
    main()
