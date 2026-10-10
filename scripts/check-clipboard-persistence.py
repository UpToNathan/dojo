#!/usr/bin/env python3
"""Live Wayland check; replaces the current clipboard with a test value."""

import subprocess
import time
import uuid


def main():
    value = f"dojo-clipboard-probe-{uuid.uuid4()}".encode()
    source = subprocess.Popen(
        ["wl-copy", "--foreground", "--type", "text/plain"],
        stdin=subprocess.PIPE,
    )
    try:
        source.stdin.write(value)
        source.stdin.close()
        deadline = time.monotonic() + 3
        while True:
            copied = subprocess.run(
                ["wl-paste", "--no-newline"], capture_output=True, timeout=3
            )
            if copied.returncode == 0 and copied.stdout == value:
                break
            if time.monotonic() >= deadline:
                raise SystemExit("FAIL: test value was not copied")
            time.sleep(0.05)
        # Allow the clipboard manager to receive and retain the selection.
        time.sleep(0.5)
        if source.poll() is None:
            source.terminate()
        source.wait(timeout=3)
        time.sleep(0.2)
        pasted = subprocess.run(
            ["wl-paste", "--no-newline"], capture_output=True, timeout=3
        )
        if pasted.returncode != 0 or pasted.stdout != value:
            raise SystemExit(
                "FAIL: clipboard value disappeared after the source process exited"
            )
        print("PASS: clipboard value survives the source process exiting")
    finally:
        if source.poll() is None:
            source.terminate()
            source.wait(timeout=3)


if __name__ == "__main__":
    main()
