#!/usr/bin/env python3
"""Require fullscreen clients to cover the frame, then restore it on exit."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from run import Peer, changed_pixels, events, screenshot, wait_for


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True)
    parser.add_argument("--menu-screenshots", action="store_true")
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
    agent = None
    shell_binary = os.environ.get("DOJO_BINARY", str(root / "target/debug/dojo"))
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
            baseline = screenshot(query, str(directory / "fullscreen-baseline.png"), show_pointer=False)
            toggle()
            shell = subprocess.Popen([shell_binary, "--enable"], env=shell_env, stdout=shell_log, stderr=shell_log)

            def frame_mapped():
                if shell.poll() is not None:
                    raise RuntimeError(f"Shell exited: {shell.returncode}; see {directory / 'shell.log'}")
                return any(item["namespace"].startswith("dojo-frame-") for item in query.request("Layers")["Layers"])

            wait_for(frame_mapped, "nested shell frame")
            time.sleep(0.5)
            toggle()
            fullscreen = screenshot(query, str(directory / "fullscreen-with-shell.png"), show_pointer=False)
            output = next(iter(query.request("Outputs")["Outputs"].values()))
            width, height = output["logical"]["width"], output["logical"]["height"]
            scale = output["logical"]["scale"]
            points = [(8, 8), (width / 2, 8), (width - 8, 8), (8, height / 2),
                      (width - 8, height / 2), (8, height - 8), (width / 2, height - 8), (width - 8, height - 8)]
            covered = [changed_pixels(baseline, fullscreen, x, y, scale) for x, y in points]
            toggle()
            restored = screenshot(query, str(directory / "frame-restored.png"), show_pointer=False)
            agent = Peer(os.environ["NIRI_SOCKET"])
            agent.agent({"Create": {"name": "frame-pass-through"}})
            agent.agent({"Focus": {"window_id": target}})
            query.request({"Action": {"MoveWindowToFloating": {"id": target}}})
            query.request({"Action": {"SetWindowWidth": {"id": target, "change": {"SetFixed": 260}}}})
            query.request({"Action": {"SetWindowHeight": {"id": target, "change": {"SetFixed": 200}}}})
            query.request({"Action": {"MoveFloatingWindow": {"id": target,
                "x": {"SetFixed": 180.0}, "y": {"SetFixed": 100.0}}}})
            query.request({"Action": {"FocusWindow": {"id": target}}})
            def pointer_geometry():
                client = next(w for w in query.request("Windows")["Windows"] if w["id"] == target)
                layout = client["layout"]
                return layout if layout.get("tile_pos_in_workspace_view") is not None and layout["window_size"] == [260, 200] else None
            layout = wait_for(pointer_geometry, "restored client pointer geometry")
            time.sleep(0.3)
            x, y = layout["tile_pos_in_workspace_view"]
            ox, oy = layout["window_offset_in_tile"]
            width, height = layout["window_size"]
            output_name = next(iter(query.request("Outputs")["Outputs"]))
            agent.agent({"Move": {"output": output_name, "x": x + ox + width / 2, "y": y + oy + height / 2}})
            before_click = len(events(args.log))
            agent.agent({"Button": {"button": 272, "state": "Pressed"}})
            agent.agent({"Button": {"button": 272, "state": "Released"}})
            wait_for(lambda: any(e.get("event") == "button" and e.get("window") == 1
                                 and e.get("button") == 272 and "Released" in e.get("state", "")
                                 for e in events(args.log)[before_click:]), "underlying client click through frame")
            restored_pixels = changed_pixels(baseline, restored, 8, 8, scale)
            assert restored_pixels > 0, "Shell frame did not return after leaving fullscreen"
            print(json.dumps({"fullscreen_edge_changes": covered, "restored_frame_pixels": restored_pixels, "pointer_pass_through": True, "evidence": str(directory)}))
            assert covered == [0] * len(points), "Shell still covers the fullscreen client's edges"
            if args.menu_screenshots:
                for command, namespace in [("system-menu", "dojo-system-menu"), ("wifi", "dojo-wifi-panel")]:
                    subprocess.run([shell_binary, f"--{command}"], env=shell_env, timeout=5, check=True)
                    wait_for(lambda: any(item["namespace"] == namespace for item in query.request("Layers")["Layers"]), namespace)
                    time.sleep(0.3)
                    screenshot(query, str(directory / f"{command}-shadow.png"), show_pointer=False)
                    subprocess.run([shell_binary, f"--{command}"], env=shell_env, timeout=5, check=True)
                    wait_for(lambda: not any(item["namespace"] == namespace for item in query.request("Layers")["Layers"]), f"{namespace} closed")
        finally:
            if agent is not None:
                agent.close()
            if shell is not None:
                subprocess.run([shell_binary, "--quit"], env=shell_env, timeout=5, check=False)
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
