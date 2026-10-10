#!/usr/bin/env python3
"""Require fullscreen clients to cover the frame, then restore it on exit."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from run import Peer, changed_pixels, screenshot, wait_for


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    directory = Path(args.log).parent
    runtime = directory / "shell-runtime"
    runtime.mkdir(mode=0o700)
    shell_env = dict(os.environ)
    shell_env["WAYLAND_DISPLAY"] = str(Path(os.environ["XDG_RUNTIME_DIR"]) / os.environ["WAYLAND_DISPLAY"])
    shell_env["XDG_RUNTIME_DIR"] = str(runtime)
    shell_env["XDG_STATE_HOME"] = str(directory / "shell-state")
    query = Peer(os.environ["NIRI_SOCKET"])
    shell = None
    with open(args.log, "w") as log, open(directory / "shell.log", "w") as shell_log:
        recorder = subprocess.Popen([str(root / "tools/agent-input/target/debug/dojo-agent-input-fixture")], stdout=log)
        try:
            def windows():
                found = [w for w in query.request("Windows")["Windows"] if w.get("app_id") == "dojo-agent-input-fixture"]
                return sorted(found, key=lambda w: w["title"]) if len(found) == 2 else None

            target = wait_for(windows, "fixture windows")[0]["id"]
            query.request({"Action": {"MoveWindowToTiling": {"id": target}}})
            query.request({"Action": {"FocusWindow": {"id": target}}})

            def toggle():
                query.request({"Action": {"FullscreenWindow": {"id": target}}})
                time.sleep(0.5)

            toggle()
            baseline = screenshot(query, str(directory / "fullscreen-baseline.png"))
            toggle()
            shell = subprocess.Popen([str(root / "target/debug/dojo"), "--enable"], env=shell_env, stdout=shell_log, stderr=shell_log)

            def frame_mapped():
                if shell.poll() is not None:
                    raise RuntimeError(f"Shell exited: {shell.returncode}; see {directory / 'shell.log'}")
                return any(item["namespace"].startswith("dojo-frame-") for item in query.request("Layers")["Layers"])

            wait_for(frame_mapped, "nested shell frame")
            time.sleep(0.5)
            toggle()
            fullscreen = screenshot(query, str(directory / "fullscreen-with-shell.png"))
            output = next(iter(query.request("Outputs")["Outputs"].values()))
            width, height = output["logical"]["width"], output["logical"]["height"]
            scale = output["logical"]["scale"]
            points = [(8, 8), (width / 2, 8), (width - 8, 8), (8, height / 2),
                      (width - 8, height / 2), (8, height - 8), (width / 2, height - 8), (width - 8, height - 8)]
            covered = [changed_pixels(baseline, fullscreen, x, y, scale) for x, y in points]
            toggle()
            restored = screenshot(query, str(directory / "frame-restored.png"))
            restored_pixels = changed_pixels(baseline, restored, 8, 8, scale)
            assert restored_pixels > 0, "Shell frame did not return after leaving fullscreen"
            print(json.dumps({"fullscreen_edge_changes": covered, "restored_frame_pixels": restored_pixels, "evidence": str(directory)}))
            assert covered == [0] * len(points), "Shell still covers the fullscreen client's edges"
        finally:
            if shell is not None:
                subprocess.run([str(root / "target/debug/dojo"), "--quit"], env=shell_env, timeout=5, check=False)
                try:
                    shell.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    shell.kill()
                    shell.wait()
            recorder.terminate()
            recorder.wait(timeout=5)
            query.close()


if __name__ == "__main__":
    main()
