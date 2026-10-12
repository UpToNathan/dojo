#!/usr/bin/env python3
"""Read desktop process counters without changing services or user state."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import time

NAMES = {"dojo", "niri", "dojo-speech", "t3code", ".wluma-wrapped", "stasis", "tailscale", "1password"}
TICKS = os.sysconf("SC_CLK_TCK")


def fields(path):
    return dict(line.split(":", 1) for line in path.read_text().splitlines() if ":" in line)


def snapshot():
    result = {}
    errors = 0
    for directory in Path("/proc").iterdir():
        if not directory.name.isdigit():
            continue
        try:
            name = (directory / "comm").read_text().strip()
            if name not in NAMES:
                continue
            stat = (directory / "stat").read_text().rsplit(")", 1)[1].split()
            memory = fields(directory / "smaps_rollup")
            switches = 0
            for task in (directory / "task").iterdir():
                try:
                    status = fields(task / "status")
                    switches += int(status["voluntary_ctxt_switches"]) + int(status["nonvoluntary_ctxt_switches"])
                except (OSError, KeyError):
                    errors += 1
            clients = {}
            for descriptor in (directory / "fdinfo").iterdir():
                try:
                    info = fields(descriptor)
                    if "drm-client-id" not in info:
                        continue
                    key = info.get("drm-pdev", "").strip() + "/" + info["drm-client-id"].strip()
                    clients[key] = {
                        "vram_KiB": int(info.get("drm-memory-vram", "0").split()[0]),
                        "gfx_ns": int(info.get("drm-engine-gfx", "0").split()[0]),
                    }
                except (OSError, ValueError):
                    errors += 1
            result[directory.name] = {
                "name": name, "ppid": int(stat[1]), "start_ticks": int(stat[19]),
                "cpu_ticks": int(stat[11]) + int(stat[12]), "switches": switches,
                "pss_KiB": int(memory["Pss"].split()[0]),
                "rss_KiB": int(memory["Rss"].split()[0]),
                "clients": clients, "cgroup": (directory / "cgroup").read_text().strip(),
                "executable": str((directory / "exe").resolve()),
            }
        except (OSError, KeyError, ValueError):
            errors += 1
    battery = {}
    for device in Path("/sys/class/power_supply").iterdir():
        battery[device.name] = {
            name: (device / name).read_text().strip()
            for name in ("type", "status", "online", "energy_now", "power_now")
            if (device / name).exists()
        }
    return {"time": time.monotonic(), "processes": result, "read_errors": errors, "battery": battery}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--interval", type=float, default=5)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.samples < 1 or not 0 < args.interval <= 60:
        parser.error("samples must be positive and interval must be between 0 and 60 seconds")
    baseline = snapshot()
    raw = [baseline]
    intervals = []
    for _ in range(args.samples):
        time.sleep(args.interval)
        current = snapshot()
        elapsed = current["time"] - baseline["time"]
        rows = []
        for pid, process in current["processes"].items():
            previous = baseline["processes"].get(pid)
            if not previous or previous["start_ticks"] != process["start_ticks"]:
                continue
            gfx_delta = sum(
                max(0, client["gfx_ns"] - previous["clients"][key]["gfx_ns"])
                for key, client in process["clients"].items() if key in previous["clients"]
            )
            rows.append({
                "pid": pid, "name": process["name"],
                "cpu_pct_one_core": (process["cpu_ticks"] - previous["cpu_ticks"]) / TICKS / elapsed * 100,
                "context_switches_per_sec": max(0, process["switches"] - previous["switches"]) / elapsed,
                "gfx_busy_pct": gfx_delta / 1e9 / elapsed * 100,
                "pss_KiB": process["pss_KiB"],
                "vram_KiB": sum(client["vram_KiB"] for client in process["clients"].values()),
            })
        intervals.append({"elapsed_seconds": elapsed, "rows": rows})
        raw.append(current)
        baseline = current
    summary = []
    for pid, process in raw[-1]["processes"].items():
        rows = [row for interval in intervals for row in interval["rows"] if row["pid"] == pid]
        if not rows:
            continue
        entry = {"pid": pid, "name": process["name"], "samples": len(rows), "pss_MiB": rows[-1]["pss_KiB"] / 1024, "vram_MiB": rows[-1]["vram_KiB"] / 1024}
        for metric in ("cpu_pct_one_core", "gfx_busy_pct", "context_switches_per_sec"):
            values = [row[metric] for row in rows]
            entry[metric] = {"median": statistics.median(values), "min": min(values), "max": max(values)}
        summary.append(entry)
    output = {
        "head": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
        "logical_cpus": os.cpu_count(), "load_average": os.getloadavg(),
        "method": "five passive process-counter intervals by default; CPU percent is one logical core; context switches are not hardware wakeups; per-client GPU memory can be shared between processes and must not be summed; charging battery power is not discharge power",
        "raw": raw, "intervals": intervals,
        "summary": sorted(summary, key=lambda row: row["cpu_pct_one_core"]["median"], reverse=True),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps({"output": str(args.output), "summary": output["summary"]}))


if __name__ == "__main__":
    main()
