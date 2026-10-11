#!/usr/bin/env python3
"""Exercise agent IPC against the two-window Wayland event recorder."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time


class Peer:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(5)
        self.socket.connect(path)
        self.stream = self.socket.makefile("rwb", buffering=0)

    def request(self, request):
        self.stream.write(json.dumps(request).encode() + b"\n")
        response = json.loads(self.stream.readline())
        if "Err" in response:
            raise RuntimeError(f"{request}: {response['Err']}")
        return response["Ok"]

    def agent(self, command):
        result = self.request({"AgentInput": command})
        if result == "AgentInputMoving":
            return self.request({"AgentInput": "Wait"})
        return result

    def close(self):
        self.stream.close()
        self.socket.close()


def events(path):
    result = []
    for line in Path(path).read_text().splitlines():
        try:
            result.append(json.loads(line))
        except json.JSONDecodeError:
            pass  # The recorder may be writing its final line.
    return result


def wait_for(check, description):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.02)
    raise AssertionError(f"Timed out waiting for {description}")


def focused_id(query):
    window = query.request("FocusedWindow")["FocusedWindow"]
    return window["id"] if window else None


def screenshot(query, path):
    path = str(Path(path).resolve())
    query.request({"Action": {"ScreenshotScreen": {
        "write_to_disk": True, "show_pointer": True, "path": path}}})
    wait_for(lambda: Path(path).exists() and Path(path).read_bytes().endswith(b"IEND\xaeB`\x82"), "screenshot " + path)
    return path


def changed_pixels(before, after, x, y, scale):
    binary = str(Path(__file__).parent / "target/debug/pixel-diff")
    result = subprocess.check_output([binary, before, after, str(round(x * scale)),
                                      str(round(y * scale)), str(round(48 * scale))], text=True)
    return json.loads(result)["changed_pixels"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", default=os.environ.get("NIRI_SOCKET"))
    parser.add_argument("--recorder", default=str(Path(__file__).parent / "target/debug/dojo-agent-input-fixture"))
    parser.add_argument("--log", help="Keep recorder JSON lines at this path")
    parser.add_argument("--motion", action="store_true", help="Check animation, retargeting and deferred clicks with the raw recorder")
    parser.add_argument("--gpui", action="store_true", help="Validate the GPUI example event format")
    parser.add_argument("--preexisting-seat", action="store_true", help="Create the agent before the recorder connects")
    args = parser.parse_args()
    if not args.socket:
        parser.error("Set NIRI_SOCKET or pass --socket for the nested compositor")
    if args.log:
        log_path = args.log
    else:
        descriptor, log_path = tempfile.mkstemp(prefix="dojo-agent-events-", suffix=".jsonl")
        os.close(descriptor)
    query = Peer(args.socket)
    agent = None
    second = None
    locker = None
    recorder = None
    try:
        with open(log_path, "w") as log:
            created = None
            if args.preexisting_seat:
                agent = Peer(args.socket)
                created = agent.agent({"Create": {"name": "fixture"}})
            recorder = subprocess.Popen([args.recorder], stdout=log)
            def fixture_windows():
                if recorder.poll() is not None:
                    raise RuntimeError(f"Recorder exited with status {recorder.returncode}")
                found = [w for w in query.request("Windows")["Windows"]
                         if w.get("app_id") == "dojo-agent-input-fixture"]
                return sorted(found, key=lambda w: w["title"]) if len(found) == 2 else None
            windows = wait_for(fixture_windows, "both fixture windows")
            human_id, agent_id = windows[0]["id"], windows[1]["id"]
            for index, window in enumerate(windows):
                window_id = window["id"]
                query.request({"Action": {"MoveWindowToFloating": {"id": window_id}}})
                query.request({"Action": {"SetWindowWidth": {"id": window_id, "change": {"SetFixed": 260}}}})
                query.request({"Action": {"SetWindowHeight": {"id": window_id, "change": {"SetFixed": 200}}}})
                query.request({"Action": {"MoveFloatingWindow": {"id": window_id,
                    "x": {"SetFixed": 20.0 + index * 310.0}, "y": {"SetFixed": 60.0}}}})
            wait_for(lambda: all(w["is_floating"] and w["layout"]["window_size"] == [260, 200]
                                 for w in (fixture_windows() or [{"is_floating": False}])), "both floating fixture windows")
            query.request({"Action": {"FocusWindow": {"id": human_id}}})
            wait_for(lambda: focused_id(query) == human_id, "human focus on fixture 1")
            if not args.gpui:
                wait_for(lambda: any(e.get("event") == "keyboard_enter" and e.get("window") == 1
                                     for e in events(log_path)), "human keyboard enter")
            time.sleep(0.3)  # Let layout animation finish before using output-local geometry.
            agent_window = next(w for w in query.request("Windows")["Windows"] if w["id"] == agent_id)
            workspace = next(w for w in query.request("Workspaces")["Workspaces"]
                             if w["id"] == agent_window["workspace_id"])
            output = workspace["output"]
            layout = agent_window["layout"]
            tile_x, tile_y = layout["tile_pos_in_workspace_view"]
            offset_x, offset_y = layout["window_offset_in_tile"]
            width, height = layout["window_size"]
            x, y = tile_x + offset_x + width * 0.75, tile_y + offset_y + height * 0.75
            scale = query.request("Outputs")["Outputs"][output]["logical"]["scale"]
            before_cursor = screenshot(query, log_path + ".before.png")
            baseline_layout = {w["id"]: w["layout"] for w in fixture_windows()}
            if agent is None:
                agent = Peer(args.socket)
                created = agent.agent({"Create": {"name": "fixture"}})
            seat_name = created["AgentInputCreated"]["seat_name"]
            seat_id = None
            if not args.gpui:
                seat_event = wait_for(lambda: next((e for e in events(log_path)
                                                    if e.get("event") == "seat_name" and e.get("name") == seat_name), None),
                                      "agent seat announcement")
                seat_id = seat_event["seat"]
            agent.agent({"Focus": {"window_id": agent_id}})
            if args.motion:
                assert not args.gpui, "Motion samples require the raw recorder"
                from motion import verify_motion
                verify_motion(agent, query, output, x, y, seat_id, log_path)
            agent.agent({"Move": {"output": output, "x": x, "y": y}})
            time.sleep(0.1)
            with_cursor = screenshot(query, log_path + ".agent.png")
            cursor_pixels = changed_pixels(before_cursor, with_cursor, x, y, scale)
            assert cursor_pixels > 0, "Agent pointer did not change screenshot pixels"
            agent.agent({"Button": {"button": 272, "state": "Pressed"}})
            agent.agent({"Button": {"button": 272, "state": "Released"}})
            agent.agent({"Key": {"evdev": 30, "state": "Pressed"}})
            agent.agent({"Key": {"evdev": 30, "state": "Released"}})
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "key_up" and e.get("window") == 2
                                     and e.get("key") == "a" for e in events(log_path)), "GPUI agent A key")
                wait_for(lambda: any(e.get("event") == "mouse_down" and e.get("window") == 2
                                     for e in events(log_path)), "GPUI agent pointer button")
            else:
                wait_for(lambda: any(e.get("event") == "key" and e.get("seat") == seat_id
                                     and e.get("window") == 2 and e.get("key") == 30
                                     and "Released" in e.get("state", "") for e in events(log_path)),
                         "agent A key at fixture 2")
                wait_for(lambda: any(e.get("event") == "button" and e.get("seat") == seat_id
                                     and e.get("window") == 2 for e in events(log_path)),
                         "agent pointer button at fixture 2")
            before_scroll = len(events(log_path))
            agent.agent({"Axis": {"horizontal": 0.0, "vertical": 10.0}})
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "scroll" and e.get("window") == 2
                    and e.get("kind") == "pixels" and e.get("x") == 0.0 and e.get("y") == -30.0
                    and e.get("modifiers", {}).get("shift") is False
                    for e in events(log_path)[before_scroll:]), "GPUI vertical scroll")
            else:
                wait_for(lambda: any(e.get("event") == "axis" and e.get("window") == 2
                    and e.get("seat") == seat_id and e.get("axis") == "Value(VerticalScroll)"
                    and e.get("value") == 10.0 for e in events(log_path)[before_scroll:]), "raw vertical axis")
            assert focused_id(query) == human_id, "Agent input changed human layout focus"
            assert {w["id"]: w["layout"] for w in fixture_windows()} == baseline_layout, "Agent input moved layout"
            observed = events(log_path)
            assert not any(e.get("event") in ("key", "key_down", "key_up")
                           and (args.gpui or e.get("seat") == seat_id)
                           and e.get("window") != 2 for e in observed), "Agent keys reached another window"
            second = Peer(args.socket)
            second.agent({"Create": {"name": "fixture-second"}})
            second.agent({"Focus": {"window_id": human_id}})
            human_layout = baseline_layout[human_id]
            human_tile_x, human_tile_y = human_layout["tile_pos_in_workspace_view"]
            human_offset_x, human_offset_y = human_layout["window_offset_in_tile"]
            human_width, human_height = human_layout["window_size"]
            second.agent({"Move": {"output": output,
                "x": human_tile_x + human_offset_x + human_width / 2,
                "y": human_tile_y + human_offset_y + human_height / 2}})
            second_seat = None
            if not args.gpui:
                second_seat = wait_for(lambda: next((e["seat"] for e in events(log_path)
                    if e.get("event") == "seat_name" and e.get("name", "").endswith("-fixture-second")), None),
                    "second agent seat")
            two_cursors = screenshot(query, log_path + ".two-agents.png")
            modifier_start = len(events(log_path))
            agent.agent({"Key": {"evdev": 42, "state": "Pressed"}})
            agent.agent({"Key": {"evdev": 30, "state": "Pressed"}})
            agent.agent({"Key": {"evdev": 30, "state": "Released"}})
            agent.agent({"Axis": {"horizontal": 0.0, "vertical": 10.0}})
            second.agent({"Key": {"evdev": 48, "state": "Pressed"}})
            second.agent({"Axis": {"horizontal": 0.0, "vertical": 7.0}})
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "key_down" and e.get("window") == 2
                    and e.get("key_char") == "A" and e.get("modifiers", {}).get("shift") is True
                    for e in events(log_path)[modifier_start:]), "shifted agent A character")
                wait_for(lambda: any(e.get("event") == "scroll" and e.get("window") == 2
                    and e.get("x") == -30.0 and e.get("y") == 0.0
                    and e.get("modifiers", {}).get("shift") is True
                    for e in events(log_path)[modifier_start:]), "shifted horizontal scroll")
                wait_for(lambda: any(e.get("event") == "scroll" and e.get("window") == 1
                    and e.get("x") == 0.0 and e.get("y") == -21.0
                    and e.get("modifiers", {}).get("shift") is False
                    for e in events(log_path)[modifier_start:]), "second seat independent vertical scroll")
                wait_for(lambda: any(e.get("event") == "key_down" and e.get("window") == 1
                    and e.get("key_char") == "b" and e.get("held") is True
                    and e.get("modifiers", {}).get("shift") is False
                    for e in events(log_path)[modifier_start:]), "independent unshifted B repeat")
                assert not any(e.get("event") == "key_down" and e.get("held") is True
                    and e.get("window") != 1 for e in events(log_path)[modifier_start:]), "Repeat reached another window"
            else:
                wait_for(lambda: any(e.get("event") == "key" and e.get("seat") == seat_id
                    and e.get("key") == 30 and e.get("depressed") == 1
                    for e in events(log_path)[modifier_start:]), "first seat Shift modifier")
                wait_for(lambda: any(e.get("event") == "key" and e.get("seat") == second_seat
                    and e.get("key") == 48 and e.get("depressed") == 0
                    for e in events(log_path)[modifier_start:]), "second seat unshifted B")
                wait_for(lambda: any(e.get("event") == "axis" and e.get("seat") == second_seat
                    and e.get("window") == 1 and e.get("value") == 7.0
                    for e in events(log_path)[modifier_start:]), "second seat literal vertical axis")
                wait_for(lambda: any(e.get("event") == "axis" and e.get("seat") == seat_id
                    and e.get("window") == 2 and e.get("value") == 10.0
                    for e in events(log_path)[modifier_start:]), "first seat shifted literal axis")
            agent.agent({"Key": {"evdev": 42, "state": "Released"}})
            second.agent("Reset")
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "key_up" and e.get("window") == 1
                    and e.get("key") == "b" for e in events(log_path)), "second controller B key")
            else:
                wait_for(lambda: any(e.get("event") == "key" and e.get("window") == 1
                    and e.get("seat") == second_seat and e.get("key") == 48
                    and "Released" in e.get("state", "") for e in events(log_path)), "second controller B key")
            if args.gpui:
                repeat_stop = len(events(log_path))
                time.sleep(0.2)
                assert not any(e.get("event") == "key_down" and e.get("held") is True
                    for e in events(log_path)[repeat_stop:]), "Reset left key repeat running"
            second.close()
            second = None
            if not args.gpui:
                wait_for(lambda: any(e.get("event") == "seat_removed" and e.get("seat") == second_seat
                                     for e in events(log_path)), "second controller removal")
            agent.agent({"Key": {"evdev": 46, "state": "Pressed"}})
            agent.agent({"Key": {"evdev": 46, "state": "Released"}})
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "key_up" and e.get("window") == 2
                    and e.get("key") == "c" for e in events(log_path)), "first controller retained after second reset")
            else:
                wait_for(lambda: any(e.get("event") == "key" and e.get("window") == 2
                    and e.get("seat") == seat_id and e.get("key") == 46
                    and "Released" in e.get("state", "") for e in events(log_path)), "first controller retained after second reset")
            assert focused_id(query) == human_id, "Second controller reset changed human focus"

            # Use a separate protocol client to lock only this nested compositor.
            before_lock = len(events(log_path))
            agent.agent({"Key": {"evdev": 42, "state": "Pressed"}})
            agent.agent({"Button": {"button": 272, "state": "Pressed"}})
            lock_log_path = log_path + ".lock"
            lock_binary = str(Path(__file__).parent / "target/debug/lock")
            with open(lock_log_path, "w") as lock_log:
                locker = subprocess.Popen([lock_binary], stdin=subprocess.PIPE, stdout=lock_log)
                wait_for(lambda: any(e.get("event") == "locked" for e in events(lock_log_path)), "session lock")
                if args.gpui:
                    wait_for(lambda: any(e.get("event") == "mouse_up" and e.get("window") == 2
                        for e in events(log_path)[before_lock:]), "GPUI lock button release")
                else:
                    wait_for(lambda: any(e.get("event") == "key" and e.get("seat") == seat_id and e.get("key") == 42
                        and "Released" in e.get("state", "") for e in events(log_path)[before_lock:]), "lock Shift release")
                    wait_for(lambda: any(e.get("event") == "button" and e.get("seat") == seat_id
                        and "Released" in e.get("state", "") for e in events(log_path)[before_lock:]), "lock pointer button release")
                before_rejected_key = len(events(log_path))
                try:
                    agent.agent({"Key": {"evdev": 32, "state": "Pressed"}})
                except RuntimeError:
                    pass
                else:
                    raise AssertionError("Locked compositor accepted agent key")
                time.sleep(0.1)
                assert not any(e.get("event") in ("key", "key_down", "key_up")
                    for e in events(log_path)[before_rejected_key:]), "Agent key reached client while locked"
                locker.stdin.write(b"unlock\n")
                locker.stdin.flush()
                locker.wait(timeout=5)
                locker = None
            wait_for(lambda: focused_id(query) == human_id, "human focus after unlock")
            agent.agent({"Focus": {"window_id": agent_id}})
            agent.agent({"Move": {"output": output, "x": x, "y": y}})
            # GPUI focuses its input element on this seat's mouse press.
            agent.agent({"Button": {"button": 272, "state": "Pressed"}})
            agent.agent({"Button": {"button": 272, "state": "Released"}})
            before_after_unlock = len(events(log_path))
            agent.agent({"Key": {"evdev": 18, "state": "Pressed"}})
            agent.agent({"Key": {"evdev": 18, "state": "Released"}})
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "key_up" and e.get("window") == 2
                    and e.get("key") == "e" for e in events(log_path)[before_after_unlock:]), "GPUI input after unlock")
            else:
                wait_for(lambda: any(e.get("event") == "key" and e.get("seat") == seat_id
                    and e.get("window") == 2 and e.get("key") == 18 and "Released" in e.get("state", "")
                    for e in events(log_path)[before_after_unlock:]), "agent input after unlock")
            # Disconnect while inputs remain down. These releases must reach the client
            # before the seat disappears, rather than relying on recorder teardown.
            agent.agent({"Key": {"evdev": 42, "state": "Pressed"}})
            agent.agent({"Key": {"evdev": 20, "state": "Pressed"}})
            agent.agent({"Button": {"button": 272, "state": "Pressed"}})
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "key_down" and e.get("window") == 2
                    and e.get("key_char") == "T" and e.get("held") is True
                    for e in events(log_path)), "first controller shifted T repeat")
            before_disconnect = len(events(log_path))
            agent.close()
            agent = None
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "mouse_up" and e.get("window") == 2
                                     for e in events(log_path)[before_disconnect:]), "GPUI disconnect button release")
            else:
                wait_for(lambda: any(e.get("event") == "seat_removed" and e.get("seat") == seat_id
                                     for e in events(log_path)[before_disconnect:]), "seat removal after disconnect")
                cleanup = events(log_path)[before_disconnect:]
                assert any(e.get("event") == "key" and e.get("key") == 42 and e.get("seat") == seat_id
                           and "Released" in e.get("state", "") for e in cleanup), "Disconnect left Shift held"
                assert any(e.get("event") == "button" and e.get("button") == 272 and e.get("seat") == seat_id
                           and "Released" in e.get("state", "") for e in cleanup), "Disconnect left left button held"
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "key_up" and e.get("window") == 2
                    and e.get("key") == "t" for e in events(log_path)[before_disconnect:]), "disconnect repeat key release")
                repeat_stop = len(events(log_path))
                time.sleep(0.2)
                assert not any(e.get("event") == "key_down" and e.get("held") is True
                    for e in events(log_path)[repeat_stop:]), "Seat removal left key repeat running"
            else:
                assert any(e.get("event") == "key" and e.get("key") == 20 and e.get("seat") == seat_id
                    and "Released" in e.get("state", "") for e in cleanup), "Disconnect left repeatable T held"
            assert focused_id(query) == human_id, "Disconnect changed human layout focus"
            time.sleep(0.1)
            after_cursor = screenshot(query, log_path + ".after.png")
            remaining_pixels = changed_pixels(before_cursor, after_cursor, x, y, scale)
            assert remaining_pixels == 0, f"Agent cursor region retained {remaining_pixels} changed pixels after disconnect"
            print(json.dumps({"result": "passed", "human_window": human_id, "agent_window": agent_id,
                              "agent_seat": seat_name, "events": log_path, "cursor_pixels": cursor_pixels,
                              "screenshots": [before_cursor, with_cursor, two_cursors, after_cursor]}))
    finally:
        if locker:
            if locker.stdin:
                try:
                    locker.stdin.write(b"unlock\n")
                    locker.stdin.flush()
                except BrokenPipeError:
                    pass
            try:
                locker.wait(timeout=5)
            except subprocess.TimeoutExpired:
                locker.terminate()
                locker.wait(timeout=5)
        if second:
            second.close()
        if agent:
            agent.close()
        query.close()
        if recorder:
            recorder.terminate()
            recorder.wait(timeout=5)


if __name__ == "__main__":
    main()
