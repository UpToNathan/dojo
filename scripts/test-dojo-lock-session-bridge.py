#!/usr/bin/env python3
"""Drive dojo-lock-session-bridge against stand-in logind tools and read its log."""

import os
import signal
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

BRIDGE = Path(__file__).with_name("dojo-lock-session-bridge")
SESSION = "/org/freedesktop/login1/session/c9"
LOCK = f"{SESSION}: org.freedesktop.login1.Session.Lock ()"
OTHER_LOCK = "/org/freedesktop/login1/session/c1: org.freedesktop.login1.Session.Lock ()"
SLEEP = "/org/freedesktop/login1: org.freedesktop.login1.Manager.PrepareForSleep ({},)"

TOOLS = {
    "loginctl": """#!/bin/sh
case "$1" in
  show-user) echo c9 ;;
  show-session) cat "$STATE/hint" 2>/dev/null || echo no ;;
esac
""",
    "busctl": f"""#!/bin/sh
echo 'o "{SESSION}"'
""",
    "gdbus": """#!/bin/sh
while IFS= read -r line; do printf '%s\\n' "$line"; sleep 0.05; done < "$STATE/events"
exec sleep 30
""",
    "systemd-inhibit": """#!/bin/sh
echo taken >> "$STATE/log"
while [ "${1#--}" != "$1" ]; do shift; done
trap 'echo released >> "$STATE/log"; exit 0' TERM
"$@" &
wait
""",
    "lock-now": """#!/bin/sh
echo lock >> "$STATE/log"
[ -e "$STATE/never-locks" ] && exit 0
if [ -e "$STATE/marker-only" ]; then
  sleep 0.15
  touch "$STATE/locked-at"
else
  echo yes > "$STATE/hint"
fi
""",
}


def run_bridge(events, never_locks=False, marker_only=False, stale_marker=False, wait_for=None):
    """Returns the tools' log as it stood when wait_for held, and the bridge's stderr."""
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        for name, text in TOOLS.items():
            tool = root / name
            tool.write_text(text)
            tool.chmod(0o755)
        (root / "events").write_text("".join(f"{event}\n" for event in events))
        if never_locks:
            (root / "never-locks").touch()
        if marker_only:
            (root / "marker-only").touch()
        if stale_marker:
            (root / "locked-at").touch()
            time.sleep(0.02)
        environment = {
            "PATH": f"{root}:/usr/bin",
            "STATE": str(root),
            "DOJO_LOCK_NOW": str(root / "lock-now"),
            "DOJO_LOCK_HINT_POLLS": "3",
            "DOJO_LOCK_MARKER": str(root / "locked-at"),
        }
        process = subprocess.Popen(
            ["bash", str(BRIDGE)],
            env=environment,
            stdin=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        log = root / "log"
        deadline = time.monotonic() + 10
        lines = []
        try:
            while time.monotonic() < deadline:
                lines = log.read_text().split() if log.exists() else []
                if wait_for(lines):
                    break
                time.sleep(0.05)
        finally:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=5)
            stderr = process.stderr.read().decode()
            process.stderr.close()
        return lines, stderr


class BridgeTests(unittest.TestCase):
    def test_lock_session_locks_only_for_its_own_session(self):
        log, _ = run_bridge(
            [OTHER_LOCK, LOCK],
            wait_for=lambda lines: lines.count("lock") >= 1,
        )
        self.assertEqual(log[:2], ["taken", "lock"])
        self.assertEqual(log.count("lock"), 1)

    def test_suspend_locks_then_releases_the_inhibitor_and_retakes_it_on_resume(self):
        log, _ = run_bridge(
            [SLEEP.format("true"), SLEEP.format("false")],
            wait_for=lambda lines: lines.count("taken") >= 2,
        )
        self.assertEqual(log, ["taken", "lock", "released", "taken"])

    def test_suspend_is_released_by_the_marker_when_logind_never_reports_a_hint(self):
        log, stderr = run_bridge(
            [SLEEP.format("true")],
            marker_only=True,
            wait_for=lambda lines: "released" in lines,
        )
        self.assertEqual(log[:3], ["taken", "lock", "released"])
        self.assertNotIn("still unlocked", stderr)

    def test_a_marker_left_by_an_earlier_lock_does_not_count(self):
        log, stderr = run_bridge(
            [SLEEP.format("true")],
            never_locks=True,
            stale_marker=True,
            wait_for=lambda lines: "released" in lines,
        )
        self.assertEqual(log[:3], ["taken", "lock", "released"])
        self.assertIn("still unlocked", stderr)

    def test_suspend_is_released_when_the_lock_never_arrives(self):
        log, stderr = run_bridge(
            [SLEEP.format("true")],
            never_locks=True,
            wait_for=lambda lines: "released" in lines,
        )
        self.assertEqual(log[:3], ["taken", "lock", "released"])
        self.assertIn("still unlocked", stderr)


if __name__ == "__main__":
    unittest.main()
