#!/usr/bin/env python3
"""Drive the real dojo-agent MCP subprocess against nested fixture windows."""
import argparse
import base64
import concurrent.futures
import hashlib
import socket
import threading
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import time

from run import Peer, changed_pixels, events, focused_id, screenshot, wait_for


class Mcp:
    def __init__(self, binary, socket, name, stderr, idle_timeout=60):
        self.process = subprocess.Popen(
            [binary, "--socket", socket, "--name", name, "--idle-timeout-seconds", str(idle_timeout)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, bufsize=0,
        )
        self.next_id = 0
        result = self.request("initialize", {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "dojo-fixture", "version": "1"},
        })
        assert result["capabilities"]["tools"] == {}
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        tools = self.request("tools/list")["tools"]
        assert {t["name"] for t in tools} == {"desktop_context", "desktop_action", "desktop_release", "desktop_feedback", "desktop_launch", "desktop_screenshot"}

    def send(self, value):
        self.process.stdin.write(json.dumps(value).encode() + b"\n")
        self.process.stdin.flush()

    def request(self, method, params=None):
        reply = self.rpc(method, params)
        assert "error" not in reply, reply
        return reply["result"]

    def rpc(self, method, params=None):
        self.next_id += 1
        message = {"jsonrpc": "2.0", "id": self.next_id, "method": method}
        if params is not None:
            message["params"] = params
        self.send(message)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if not select.select([self.process.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
                break
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError(f"MCP exited: {self.process.poll()}")
            reply = json.loads(line)
            if reply.get("id") == self.next_id:
                return reply
        raise AssertionError(f"MCP request timed out: {method}")

    def tool(self, name, arguments=None, error=False):
        result = self.request("tools/call", {"name": name, "arguments": arguments or {}})
        assert bool(result.get("isError")) == error, result
        text = result["content"][0]["text"]
        return text if error else json.loads(text)

    def action(self, action, **arguments):
        return self.tool("desktop_action", {"action": action, **arguments})

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
                raise AssertionError("MCP did not exit on stdin EOF")
        self.process.stdout.close()
        assert self.process.returncode in (0, -signal.SIGKILL), self.process.returncode


def children(process):
    found = set()
    for path in Path(f"/proc/{process.pid}/task").glob("*/children"):
        try:
            found.update(int(child) for child in path.read_text().split())
        except FileNotFoundError:
            pass
    return found


def alive(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().split(")", 1)[1].split()[0] != "Z"
    except FileNotFoundError:
        return False


def lifecycle(args, query, human, target, stderr, servers, output, x, y):
    server = Mcp(args.server, args.socket, "mcp-idle", stderr, idle_timeout=2)
    servers.append(server)
    assert not children(server.process)
    server.action("reset")
    server.tool("desktop_release")
    assert server.tool("desktop_context")["worker_pid"] is None
    assert not children(server.process)

    def start():
        seat = server.action("focus", window_id=target)["seat_name"]
        context = server.tool("desktop_context")
        pid = context["worker_pid"]
        assert alive(pid)
        assert context["seat_name"] == seat
        return seat, pid

    def held_release(before):
        return any(e.get("window") == 2 and (
            e.get("event") == "key_up" and e.get("key") == "t" or
            e.get("event") == "key" and e.get("key") == 20 and "Released" in e.get("state", "")
        ) for e in events(args.log)[before:])

    seat, pid = start()
    server.tool("desktop_action", {"action": "key", "evdev": 0, "state": "tap"}, error=True)
    assert server.tool("desktop_context")["worker_pid"] == pid
    server.action("move", output=output, x=x, y=y)
    server.action("click", button="left")
    server.action("key", evdev=20, state="press")
    wait_for(lambda: any(e.get("window") == 2 and e.get("key") in (20, "t") for e in events(args.log)), "held key before idle")
    before = len(events(args.log))
    # Poll only /proc: no MCP request may help expiry or reaping happen.
    wait_for(lambda: not Path(f"/proc/{pid}").exists(), "idle service exit and reap")
    wait_for(lambda: held_release(before), "held key release on idle expiry")
    if not args.gpui:
        seat_id = next(e["seat"] for e in events(args.log) if e.get("name") == seat)
        wait_for(lambda: any(e.get("event") == "seat_removed" and e.get("seat") == seat_id for e in events(args.log)), "idle seat removal")
    assert not children(server.process)
    assert server.tool("desktop_context")["seat_name"] is None
    assert server.tool("desktop_context")["worker_pid"] is None

    # Keep the cursor crop clear of fixture text and the human cursor.
    logical = server.tool("desktop_context")["outputs"][output]["logical"]
    cursor_x, cursor_y = logical["width"] - 70, logical["height"] - 70
    directory = Path(args.log).parent
    baseline = screenshot(query, str(directory / "idle-cursor-baseline.png"))
    server.action("move", output=output, x=cursor_x, y=cursor_y)
    pid = server.tool("desktop_context")["worker_pid"]
    visible = screenshot(query, str(directory / "idle-cursor-visible.png"))
    assert changed_pixels(baseline, visible, cursor_x, cursor_y, logical["scale"]) > 0
    wait_for(lambda: not Path(f"/proc/{pid}").exists(), "visible cursor service expiry")
    assert server.tool("desktop_release") == {"released": True}
    removed = screenshot(query, str(directory / "idle-cursor-removed.png"))
    assert changed_pixels(baseline, removed, cursor_x, cursor_y, logical["scale"]) == 0

    # Context traffic must not renew the input lease.
    second_seat, pid = start()
    assert second_seat != seat
    deadline = time.monotonic() + 4
    while alive(pid) and time.monotonic() < deadline:
        server.tool("desktop_context")
        time.sleep(0.1)
    wait_for(lambda: not Path(f"/proc/{pid}").exists(), "expiry despite context traffic")

    _, pid = start()
    server.action("key", evdev=20, state="press")
    time.sleep(0.1)
    before = len(events(args.log))
    os.kill(pid, signal.SIGKILL)
    wait_for(lambda: not Path(f"/proc/{pid}").exists(), "crashed service reap")
    wait_for(lambda: held_release(before), "held key release on service crash")
    assert server.tool("desktop_context")["worker_pid"] is None

    _, pid = start()
    server.tool("desktop_release")
    server.tool("desktop_release")
    wait_for(lambda: not Path(f"/proc/{pid}").exists(), "explicit session release stops empty service")

    server.close()
    server = Mcp(args.server, args.socket, "mcp-crash", stderr, idle_timeout=60)
    servers.append(server)
    _, pid = start()
    server.action("key", evdev=20, state="press")
    time.sleep(0.1)
    before = len(events(args.log))
    disconnected = time.monotonic()
    server.process.kill()
    server.process.wait(timeout=5)
    wait_for(lambda: not alive(pid), "service exit after last adapter crash")
    assert time.monotonic() - disconnected < 2, "Service waited for session idle expiry after adapter crash"
    wait_for(lambda: held_release(before), "held key release on adapter crash")
    assert focused_id(query) == human


def shared_service(args, query, human, target, stderr, servers):
    barrier = threading.Barrier(2)
    first = Mcp(args.server, args.socket, "shared-first", stderr, idle_timeout=2)
    second = Mcp(args.server, args.socket, "shared-second", stderr, idle_timeout=60)
    servers.extend([first, second])
    def start(client, window):
        barrier.wait(timeout=5)
        return client.action("focus", window_id=window)["seat_name"]
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        a = pool.submit(start, first, target)
        b = pool.submit(start, second, human)
        first_seat, second_seat = a.result(), b.result()
    pid = first.tool("desktop_context")["worker_pid"]
    assert pid == second.tool("desktop_context")["worker_pid"]
    assert first_seat != second_seat
    key = hashlib.sha256(os.fsencode(str(Path(args.socket).resolve()))).hexdigest()[:24]
    service_socket = Path(os.environ["XDG_RUNTIME_DIR"]) / "dojo-desktop" / (key + ".sock")
    reported_path = subprocess.check_output([args.server, "--socket", args.socket, "--print-service-socket"], text=True).strip()
    assert Path(reported_path) == service_socket
    assert service_socket.stat().st_mode & 0o777 == 0o600
    assert service_socket.parent.stat().st_mode & 0o777 == 0o700
    def wire(message):
        with socket.socket(socket.AF_UNIX) as stream:
            stream.settimeout(3)
            stream.connect(str(service_socket))
            stream.sendall(json.dumps(message).encode() + b"\n")
            return json.loads(stream.makefile("rb").readline())
    assert "Err" in wire({"operation":"status"})
    assert "Err" in wire({"operation":"hello", "version":2, "name":"bad", "idle_timeout_seconds":2})
    assert "Err" in wire({"operation":"hello", "version":1, "name":"bad name", "idle_timeout_seconds":2})
    assert "Err" in wire({"operation":"hello", "version":1, "name":"valid", "idle_timeout_seconds":0})
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if first.tool("desktop_context")["seat_name"] is None:
            break
        time.sleep(0.1)
    assert first.tool("desktop_context")["worker_pid"] is None
    assert second.tool("desktop_context")["seat_name"] == second_seat
    assert second.tool("desktop_context")["worker_pid"] == pid and alive(pid)
    # An execution error must leave the other client's seat intact.
    first.action("focus", window_id=target)
    first.tool("desktop_action", {"action":"focus", "window_id":2**64-1}, error=True)
    assert second.tool("desktop_context")["seat_name"] == second_seat
    first.action("focus", window_id=target)
    first.action("key", evdev=20, state="press")
    time.sleep(0.1)
    before = len(events(args.log))
    first.process.kill()
    first.process.wait(timeout=5)
    wait_for(lambda: any(e.get("window") == 2 and (
        e.get("event") == "key_up" and e.get("key") == "t" or
        e.get("event") == "key" and e.get("key") == 20 and "Released" in e.get("state", "")
    ) for e in events(args.log)[before:]), "client crash releases its held key")
    assert alive(pid)
    assert second.tool("desktop_context")["seat_name"] == second_seat
    second.action("key", evdev=48, state="tap")
    second.tool("desktop_release")
    wait_for(lambda: not alive(pid), "last shared session release scales service to zero")
    assert not service_socket.exists()
    second.close()
    # The adapter that launched the service may leave while another client works.
    owner = Mcp(args.server, args.socket, "shared-owner", stderr)
    follower = Mcp(args.server, args.socket, "shared-follower", stderr)
    servers.extend([owner, follower])
    owner.action("focus", window_id=target)
    pid = owner.tool("desktop_context")["worker_pid"]
    assert pid in children(owner.process)
    follower_seat = follower.action("focus", window_id=human)["seat_name"]
    assert follower.tool("desktop_context")["worker_pid"] == pid
    owner.close()
    assert follower.tool("desktop_context")["seat_name"] == follower_seat
    follower.action("key", evdev=48, state="tap")
    assert alive(pid)
    follower.tool("desktop_release")
    wait_for(lambda: not alive(pid), "service exit after launcher adapter has exited")
    assert not service_socket.exists()
    follower.close()
    assert focused_id(query) == human


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    root = Path(__file__).resolve().parents[2]
    parser.add_argument("--socket", default=os.environ.get("NIRI_SOCKET"))
    parser.add_argument("--server", default=str(root / "target/debug/dojo-agent"))
    parser.add_argument("--recorder", default=str(Path(__file__).parent / "target/debug/dojo-agent-input-fixture"))
    parser.add_argument("--gpui", action="store_true")
    parser.add_argument("--lifecycle", action="store_true")
    parser.add_argument("--log", required=True)
    args = parser.parse_args()
    host_runtime = Path(os.environ["XDG_RUNTIME_DIR"])
    runtime = Path(args.log).resolve().parent / "mcp-runtime"
    runtime.mkdir(mode=0o700, exist_ok=True)
    assert runtime.resolve() != host_runtime.resolve(), "MCP fixture must not use the host runtime"
    assert not (runtime / "dojo.sock").exists(), "Unexpected shell control socket in fixture runtime"
    os.environ["WAYLAND_DISPLAY"] = str(host_runtime / os.environ["WAYLAND_DISPLAY"])
    os.environ["XDG_RUNTIME_DIR"] = str(runtime)
    Path(args.log + ".runtime.json").write_text(json.dumps({
        "runtime": str(runtime), "wayland_display": os.environ["WAYLAND_DISPLAY"],
        "niri_socket": args.socket, "host_shell_socket_accessible": False,
    }, indent=2))
    query = Peer(args.socket)
    servers = []
    with open(args.log, "w") as log, open(args.log + ".mcp-stderr", "w") as stderr:
        recorder = subprocess.Popen([args.recorder], stdout=log)
        try:
            def windows():
                found = [w for w in query.request("Windows")["Windows"]
                         if w.get("app_id") == "dojo-agent-input-fixture"]
                return sorted(found, key=lambda w: w["title"]) if len(found) == 2 else None

            found = wait_for(windows, "fixture windows")
            human, target = [w["id"] for w in found]
            for index, window in enumerate(found):
                window_id = window["id"]
                for command in [
                    {"MoveWindowToFloating": {"id": window_id}},
                    {"SetWindowWidth": {"id": window_id, "change": {"SetFixed": 260}}},
                    {"SetWindowHeight": {"id": window_id, "change": {"SetFixed": 200}}},
                    {"MoveFloatingWindow": {"id": window_id, "x": {"SetFixed": 20.0 + index * 310.0}, "y": {"SetFixed": 60.0}}},
                ]:
                    query.request({"Action": command})
            query.request({"Action": {"FocusWindow": {"id": human}}})
            wait_for(lambda: focused_id(query) == human, "human focus")
            time.sleep(0.3)
            server = Mcp(args.server, args.socket, "mcp-fixture", stderr)
            servers.append(server)
            context = server.tool("desktop_context")
            assert context["seat_name"] is None
            assert context["worker_pid"] is None
            assert not children(server.process)
            assert {human, target} <= {w["id"] for w in context["windows"]}
            assert context["outputs"]
            output_name = next(name for name, output in context["outputs"].items() if output["logical"])
            capture = server.request("tools/call", {"name": "desktop_screenshot", "arguments": {"output": output_name}})
            assert not capture.get("isError"), capture
            metadata = json.loads(capture["content"][0]["text"])
            image = capture["content"][1]
            assert image["type"] == "image" and image["mimeType"] == "image/png"
            png = base64.b64decode(image["data"], validate=True)
            assert png.startswith(b"\x89PNG\r\n\x1a\n") and png.endswith(b"IEND\xaeB`\x82")
            width = int.from_bytes(png[16:20], "big")
            height = int.from_bytes(png[20:24], "big")
            assert [metadata["image_width"], metadata["image_height"]] == [width, height]
            logical = context["outputs"][output_name]["logical"]
            assert metadata["logical_x_per_image_pixel"] == logical["width"] / width
            assert metadata["logical_y_per_image_pixel"] == logical["height"] / height
            Path(args.log + ".mcp-screenshot.png").write_bytes(png)
            assert focused_id(query) == human
            assert server.tool("desktop_context")["seat_name"] is None
            assert not children(server.process)
            server.tool("desktop_screenshot", {"output": "missing-output"}, error=True)
            invalid = server.rpc("tools/call", {"name": "desktop_action", "arguments": {"action": "key", "evdev": 30, "state": "typo"}})
            assert "error" in invalid or invalid["result"].get("isError"), invalid
            server.tool("desktop_action", {"action": "key", "evdev": 0, "state": "tap"}, error=True)
            assert server.tool("desktop_context")["seat_name"] is None
            assert not children(server.process)
            seat = server.action("focus", window_id=target)["seat_name"]
            assert server.tool("desktop_context")["seat_name"] == seat
            window = next(w for w in windows() if w["id"] == target)
            workspace = next(w for w in query.request("Workspaces")["Workspaces"] if w["id"] == window["workspace_id"])
            layout = window["layout"]
            tile_x, tile_y = layout["tile_pos_in_workspace_view"]
            offset_x, offset_y = layout["window_offset_in_tile"]
            width, height = layout["window_size"]
            server.action("move", output=workspace["output"], x=tile_x + offset_x + width / 2, y=tile_y + offset_y + height / 2)
            server.action("click", button="left")
            server.action("key", evdev=30, state="tap")
            server.action("scroll", horizontal=0.0, vertical=10.0)
            if args.gpui:
                wait_for(lambda: any(e.get("event") == "key_up" and e.get("key") == "a" and e.get("window") == 2 for e in events(args.log)), "GPUI MCP key")
                wait_for(lambda: any(e.get("event") == "mouse_down" and e.get("window") == 2 for e in events(args.log)), "GPUI MCP click")
                wait_for(lambda: any(e.get("event") == "scroll" and e.get("window") == 2 for e in events(args.log)), "GPUI MCP scroll")
            else:
                seat_id = wait_for(lambda: next((e["seat"] for e in events(args.log) if e.get("event") == "seat_name" and e.get("name") == seat), None), "MCP seat")
                wait_for(lambda: any(e.get("event") == "key" and e.get("key") == 30 and e.get("seat") == seat_id and e.get("window") == 2 for e in events(args.log)), "raw MCP key")
                wait_for(lambda: any(e.get("event") == "button" and e.get("seat") == seat_id and e.get("window") == 2 for e in events(args.log)), "raw MCP click")
                wait_for(lambda: any(e.get("event") == "axis" and e.get("seat") == seat_id for e in events(args.log)), "raw MCP scroll")
            assert focused_id(query) == human
            server.action("reset")
            assert server.tool("desktop_context")["seat_name"] == seat
            server.action("focus", window_id=target)
            server.action("key", evdev=42, state="press")
            server.tool("desktop_action", {"action": "focus", "window_id": 2**64 - 1}, error=True)
            assert server.tool("desktop_context")["seat_name"] is None
            new_seat = server.action("focus", window_id=target)["seat_name"]
            assert new_seat != seat
            second = Mcp(args.server, args.socket, "mcp-second", stderr)
            servers.append(second)
            second_seat = second.action("focus", window_id=human)["seat_name"]
            assert second.tool("desktop_context")["worker_pid"] == server.tool("desktop_context")["worker_pid"]
            assert second_seat != new_seat
            second.action("key", evdev=48, state="tap")
            second.tool("desktop_release")
            second.tool("desktop_release")
            assert server.tool("desktop_context")["seat_name"] == new_seat
            server.action("key", evdev=20, state="press")
            wait_for(lambda: any(e.get("event") in ("key", "key_down") and e.get("window") == 2 and e.get("key") in (20, "t") for e in events(args.log)), "held key before EOF")
            before = len(events(args.log))
            worker_pid = server.tool("desktop_context")["worker_pid"]
            server.close()
            wait_for(lambda: not Path(f"/proc/{worker_pid}").exists(), "service exits on last MCP EOF")
            wait_for(lambda: any(e.get("window") == 2 and (
                e.get("event") == "key_up" and e.get("key") == "t" or
                e.get("event") == "key" and e.get("key") == 20 and "Released" in e.get("state", "")
            ) for e in events(args.log)[before:]), "held key release on MCP EOF")
            assert focused_id(query) == human
            if args.lifecycle:
                shared_service(args, query, human, target, stderr, servers)
                lifecycle(args, query, human, target, stderr, servers,
                          workspace["output"], tile_x + offset_x + width / 2,
                          tile_y + offset_y + height / 2)
            print(json.dumps({"passed": True, "transport": "MCP stdio", "gpui": args.gpui, "lifecycle": args.lifecycle, "events": args.log}))
        finally:
            for server in servers:
                server.close()
            recorder.terminate()
            recorder.wait(timeout=5)
            query.close()


if __name__ == "__main__":
    main()
