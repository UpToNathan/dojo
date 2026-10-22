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
SHUTDOWN = "/org/freedesktop/login1: org.freedesktop.login1.Manager.PrepareForShutdown ({},)"
SHUTDOWN_WITH_TYPE = (
    "/org/freedesktop/login1: org.freedesktop.login1.Manager.PrepareForShutdownWithMetadata"
    " ({}, {{'type': <'{}'>}})"
)

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
what=${1#--what=}
echo "taken:$what" >> "$STATE/log"
while [ "${1#--}" != "$1" ]; do shift; done
trap 'echo "released:$what" >> "$STATE/log"; exit 0' TERM
"$@" &
wait
""",
    "dojo": """#!/bin/sh
echo "$1:$2" >> "$STATE/log"
[ -e "$STATE/stalled" ] && exec sleep 30
if [ -e "$STATE/plays" ]; then sleep 0.3; echo played >> "$STATE/log"; fi
exit 0
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


def run_bridge(
    events,
    never_locks=False,
    marker_only=False,
    stale_marker=False,
    stalled_shell=False,
    playing_shell=False,
    missing_shell=False,
    wait_for=None,
):
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
        if stalled_shell:
            (root / "stalled").touch()
        if missing_shell:
            (root / "dojo").unlink()
        if playing_shell:
            (root / "plays").touch()
        if stale_marker:
            (root / "locked-at").touch()
            time.sleep(0.02)
        environment = {
            "PATH": f"{root}:/usr/bin",
            "STATE": str(root),
            "DOJO_LOCK_NOW": str(root / "lock-now"),
            "DOJO_DESKTOP": str(root / "dojo"),
            "DOJO_FAREWELL_BOUND": "0.5",
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


def kind(log, what):
    """The log entries about one inhibitor kind, and the lock; the two kinds start in either order."""
    return [
        entry
        for entry in log
        if entry in (f"taken:{what}", f"released:{what}") or entry == "lock"
    ]


def of(log, prefix):
    return [entry for entry in log if entry.startswith(prefix)]


FAREWELL_SUSPEND = "--farewell:suspend"
FAREWELL_SHUTDOWN = "--farewell:shutdown"
FAREWELL_RESTART = "--farewell:restart"


class BridgeTests(unittest.TestCase):
    def test_both_inhibitors_are_held_while_idle(self):
        log, _ = run_bridge([], wait_for=lambda lines: len(lines) >= 2)
        self.assertCountEqual(log, ["taken:sleep", "taken:shutdown"])

    def test_lock_session_locks_only_for_its_own_session(self):
        log, _ = run_bridge(
            [OTHER_LOCK, LOCK],
            wait_for=lambda lines: lines.count("lock") >= 1,
        )
        self.assertEqual(log.count("lock"), 1)
        self.assertEqual(of(log, "--farewell"), [])

    def test_suspend_locks_then_releases_the_inhibitor_and_retakes_it_on_resume(self):
        log, _ = run_bridge(
            [SLEEP.format("true"), SLEEP.format("false")],
            wait_for=lambda lines: lines.count("taken:sleep") >= 2,
        )
        self.assertEqual(
            kind(log, "sleep"), ["taken:sleep", "lock", "released:sleep", "taken:sleep"]
        )

    def test_suspend_asks_the_shell_for_the_short_farewell_beside_the_lock(self):
        log, _ = run_bridge(
            [SLEEP.format("true")],
            wait_for=lambda lines: "released:sleep" in lines,
        )
        self.assertEqual(of(log, "--farewell"), [FAREWELL_SUSPEND])
        self.assertEqual(log.count("lock"), 1)
        self.assertLess(log.index("lock"), log.index("released:sleep"))
        self.assertLess(log.index(FAREWELL_SUSPEND), log.index("released:sleep"))

    def test_suspend_is_released_by_the_marker_when_logind_never_reports_a_hint(self):
        log, stderr = run_bridge(
            [SLEEP.format("true")],
            marker_only=True,
            wait_for=lambda lines: "released:sleep" in lines,
        )
        self.assertEqual(kind(log, "sleep")[:3], ["taken:sleep", "lock", "released:sleep"])
        self.assertNotIn("still unlocked", stderr)

    def test_a_marker_left_by_an_earlier_lock_does_not_count(self):
        log, stderr = run_bridge(
            [SLEEP.format("true")],
            never_locks=True,
            stale_marker=True,
            wait_for=lambda lines: "released:sleep" in lines,
        )
        self.assertEqual(kind(log, "sleep")[:3], ["taken:sleep", "lock", "released:sleep"])
        self.assertIn("still unlocked", stderr)

    def test_suspend_is_released_when_the_lock_never_arrives(self):
        log, stderr = run_bridge(
            [SLEEP.format("true")],
            never_locks=True,
            wait_for=lambda lines: "released:sleep" in lines,
        )
        self.assertEqual(kind(log, "sleep")[:3], ["taken:sleep", "lock", "released:sleep"])
        self.assertIn("still unlocked", stderr)

    def test_a_stalled_shell_cannot_hold_suspend_past_the_farewell_bound(self):
        started = time.monotonic()
        log, stderr = run_bridge(
            [SLEEP.format("true")],
            stalled_shell=True,
            wait_for=lambda lines: "released:sleep" in lines,
        )
        self.assertIn("released:sleep", log)
        self.assertLess(time.monotonic() - started, 2.5)
        self.assertIn("farewell suspend ended early", stderr)

    def test_shutdown_plays_the_farewell_then_releases_the_inhibitor(self):
        log, _ = run_bridge(
            [SHUTDOWN.format("true")],
            wait_for=lambda lines: "released:shutdown" in lines,
        )
        self.assertEqual(kind(log, "shutdown"), ["taken:shutdown", "released:shutdown"])
        self.assertEqual(of(log, "--farewell"), [FAREWELL_SHUTDOWN])
        self.assertLess(log.index(FAREWELL_SHUTDOWN), log.index("released:shutdown"))
        self.assertNotIn("lock", log)
        self.assertNotIn("released:sleep", log)

    def test_the_farewell_is_awaited_until_it_has_played(self):
        log, _ = run_bridge(
            [SHUTDOWN.format("true")],
            playing_shell=True,
            wait_for=lambda lines: "released:shutdown" in lines,
        )
        self.assertLess(log.index("played"), log.index("released:shutdown"))

    def test_a_restart_is_told_from_a_poweroff_by_the_metadata_logind_sends_first(self):
        for kind_of, farewell in [
            ("reboot", FAREWELL_RESTART),
            ("soft-reboot", FAREWELL_RESTART),
            ("power-off", FAREWELL_SHUTDOWN),
            ("halt", FAREWELL_SHUTDOWN),
            ("kexec", FAREWELL_SHUTDOWN),
        ]:
            with self.subTest(kind_of):
                log, _ = run_bridge(
                    [SHUTDOWN_WITH_TYPE.format("true", kind_of), SHUTDOWN.format("true")],
                    wait_for=lambda lines: "released:shutdown" in lines,
                )
                self.assertEqual(of(log, "--farewell"), [farewell])

    def test_a_reboot_does_not_leak_into_the_next_shutdown(self):
        log, _ = run_bridge(
            [
                SHUTDOWN_WITH_TYPE.format("true", "reboot"),
                SHUTDOWN.format("true"),
                SHUTDOWN.format("false"),
                SHUTDOWN.format("true"),
            ],
            wait_for=lambda lines: lines.count("released:shutdown") >= 2,
        )
        self.assertEqual(of(log, "--farewell"), [FAREWELL_RESTART, FAREWELL_SHUTDOWN])

    def test_a_cancelled_shutdown_retakes_the_inhibitor(self):
        log, _ = run_bridge(
            [SHUTDOWN.format("true"), SHUTDOWN.format("false")],
            wait_for=lambda lines: lines.count("taken:shutdown") >= 2,
        )
        self.assertEqual(
            kind(log, "shutdown"),
            ["taken:shutdown", "released:shutdown", "taken:shutdown"],
        )

    def test_a_stalled_shell_cannot_hold_shutdown_past_the_farewell_bound(self):
        started = time.monotonic()
        log, stderr = run_bridge(
            [SHUTDOWN.format("true")],
            stalled_shell=True,
            wait_for=lambda lines: "released:shutdown" in lines,
        )
        self.assertIn("released:shutdown", log)
        self.assertLess(time.monotonic() - started, 2.5)
        self.assertIn("farewell shutdown ended early", stderr)

    def test_an_absent_shell_still_releases_shutdown(self):
        log, stderr = run_bridge(
            [SHUTDOWN.format("true")],
            missing_shell=True,
            wait_for=lambda lines: "released:shutdown" in lines,
        )
        self.assertIn("released:shutdown", log)
        self.assertIn("farewell shutdown ended early", stderr)


if __name__ == "__main__":
    unittest.main()
