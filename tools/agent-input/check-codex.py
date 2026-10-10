#!/usr/bin/env python3
"""Verify project MCP discovery through Codex app-server without running a model."""
import json
from pathlib import Path
import queue
import subprocess
import threading
import time


def main():
    root = Path(__file__).resolve().parents[2]
    replies = queue.Queue()
    process = subprocess.Popen(
        ["codex", "app-server"], cwd=root,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        text=True, bufsize=1,
    )

    def read():
        for line in process.stdout:
            replies.put(json.loads(line))
        replies.put(None)

    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    next_id = 0

    def request(method, params):
        nonlocal next_id
        next_id += 1
        process.stdin.write(json.dumps({"id": next_id, "method": method, "params": params}) + "\n")
        process.stdin.flush()
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            reply = replies.get(timeout=max(0.01, deadline - time.monotonic()))
            assert reply is not None, "Codex app-server exited"
            if reply.get("id") == next_id:
                assert "error" not in reply, reply
                return reply["result"]
        raise AssertionError(f"Timed out: {method}")

    try:
        request("initialize", {
            "clientInfo": {"name": "dojo-mcp-check", "version": "1"},
            "capabilities": {"experimentalApi": True},
        })
        config = request("config/read", {"cwd": str(root)})["config"]
        assert "dojo-desktop" in config.get("mcp_servers", {}), "Project configuration is not loaded"
        # The installed T3 bundle supplies this whole-table thread override. A
        # disabled placeholder verifies merging without connecting to a fake URL.
        thread = request("thread/start", {
            "cwd": str(root), "ephemeral": True,
            "config": {"mcp_servers": {"t3-code": {
                "url": "http://127.0.0.1:1/mcp", "enabled": False,
            }}},
        })["thread"]["id"]
        status = request("mcpServerStatus/list", {
            "threadId": thread, "serverName": "dojo-desktop", "detail": "toolsAndAuthOnly",
        })
        server = next(item for item in status["data"] if item["name"] == "dojo-desktop")
        tools = set(server["tools"])
        assert tools == {"desktop_context", "desktop_action", "desktop_release"}, server
        result = request("mcpServer/tool/call", {
            "threadId": thread, "server": "dojo-desktop", "tool": "desktop_context", "arguments": {},
        })
        assert not result.get("isError"), result
        context = json.loads(result["content"][0]["text"])
        assert context["seat_name"] is None
        print(json.dumps({
            "passed": True, "provider": "Codex app-server",
            "t3_override_merge": True, "tools": sorted(tools),
            "windows": len(context["windows"]), "outputs": len(context["outputs"]),
            "model_calls": 0,
        }))
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        reader.join(timeout=2)
        process.stdin.close()
        process.stdout.close()


if __name__ == "__main__":
    main()
