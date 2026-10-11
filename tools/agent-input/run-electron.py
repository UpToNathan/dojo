#!/usr/bin/env python3
"""Verify shared input against an isolated copy of the installed Electron runtime."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time

from run import Peer, wait_for, screenshot

spec = importlib.util.spec_from_file_location("mcp_driver", "tools/agent-input/run-mcp.py")
mcp_driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mcp_driver)

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--electron", required=True, help="Installed T3/Electron executable")
parser.add_argument("--niri", default="vendor/niri/target/release/niri")
args = parser.parse_args()
root = Path(tempfile.mkdtemp(prefix="dojo-electron-coordination."))
runtime = root / "runtime"
runtime.mkdir()
source = Path(args.electron).resolve()
for path in source.parent.iterdir():
    if path.name == "resources":
        continue
    destination = runtime / path.name
    if path == source:
        shutil.copy2(path, destination)
    else:
        destination.symlink_to(path)
app = runtime / "resources" / "app"
app.mkdir(parents=True)
(app / "package.json").write_text(json.dumps({
    "name": "dojo-seat-fixture", "version": "1.0.0", "main": "main.cjs",
}))
shutil.copyfile("tools/agent-input/electron-fixture.cjs", app / "main.cjs")

niri = fixture = server = query = observer = raw_agent = None
observed_events = []


def page_messages():
    return [json.loads(line[5:])[1] for line in (root / "fixture.log").read_text().splitlines()
            if line.startswith("PAGE ")]


def read_coordination(revision):
    for _ in range(100):
        event = json.loads(observer.stream.readline())
        observed_events.append(event)
        coordination = event.get("InputCoordinationChanged", {}).get("coordination")
        if coordination and coordination["revision"] == revision:
            return coordination
    raise AssertionError(f"No ownership event for revision {revision}")


try:
    nlog = open(root / "niri.log", "w")
    niri = subprocess.Popen(
        [args.niri, "--config", "tools/agent-input/nested.kdl"],
        env=dict(os.environ, DOJO_AGENT_INPUT="1"), stdout=nlog, stderr=nlog,
    )

    def connection():
        assert niri.poll() is None, "Nested compositor exited"
        text = (root / "niri.log").read_text()
        display = re.search(r"listening on Wayland socket: (\S+)", text)
        socket = re.search(r"IPC listening on: (\S+)", text)
        return (display[1], socket[1]) if display and socket else None

    display, socket = wait_for(connection, "nested compositor")
    query = Peer(socket)
    elog = open(root / "electron.log", "w")
    out = open(root / "fixture.log", "w")
    env = dict(os.environ, WAYLAND_DISPLAY=display, NIRI_SOCKET=socket,
               WAYLAND_DEBUG="client", DOJO_ELECTRON_PROFILE=str(root / "profile"))
    env.pop("ELECTRON_RUN_AS_NODE", None)
    fixture = subprocess.Popen(
        [str(runtime / source.name), "--ozone-platform=wayland", "--no-first-run"],
        env=env, stdout=out, stderr=elog,
    )

    def fixture_window():
        assert fixture.poll() is None, "Electron fixture exited"
        return next((w for w in query.request("Windows")["Windows"]
                     if w["title"] == "Dojo Electron seat fixture"), None)

    window = wait_for(fixture_window, "Electron window")
    wait_for(lambda: "FIXTURE_READY" in (root / "fixture.log").read_text(), "Electron page loaded")
    time.sleep(.5)
    query.request({"Action": {"MoveWindowToFloating": {"id": window["id"]}}})
    query.request({"Action": {"MoveFloatingWindow": {
        "id": window["id"], "x": {"SetFixed": 50.0}, "y": {"SetFixed": 50.0},
    }}})
    query.request({"Action": {"FocusWindow": {"id": window["id"]}}})
    time.sleep(.4)
    outputs = query.request("Outputs")["Outputs"]
    output = next(iter(outputs))
    logical = outputs[output]["logical"]

    def human_pointer(x, y, click=False):
        command = ["tools/agent-input/target/debug/human-pointer", str(x), str(y),
                   str(logical["width"]), str(logical["height"])]
        if click:
            command.append("click")
        subprocess.run(command, env=env, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    human_pointer(logical["width"] - 2, logical["height"] - 2)
    baseline = screenshot(query, root / "before-cursor.png")
    observer = Peer(socket)
    observer.request("EventStream")
    initial = query.request("InputCoordination")["InputCoordination"]
    assert read_coordination(initial["revision"])["human_owned_windows"] == []

    adapter = root / "dojo-agent"
    shutil.copy2(Path("target/debug/dojo-agent").resolve(), adapter)
    server = mcp_driver.Mcp(str(adapter), socket, "electron-repro", sys.stderr)
    adapter.unlink()  # Exercise worker startup while its parent's executable has been replaced.
    focused = server.action("focus", window_id=window["id"])
    assert focused["handled"], focused
    time.sleep(.5)
    assert server.action("move", output=output, x=150, y=150)["handled"]
    assert server.action("click", button="left")["handled"]
    server.action("key", evdev=42, state="press")
    server.action("key", evdev=30, state="tap")
    server.action("key", evdev=42, state="release")
    server.action("scroll", horizontal=0, vertical=40)
    server.action("key", evdev=48, state="press")
    before_takeover = screenshot(query, root / "before-takeover.png")
    before_context = server.tool("desktop_context")

    human_pointer(250, 150)
    coordination = server.tool("desktop_context")["coordination"]
    assert coordination["human_owned_windows"] == [window["id"]], coordination
    assert read_coordination(coordination["revision"]) == coordination
    wait_for(lambda: 'AGENT_UP {"key":"b","shift":false}' in page_messages(), "held B release on takeover")
    paused = server.action("focus", window_id=window["id"])
    assert paused["paused"] and not paused["handled"], paused
    blocked = server.action("focus", window_id=window["id"])
    assert blocked["paused"] and "human owns" in blocked["reason"], blocked
    after_takeover = screenshot(query, root / "after-takeover.png")

    def agent_pixels(path):
        command = ["tools/agent-input/target/debug/pixel-diff", "--color", str(path),
                   str(round(150 * logical["scale"])), str(round(150 * logical["scale"])),
                   str(round(48 * logical["scale"])), "40", "200", "240"]
        return json.loads(subprocess.check_output(command, text=True))["matching_pixels"]

    assert agent_pixels(baseline) == 0
    assert agent_pixels(before_takeover) > 0
    cursor_pixels = agent_pixels(after_takeover)
    assert cursor_pixels > 0, "Human takeover hid the agent cursor"
    after_context = server.tool("desktop_context")
    assert after_context["worker_pid"] == before_context["worker_pid"]
    assert after_context["seat_name"] == before_context["seat_name"]
    human_pointer(250, 150, click=True)
    wait_for(lambda: page_messages().count("AGENT_CLICK_RECEIVED") >= 2, "human click after takeover")
    human_pointer(logical["width"] - 2, logical["height"] - 2)
    assert read_coordination(coordination["revision"] + 1)["human_owned_windows"] == []
    paused = server.action("focus", window_id=window["id"])
    assert paused["paused"], paused
    assert server.action("focus", window_id=window["id"])["handled"]
    server.action("key", evdev=48, state="tap")
    server.tool("desktop_release")
    time.sleep(.5)
    after_release = screenshot(query, root / "after-release.png")
    assert agent_pixels(after_release) == 0
    assert server.tool("desktop_context")["worker_pid"] is None

    raw_agent = Peer(socket)
    raw_agent.agent({"Create": {"name": "held-pointer"}})
    raw_agent.agent({"Focus": {"window_id": window["id"]}})
    raw_agent.agent({"Move": {"output": output, "x": 150.0, "y": 150.0}})
    releases_before = page_messages().count("AGENT_MOUSE_UP 0")
    raw_agent.agent({"Button": {"button": 272, "state": "Pressed"}})
    human_pointer(250, 150)
    wait_for(lambda: page_messages().count("AGENT_MOUSE_UP 0") > releases_before,
             "held mouse release before human takeover")
    assert read_coordination(coordination["revision"] + 2)["human_owned_windows"] == [window["id"]]
    raw_agent.close()
    raw_agent = None
    human_pointer(logical["width"] - 2, logical["height"] - 2)
    assert read_coordination(coordination["revision"] + 3)["human_owned_windows"] == []

    protocol = (root / "electron.log").read_text()
    seats = re.findall(r'global\((\d+), "wl_seat", (\d+)\)', protocol)
    binds = re.findall(r'bind\((\d+), "wl_seat",', protocol)
    messages = page_messages()
    assert messages.count("AGENT_CLICK_RECEIVED") >= 2
    assert 'AGENT_KEY {"key":"A","shift":true}' in messages, messages
    assert 'AGENT_UP {"key":"Shift","shift":false}' in messages, messages
    assert any(message.startswith("AGENT_WHEEL ") for message in messages), messages
    assert re.search(r'wl_pointer#\d+\.axis\(\d+, 0, 40\.0+\)', protocol)
    assert messages.count('AGENT_UP {"key":"b","shift":false}') >= 2, messages
    assert len(set(binds)) == 1, binds
    (root / "ownership-events.json").write_text(json.dumps(observed_events, indent=2))
    (root / "page-events.json").write_text(json.dumps(messages, indent=2))
    print(json.dumps({"announced_seats": seats, "bound_seats": binds, "click_received": True,
                      "takeover_cursor_pixels": cursor_pixels, "coordination": coordination,
                      "evidence": str(root)}))
finally:
    if server:
        server.close()
    for peer in (raw_agent, query, observer):
        if peer:
            peer.close()
    for process in (fixture, niri):
        if process and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
