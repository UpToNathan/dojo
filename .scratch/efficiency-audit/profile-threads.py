#!/usr/bin/env python3
"""Attribute CPU time to existing threads without tracing or pausing a process."""
import argparse
import json
import os
from pathlib import Path
import time


def snapshot(pid):
    result = {}
    for task in (Path("/proc") / str(pid) / "task").iterdir():
        try:
            stat = (task / "stat").read_text().rsplit(")", 1)[1].split()
            result[task.name] = {
                "name": (task / "comm").read_text().strip(),
                "cpu_ticks": int(stat[11]) + int(stat[12]),
                "start_ticks": int(stat[19]),
                "wchan": (task / "wchan").read_text().strip(),
            }
        except OSError:
            pass
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pid", type=int)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 0 < args.seconds <= 60:
        parser.error("seconds must be between 0 and 60")
    before = snapshot(args.pid)
    start = time.monotonic()
    time.sleep(args.seconds)
    after = snapshot(args.pid)
    elapsed = time.monotonic() - start
    ticks = os.sysconf("SC_CLK_TCK")
    rows = []
    for tid, current in after.items():
        previous = before.get(tid)
        if not previous or previous["start_ticks"] != current["start_ticks"]:
            continue
        rows.append({"tid": tid, "name": current["name"], "wchan_at_end": current["wchan"], "cpu_pct_one_core": (current["cpu_ticks"] - previous["cpu_ticks"]) / ticks / elapsed * 100})
    output = {"pid": args.pid, "elapsed_seconds": elapsed, "samples": 1, "before": before, "after": after, "summary": sorted(rows, key=lambda row: row["cpu_pct_one_core"], reverse=True)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2) + "\n")
    print(json.dumps({"elapsed_seconds": elapsed, "summary": output["summary"][:8]}))


if __name__ == "__main__":
    main()
