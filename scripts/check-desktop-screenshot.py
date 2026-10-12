#!/usr/bin/env python3
"""Capture through the real desktop MCP and check image/coordinate metadata."""
import argparse
import base64
import json
import importlib.util
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(root / "tools/agent-input"))
spec = importlib.util.spec_from_file_location("run_mcp", root / "tools/agent-input/run-mcp.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
Mcp = module.Mcp


def clipboard():
    return subprocess.run(["wl-paste", "--no-newline"], capture_output=True, timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True)
    parser.add_argument("--image", type=Path, required=True)
    args = parser.parse_args()
    before_clipboard = clipboard()
    server = Mcp(str(root / "target/debug/dojo-agent"), os.environ["NIRI_SOCKET"], "screenshot-check", sys.stderr)
    try:
        before = server.tool("desktop_context")
        result = server.request("tools/call", {"name": "desktop_screenshot", "arguments": {"output": args.output}})
        assert not result.get("isError"), result
        metadata = json.loads(result["content"][0]["text"])
        image = result["content"][1]
        assert image["type"] == "image" and image["mimeType"] == "image/png"
        png = base64.b64decode(image["data"], validate=True)
        assert png.startswith(b"\x89PNG\r\n\x1a\n") and png.endswith(b"IEND\xaeB`\x82")
        width, height = [int.from_bytes(png[start:start + 4], "big") for start in (16, 20)]
        assert [width, height] == [metadata["image_width"], metadata["image_height"]]
        logical = before["outputs"][args.output]["logical"]
        assert metadata["logical_x_per_image_pixel"] == logical["width"] / width
        assert metadata["logical_y_per_image_pixel"] == logical["height"] / height
        server.tool("desktop_screenshot", {"output": "missing-output"}, error=True)
        after = server.tool("desktop_context")
        assert after["seat_name"] is None and after["worker_pid"] is None
        assert [w["id"] for w in before["windows"] if w["is_focused"]] == [w["id"] for w in after["windows"] if w["is_focused"]]
        after_clipboard = clipboard()
        assert (before_clipboard.returncode, before_clipboard.stdout) == (after_clipboard.returncode, after_clipboard.stdout), "clipboard changed"
        args.image.write_bytes(png)
        print(json.dumps({"result": "PASS", "image": str(args.image), **metadata}))
    finally:
        server.close()


if __name__ == "__main__":
    main()
