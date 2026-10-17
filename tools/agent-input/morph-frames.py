#!/usr/bin/env python3
"""Reads a run of run-lock-handoff.py (kept frames) and lines each grim frame up with the shell's own frame log.

grim lags: a capture's file is written 100 to 200 ms after the compositor produced the picture it holds, so
the offset in a frame's name is not when it was drawn. The shell (`dojo-trace: frame`) and the locker
(`dojo-lock-trace: frame`) log the geometry of every frame they render, with a wall-clock stamp. This tool
measures the opening's width in each capture (a scan of its middle row), finds the logged frame with that
width, and takes the difference of the two stamps as that capture's lag. The median lag then dates the
captures that cannot be matched (the opening is not moving in them). Output: a table of every capture, the
lag statistics, and a contact sheet ordered by estimated draw time, labelled with it.

The trace lines exist only in a build with the `record-session-actions` feature; without them the tool has nothing to align with.

    python3 tools/agent-input/morph-frames.py RUN_DIR [--from MS] [--to MS]
"""
import argparse
import importlib.util
import json
import re
import statistics
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("handoff", HERE / "run-lock-handoff.py")
handoff = importlib.util.module_from_spec(spec)
spec.loader.exec_module(handoff)

SHELL = re.compile(r"dojo-trace: frame since_begin_ms=(\d+) progress=([\d.]+) cover=([\d.]+)(?: frame=([\d.]+))? "
                   r"open_w=([\d.]+) .*?wallpaper=([\d.]+).*?unix_ms=(\d+)")
LOCKER = re.compile(r"dojo-lock-trace: frame ring=([\d.]+) text=([\d.]+) open_w=([\d.]+) wallpaper=([\d.]+) "
                    r"quiet=([\d.]+) animating=(\w+) unix_ms=(\d+)")
EDGE = 5


def trace_lines(path):
    text = Path(path).read_text(errors="replace")
    shell = [dict(progress=float(m[2]), cover=float(m[3]), frame=float(m[4] or 1), width=float(m[5]),
                  wallpaper=float(m[6]), unix=int(m[7])) for m in SHELL.finditer(text)]
    return shell


def locker_lines(path):
    text = Path(path).read_text(errors="replace")
    return [dict(ring=float(m[1]), text=float(m[2]), width=float(m[3]), wallpaper=float(m[4]),
                 quiet=float(m[5]), animating=m[6] == "true", unix=int(m[7])) for m in LOCKER.finditer(text)]


def measure(path):
    """(left, right, centre colour) of the opening on the middle row, in the capture's pixels."""
    width, height, pixels = handoff.read_ppm(path)
    y = height // 2
    row = [pixels[(y * width + x) * 3:(y * width + x) * 3 + 3] for x in range(width)]
    far = lambda a, b: max(abs(a[i] - b[i]) for i in range(3)) > EDGE
    left = next((x for x in range(3, width) if far(row[x], row[2])), None)
    right = next((x for x in range(width - 4, 0, -1) if far(row[x], row[width - 3])), None)
    return width, left, right, tuple(row[width // 2])


def mean_difference(first, second):
    """Mean absolute channel difference between two captures of the same size, 0 to 255."""
    (width, height, a), (_, _, b) = handoff.read_ppm(first), handoff.read_ppm(second)
    total = sum(abs(x - y) for x, y in zip(a, b))
    return total / (width * height * 3)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("run")
    parser.add_argument("--from", dest="start", type=int, default=-60, help="first estimated draw time to show, ms from closing-begin")
    parser.add_argument("--to", dest="end", type=int, default=100000)
    parser.add_argument("--columns", type=int, default=6)
    parser.add_argument("--name", default="morph")
    args = parser.parse_args()
    run = Path(args.run)
    result = json.loads((run / "result.json").read_text())
    logical = next(iter(result["outputs"].values()))["width"]
    shell_log = (run / "shell.log").read_text(errors="replace")
    begin = int(re.search(r"closing-begin occasion=lock ms=0 unix_ms=(\d+)", shell_log).group(1))
    shell = trace_lines(run / "shell.log")
    locker = locker_lines(run / "locker-0.log")
    frames = []
    for path in sorted((run / "frames").glob("lock-*.ppm")):
        offset = int(re.search(r"-\d{4}-([+-]\d+)\.ppm$", path.name).group(1))
        width, left, right, centre = measure(path)
        scale = width / logical
        frames.append(dict(path=path, offset=offset, left=left, right=right,
                           width=None if left is None or right is None else (right - left) / scale, centre=centre))
    rest = shell[0]["width"]
    dial = min(f["width"] for f in shell)
    candidates = []
    for frame in frames:
        width = frame["width"]
        if width is None or not dial + 8 < width < rest - 8:
            continue
        near = min(shell, key=lambda f: abs(f["width"] - width))
        if abs(near["width"] - width) <= 6:
            candidates.append((frame, near["unix"] - begin))
    # A width can recur (the locker's ring, a capture written late), so a match far from the others is not trusted.
    lag = None
    lags = []
    if candidates:
        lag = round(statistics.median(frame["offset"] - drawn for frame, drawn in candidates))
        for frame, drawn in candidates:
            if abs(frame["offset"] - drawn - lag) <= 30:
                frame["matched"] = drawn
                lags.append(frame["offset"] - drawn)
        lag = round(statistics.median(lags)) if lags else lag
    report = {"matched_frames": len(lags), "lag_ms_median": lag,
              "lag_ms_spread": [min(lags), max(lags)] if lags else None}
    rows = []
    for frame in frames:
        drawn = frame.get("matched", frame["offset"] - (lag or 0))
        frame["drawn"] = drawn
        near = min(shell, key=lambda f: abs(f["unix"] - begin - drawn)) if shell else None
        rows.append({"drawn_ms": drawn, "capture_offset_ms": frame["offset"], "opening_width": frame["width"],
                     "matched": "matched" in frame, "centre": frame["centre"],
                     "progress_then": near["progress"] if near and abs(near["unix"] - begin - drawn) < 40 else None})
    rows.sort(key=lambda r: r["drawn_ms"])
    report["frames"] = rows
    picked = sorted((f for f in frames if args.start <= f["drawn"] <= args.end), key=lambda f: f["drawn"])
    sheet = run / "sheets" / f"{args.name}.png"
    handoff.write_sheet([(f"{f['drawn']:+d}", f["path"]) for f in picked], sheet, args.columns)
    report["sheet"] = str(sheet)
    confirmed = result["timeline_ms_from_closing_begin"].get("lock-confirmed", {}).get("ms_after_closing_begin")
    if confirmed is not None:
        ordered = sorted(frames, key=lambda f: f["drawn"])
        report["handoff_at_ms"] = confirmed
        report["difference_between_neighbouring_captures"] = [
            {"drawn_ms": b["drawn"], "mean_abs_difference": round(mean_difference(a["path"], b["path"]), 3)}
            for a, b in zip(ordered, ordered[1:]) if confirmed - 150 <= b["drawn"] <= confirmed + 150]
    shrinking = sorted({round(f["width"]) for f in frames if f["width"] and dial + 8 < f["width"] < rest - 8
                        and args.start <= f["drawn"] <= args.end}, reverse=True)
    report["distinct_shrinking_widths"] = shrinking
    report["shell_frames_logged"] = len(shell)
    report["locker_frames_logged"] = len(locker)
    gaps = [b["unix"] - a["unix"] for a, b in zip(shell, shell[1:])]
    report["shell_frame_gaps_ms"] = {"median": statistics.median(gaps), "max": max(gaps)} if gaps else None
    (run / f"{args.name}.json").write_text(json.dumps(report, indent=1, default=str))
    print(json.dumps({k: v for k, v in report.items() if k != "frames"}, indent=1, default=str))
    for row in rows:
        print(f"{row['drawn_ms']:+5d} ms (capture {row['capture_offset_ms']:+5d})  width={row['opening_width'] and round(row['opening_width'])}  "
              f"matched={row['matched']}  centre={row['centre']}  progress={row['progress_then']}")


if __name__ == "__main__":
    main()
