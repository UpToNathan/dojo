#!/usr/bin/env python3
"""Check `dojo --farewell` and `dojo --leave` against a shell in a nested Niri, with recorded actions.

Never touches the live session: the shell runs in a nested Niri with a private runtime
directory, home, session bus and a fake logind, and it is a `record-session-actions` build, so
no power action is ever performed; each one is appended to actions.log instead. Build first:

    devenv shell -- cargo build --bin dojo --example fake_logind --features record-session-actions
    devenv shell -- python3 tools/agent-input/run-farewell.py --out DIR

Needs a host Wayland session to nest in.
"""
import argparse
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))

spec = importlib.util.spec_from_file_location("handoff", HERE / "run-lock-handoff.py")
handoff = importlib.util.module_from_spec(spec)
spec.loader.exec_module(handoff)

HARD_CAP = 5.5


class Smoke:
    def __init__(self, rig):
        self.rig = rig
        self.actions = rig.out / "actions.log"
        self.failures = []

    def recorded(self):
        return self.actions.read_text().split() if self.actions.exists() else []

    def reset(self):
        """Let any cover come down, then forget what was recorded."""
        time.sleep(HARD_CAP)
        self.actions.unlink(missing_ok=True)

    def dojo(self, *argv, timeout=10):
        started = time.monotonic()
        subprocess.run([str(self.rig.args.shell), *argv], env=self.rig.client_env(), check=True,
                       stdout=subprocess.DEVNULL, timeout=timeout)
        return time.monotonic() - started

    def expect(self, ok, description, detail=""):
        print(("PASS " if ok else "FAIL ") + description + (f" ({detail})" if detail else ""))
        if not ok:
            self.failures.append(description)

    def run(self):
        self.reset()
        took = self.dojo("--farewell", "shutdown")
        self.expect(0.8 < took < 2.5, "a farewell returns when its closing has played", f"{took:.2f}s")
        self.expect(self.recorded() == [], "a farewell performs no action", str(self.recorded()))

        again = self.dojo("--farewell", "shutdown")
        self.expect(again < 0.4, "a second farewell during the cover returns at once", f"{again:.2f}s")

        self.reset()
        self.dojo("--leave", "shutdown")
        waited = self.dojo("--farewell", "shutdown")
        self.expect(waited > 0.5, "a farewell for the shell's own closing waits for its timeline", f"{waited:.2f}s")
        self.expect(self.recorded() == ["shutdown"], "the shell's own shutdown ran once, not twice", str(self.recorded()))
        after = self.dojo("--farewell", "shutdown")
        self.expect(after < 0.4, "the farewell logind sends after the action returns at once", f"{after:.2f}s")
        self.expect(self.recorded() == ["shutdown"], "still one shutdown", str(self.recorded()))

        self.reset()
        took = self.dojo("--farewell", "suspend")
        self.dojo("--leave", "lock")
        self.expect(took < 1.0, "the suspend farewell is the short closing", f"{took:.2f}s")
        time.sleep(0.3)
        self.expect(self.recorded() == ["start-locker"],
                    "a lock asked for behind the sleeping closing starts the locker once", str(self.recorded()))
        self.dojo("--leave", "lock")
        time.sleep(0.2)
        self.expect(self.recorded() == ["start-locker"], "and not twice", str(self.recorded()))

        self.reset()
        self.dojo("--leave", "suspend-then-hibernate")
        time.sleep(0.6)
        self.expect(self.recorded() == ["suspend-then-hibernate"],
                    "suspend-then-hibernate runs its own action after the short closing", str(self.recorded()))

        self.reset()
        self.dojo("--leave", "logout")
        self.dojo("--farewell", "restart")
        time.sleep(1.5)
        self.expect(self.recorded() == ["logout"], "a farewell never adds an action to another closing",
                    str(self.recorded()))

        self.rig.shell.terminate()
        self.rig.shell.wait(timeout=10)
        absent = self.dojo("--farewell", "shutdown")
        self.expect(absent < 0.5, "with no shell a farewell returns at once", f"{absent:.2f}s")
        return not self.failures


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    parser.add_argument("--shell", default=str(ROOT / "target/debug/dojo"))
    parser.add_argument("--fake-logind", default=str(ROOT / "target/debug/examples/fake_logind"))
    parser.add_argument("--scale", default="1")
    parser.add_argument("--settle", type=float, default=4.0)
    parser.add_argument("--vk-driver-files", default=None)
    args = parser.parse_args()
    args.lock_wallpaper = None
    assert os.environ.get("WAYLAND_DISPLAY") and os.environ.get("XDG_RUNTIME_DIR"), \
        "needs a host Wayland session to nest in"
    rig = handoff.Rig()
    try:
        rig.start(args)
        ok = Smoke(rig).run()
    finally:
        rig.close()
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
