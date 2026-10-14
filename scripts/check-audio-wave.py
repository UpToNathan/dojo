#!/usr/bin/env python3
"""Drive the live audio wave with an independent seat; restore volume afterward."""

import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import time


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def volume():
    fields = run("wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@").split()
    return round(float(fields[1]) * 100), "[MUTED]" in fields


def wait_volume(expected):
    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        actual = volume()
        if actual == expected:
            return actual
        time.sleep(0.05)
    raise AssertionError(f"Expected {expected}, got {actual}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True)
    parser.add_argument("--x", type=float, required=True, help="Visible wave center, logical pixels")
    parser.add_argument("--y", type=float, default=12)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    original = volume()
    client = socket.socket(socket.AF_UNIX)
    client.settimeout(5)
    client.connect(os.environ["NIRI_SOCKET"])
    stream = client.makefile("rwb")

    def request(message):
        stream.write(json.dumps(message).encode() + b"\n")
        stream.flush()
        reply = json.loads(stream.readline())
        if "Err" in reply:
            raise RuntimeError(reply["Err"])
        result = reply["Ok"]
        if isinstance(result, dict) and "AgentInputPaused" in result:
            raise RuntimeError(f"Input paused: {result}")
        return result

    def action(command):
        return request({"AgentInput": {"Coordinated": {"revision": revision, "command": command}}})

    def move(x, y):
        result = action({"Move": {"output": args.output, "x": x, "y": y}})
        if result == "AgentInputMoving":
            action("Wait")

    def button(state):
        action({"Button": {"button": 272, "state": state}})

    def capture(name):
        time.sleep(0.35)
        subprocess.run(["grim", "-o", args.output, str(args.evidence / f"{name}.png")], check=True)

    results = {}
    request({"AgentInput": {"Create": {"name": "audio-wave-check"}}})
    revision = request("InputCoordination")["InputCoordination"]["revision"]
    try:
        run("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "60%")
        run("wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "0")
        time.sleep(0.25)
        move(args.x, args.y)
        capture("hover-60")
        button("Pressed")
        button("Released")
        results["click_mute"] = wait_volume((60, True))
        capture("muted")
        button("Pressed")
        button("Released")
        results["click_unmute"] = wait_volume((60, False))
        button("Pressed")
        move(args.x - 16, args.y)
        button("Released")
        results["drag_left"] = wait_volume((40, False))
        capture("drag-40")
        move(args.x, args.y)
        action({"Axis": {"horizontal": 0., "vertical": -12.}})
        results["scroll_up"] = wait_volume((46, False))
        capture("scroll-46")
        # Release outside the wave, within the bar; agent seats do not support client pointer grabs.
        button("Pressed")
        move(args.x - 8, 23.5)
        button("Released")
        results["release_outside"] = wait_volume((36, False))
        run("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "25%")
        results["external_change"] = wait_volume((25, False))
        move(args.x, args.y)
        capture("external-25")
        print(json.dumps(results, indent=2))
        (args.evidence / "interaction-results.json").write_text(json.dumps(results, indent=2) + "\n")
    finally:
        request({"AgentInput": "Reset"})
        request({"AgentInput": "Destroy"})
        stream.close()
        client.close()
        run("wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", f"{original[0]}%")
        run("wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "1" if original[1] else "0")
        wait_volume(original)
        print(f"Restored volume {original[0]}%, muted={original[1]}")


if __name__ == "__main__":
    main()
