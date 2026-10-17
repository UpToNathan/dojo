#!/usr/bin/env python3
"""Exercise dojo-lock against the nested Niri that run-nested.sh started.

Never run this against the live session: it refuses unless NIRI_SOCKET and
WAYLAND_DISPLAY name a compositor that is itself a client of another display.
The locker is a `fake-pam-stack` build run with stand-in PAM stacks: the
password stack accepts one fixed test word, and the fingerprint stack succeeds
only when a trigger file appears.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import struct
import subprocess
import time
import zlib

from run import Peer

ROOT = Path(__file__).resolve().parents[2]
TEST_WORD = "kumite"
WRONG_WORD = "nope"
LOCK_DEADLINE_MS = 1000
FINGER_HINT = "Touch the stand-in sensor"

CHECK_PASSWORD = """#!/bin/bash
# Compares stdin with the fixed test word, never a real password.
IFS= read -r -d '' pw
[ "$pw" = "%s" ]
""" % TEST_WORD

FINGER = """#!/bin/bash
# Waits like a sensor until the trigger file appears, then succeeds once.
# Gives up once its parent is no longer a dojo-lock-pam helper, as a sensor claim does, so an
# orphan never eats the trigger. The helper may die before this starts, so $PPID is not enough.
helper_alive() {
    local parent args
    read -r _ _ _ parent _ < /proc/$$/stat
    mapfile -d '' args < "/proc/$parent/cmdline" 2>/dev/null || return 1
    [[ " ${args[*]} " == *dojo-lock-pam* ]]
}
while [ ! -e "$1/finger.trigger" ]; do helper_alive || exit 1; sleep 0.05; done
rm -f "$1/finger.trigger"
"""


def wait_for(check, description, timeout=10.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.01)
    raise AssertionError(f"Timed out waiting for {description}")


def write_png(path, width, height, pixel):
    rows = b"".join(b"\x00" + b"".join(bytes(pixel(x, y)) for x in range(width)) for y in range(height))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(rows, 6)) + chunk(b"IEND", b""))


def stand_in_wallpaper(path):
    """A smooth two-colour field, standing in for the pre-blurred theme wallpaper."""
    a, b = (0x2a, 0x3a, 0x6a), (0x6a, 0x3a, 0x5a)
    def pixel(x, y):
        t = (x / 319 + y / 179) / 2
        return [round(a[i] * (1 - t) + b[i] * t) for i in range(3)]
    write_png(path, 320, 180, pixel)


class Nested:
    """A nested compositor, proven not to be the live session."""

    def __init__(self, socket_path, display, log_path):
        match = re.search(r"niri\.(wayland-\d+)\.(\d+)\.sock$", socket_path)
        assert match, f"unexpected NIRI_SOCKET {socket_path}"
        assert match.group(1) == display, "NIRI_SOCKET and WAYLAND_DISPLAY name different compositors"
        self.pid = int(match.group(2))
        self.socket = socket_path
        self.display = display
        self.runtime = Path(os.environ["XDG_RUNTIME_DIR"])
        self.display_path = self.runtime / display
        environ = dict(item.split("=", 1) for item in
                       Path(f"/proc/{self.pid}/environ").read_text().split("\0") if "=" in item)
        host = environ.get("WAYLAND_DISPLAY")
        assert host and host != display, "this Niri is not nested in another Wayland display"
        self.host_display = host
        assert self.display_path.is_socket()
        self.log_path = log_path
        self.client_env = {k: v for k, v in os.environ.items()
                           if k not in ("WAYLAND_SOCKET", "DISPLAY", "WAYLAND_DEBUG", "NIRI_SOCKET")}
        self.client_env.update({"WAYLAND_DISPLAY": display, "NIRI_SOCKET": socket_path})

    def count(self, text):
        return Path(self.log_path).read_text(errors="replace").count(text)


def start_nested(evidence, host_display, name):
    """A second nested Niri without agent input, so virtual keyboards may type into the lock."""
    log_path = evidence / f"{name}.log"
    env = {k: v for k, v in os.environ.items() if k not in ("NIRI_SOCKET", "WAYLAND_SOCKET")}
    env.update({"WAYLAND_DISPLAY": host_display, "DOJO_AGENT_INPUT": "0", "RUST_LOG": "niri=info"})
    binary = os.environ.get("NIRI_BINARY", str(ROOT / "vendor/niri/target/debug/niri"))
    process = subprocess.Popen([binary, "--config", str(ROOT / "tools/agent-input/nested.kdl")],
                               env=env, stdout=log_path.open("w"), stderr=subprocess.STDOUT)
    def ready():
        text = log_path.read_text(errors="replace")
        display = re.findall(r"listening on Wayland socket: (\S+)", text)
        socket_path = re.findall(r"IPC listening on: (\S+)", text)
        if process.poll() is not None:
            raise AssertionError(f"{name} exited:\n{text}")
        return (display[-1], socket_path[-1]) if display and socket_path and Path(socket_path[-1]).is_socket() else None
    display, socket_path = wait_for(ready, name)
    return process, Nested(socket_path, display, log_path)


def clients_of(display_path):
    """Peer inodes of connections accepted on a Wayland socket.

    The locker sets PR_SET_DUMPABLE 0, which hides its sockets from `ss -p`, so
    a connection is attributed by counting the compositor's side instead.
    """
    output = subprocess.check_output(["ss", "-xn"], text=True)
    return {fields[7] for fields in (line.split() for line in output.splitlines())
            if len(fields) >= 8 and fields[0] == "u_str" and fields[4] == str(display_path)}


class Locker:
    def __init__(self, run, nested, name, extra_env=None):
        self.run = run
        self.nested = nested
        self.name = name
        self.log_path = run.evidence / f"{name}.log"
        env = dict(run.locker_env)
        env.update({"WAYLAND_DISPLAY": str(nested.display_path), "NIRI_SOCKET": nested.socket})
        env.update(extra_env or {})
        assert env["WAYLAND_DISPLAY"] != nested.host_display and Path(env["WAYLAND_DISPLAY"]).is_socket()
        self.clients_before = clients_of(nested.display_path)
        self.locks_before = nested.count("locking session")
        self.started_wall = time.time()
        self.process = subprocess.Popen([str(run.binary), "--pam-confdir", str(run.confdir)],
                                        env=env, stdout=subprocess.DEVNULL,
                                        stderr=self.log_path.open("w"))

    def text(self):
        return self.log_path.read_text(errors="replace")

    def ms(self, event):
        match = re.search(rf"dojo-lock: {event}.* ms=(\d+)", self.text())
        return int(match.group(1)) if match else None

    def wait_event(self, event, timeout=10.0):
        return wait_for(lambda: self.ms(event) is not None or self.dead(event), f"{self.name} {event}", timeout)

    def dead(self, waiting_for):
        if self.process.poll() is not None:
            raise AssertionError(f"{self.name} exited with {self.process.returncode} before {waiting_for}:\n{self.text()}")
        return False

    def wait_locked(self):
        self.wait_event("event=locked")
        assert clients_of(self.nested.display_path) - self.clients_before, \
            f"{self.name} locked without a new client on the nested compositor"
        assert self.nested.count("locking session") > self.locks_before, \
            "the nested Niri did not log this lock"
        return self.ms("event=locked")

    def reports(self, pattern):
        return [m for m in re.finditer(r"event=report (.*) ms=(\d+)", self.text()) if re.search(pattern, m.group(1))]

    def wait_report(self, pattern, count=1, timeout=10.0):
        """The `count`th report matching `pattern`, as a match whose group 2 is its ms."""
        def found():
            hits = self.reports(pattern)
            return hits[count - 1] if len(hits) >= count else self.dead(pattern)
        return wait_for(found, f"{self.name} report {pattern} #{count}", timeout)

    def exit_code(self, timeout=10.0):
        return self.process.wait(timeout=timeout)

    def descendants(self):
        children = []
        stack = [self.process.pid]
        while stack:
            pid = stack.pop()
            for task in Path(f"/proc/{pid}/task").glob("*"):
                try:
                    found = (task / "children").read_text().split()
                except OSError:
                    found = []
                children.extend(int(c) for c in found)
                stack.extend(int(c) for c in found)
        return children


class Run:
    def __init__(self, args):
        self.evidence = Path(args.log).resolve().parent
        self.shots = Path(args.screenshots).resolve()
        self.shots.mkdir(parents=True, exist_ok=True)
        self.agent_niri = Nested(args.socket, os.environ["WAYLAND_DISPLAY"], self.evidence / "niri.log")
        self.binary = Path(args.locker).resolve()
        self.results = {"agent_input_display": self.agent_niri.display,
                        "host_display": self.agent_niri.host_display, "agent_input_niri_pid": self.agent_niri.pid}
        self.wtype = shutil.which("wtype") or str(Path(subprocess.check_output(
            ["nix", "build", "--no-link", "--print-out-paths", "nixpkgs#wtype"], text=True).strip()) / "bin/wtype")
        self.grim = shutil.which("grim") or str(Path(subprocess.check_output(
            ["nix", "build", "--no-link", "--print-out-paths", "nixpkgs#grim"], text=True).strip()) / "bin/grim")
        self.prepare_stacks()
        self.prepare_home(args.wallpaper)
        self.agent = Peer(self.agent_niri.socket)
        self.agent.agent({"Create": {"name": "lock-check"}})
        self.typing_process, self.typing_niri = start_nested(
            self.evidence, self.agent_niri.host_display, "niri-typing")
        self.results["typing_display"] = self.typing_niri.display

    def prepare_stacks(self):
        self.pam = self.evidence / "pam"
        self.confdir = self.pam / "pam.d"
        self.confdir.mkdir(parents=True, exist_ok=True)
        (self.pam / "tally").mkdir(exist_ok=True)
        for name, body in [("check_password.sh", CHECK_PASSWORD), ("finger.sh", FINGER)]:
            path = self.pam / name
            path.write_text(body)
            path.chmod(0o755)
        (self.pam / "finger.txt").write_text(FINGER_HINT)
        tally = f"dir={self.pam}/tally deny=3 unlock_time=120"
        (self.confdir / "dojo-lock-password").write_text(
            f"auth required pam_faillock.so preauth {tally}\n"
            f"auth [success=2 default=ignore] pam_exec.so quiet expose_authtok {self.pam}/check_password.sh\n"
            # pam_exec fails with PAM_SYSTEM_ERR; pam_unix, which this stands in for, rejects with PAM_AUTH_ERR.
            "auth [default=bad] pam_deny.so\n"
            f"auth [default=die] pam_faillock.so authfail {tally}\n"
            f"auth required pam_faillock.so authsucc {tally}\n")
        (self.confdir / "dojo-lock-fingerprint").write_text(
            f"auth optional pam_echo.so file={self.pam}/finger.txt\n"
            f"auth required pam_exec.so quiet {self.pam}/finger.sh {self.pam}\n")

    def prepare_home(self, wallpaper):
        """Private runtime and state directories, so the marker and greeting history stay in the evidence."""
        self.runtime = self.evidence / "runtime"
        self.runtime.mkdir(mode=0o700, exist_ok=True)
        state = self.evidence / "state"
        theme = state / "dojo/themes/current"
        theme.mkdir(parents=True, exist_ok=True)
        real = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local/state") / "dojo/themes/current"
        for name in ("dojo.json", "palette.json", "lock-wallpaper.png"):
            if (real / name).exists():
                shutil.copy(real / name, theme / name)
        if wallpaper:
            shutil.copy(wallpaper, theme / "lock-wallpaper.png")
            self.results["wallpaper"] = str(wallpaper)
        elif not (theme / "lock-wallpaper.png").exists():
            stand_in_wallpaper(theme / "lock-wallpaper.png")
            self.results["wallpaper"] = "stand-in gradient (no theme lock-wallpaper.png yet)"
        else:
            self.results["wallpaper"] = "theme lock-wallpaper.png"
        env = {k: v for k, v in os.environ.items()
               if k not in ("WAYLAND_SOCKET", "DISPLAY", "WAYLAND_DEBUG", "RUST_LOG")}
        env.update({"XDG_RUNTIME_DIR": str(self.runtime), "XDG_STATE_HOME": str(state)})
        self.locker_env = env
        self.marker = self.runtime / "dojo/locked-at"

    def type(self, text):
        subprocess.run([self.wtype, "-d", "15", text], env=self.typing_niri.client_env, check=True)

    def press(self, key):
        subprocess.run([self.wtype, "-k", key], env=self.typing_niri.client_env, check=True)

    def shoot(self, name, nested=None):
        path = self.shots / f"{name}.png"
        subprocess.run([self.grim, str(path)], env=(nested or self.typing_niri).client_env, check=True)
        return str(path)

    def touch_sensor(self):
        (self.pam / "finger.trigger").write_text("")

    def assert_agent_rejected(self, label):
        try:
            self.agent.agent({"Key": {"evdev": 32, "state": "Pressed"}})
        except RuntimeError as error:
            return str(error)
        raise AssertionError(f"Locked compositor accepted an agent key ({label})")

    def assert_secret_absent(self, locker, label):
        """The test word is in no readable argv or environment of the locker or its descendants.

        Both the locker and the helper set PR_SET_DUMPABLE 0, which makes their
        environ unreadable even to this user; those are recorded, not skipped silently.
        """
        checked = {"readable": 0, "unreadable": []}
        for pid in [locker.process.pid, *locker.descendants()]:
            for part in ("cmdline", "environ"):
                try:
                    data = Path(f"/proc/{pid}/{part}").read_bytes()
                except OSError:
                    checked["unreadable"].append(f"{Path(f'/proc/{pid}/comm').read_text().strip()}/{part}")
                    continue
                checked["readable"] += 1
                assert TEST_WORD.encode() not in data, f"test word in /proc/{pid}/{part} ({label})"
        return checked


def scenario_password(run):
    """Resting, typing, wrong, then the right word unlocks with the greeting and opening."""
    niri = run.typing_niri
    unlocks = niri.count("unlocking session")
    locker = Locker(run, niri, "password")
    locked_ms = locker.wait_locked()
    locker.wait_report("Password.*Prompt")
    time.sleep(1.0)
    shots = {"resting": run.shoot("resting")}
    run.type(WRONG_WORD[:3])
    time.sleep(0.35)
    shots["typing"] = run.shoot("typing")
    proc_typing = run.assert_secret_absent(locker, "typing")
    run.type(WRONG_WORD[3:])
    run.press("Return")
    locker.wait_report(r"Password.*Finished\(Rejected\)")
    time.sleep(0.12)  # the report is logged before its frame is committed; the shake lasts 400 ms
    shots["wrong"] = run.shoot("wrong")
    time.sleep(0.6)
    shots["after-wrong"] = run.shoot("after-wrong")
    assert locker.process.poll() is None, "a wrong word ended the lock"
    proc_wrong = run.assert_secret_absent(locker, "after a wrong word")
    locker.wait_report("Password.*Prompt", count=2)
    run.type(TEST_WORD)
    run.press("Return")
    success = locker.wait_report(r"Password.*Finished\(Success\)")
    burst = []
    while locker.process.poll() is None:
        offset = round((time.time() - locker.started_wall) * 1000) - int(success.group(2))
        try:
            burst.append((offset, run.shoot(f"unlocking-{len(burst):02}")))
        except subprocess.CalledProcessError:
            break
    shots["unlocking"] = burst
    code = locker.exit_code()
    wait_for(lambda: niri.count("unlocking session") > unlocks, "niri unlocking")
    assert code == 0, f"exit {code}"
    assert not run.marker.exists(), "marker left after unlock"
    history = run.evidence / "state/dojo/session/unlock"
    return {"locked_ms": locked_ms, "exit": code, "screenshots": shots,
            "proc_checked_typing": proc_typing, "proc_checked_after_wrong": proc_wrong,
            "success_to_unlocked_ms": locker.ms("event=unlocked") - int(success.group(2)),
            "unlocking_burst_ms_after_success": [offset for offset, _ in burst],
            "greeting_stored": history.read_text().strip() if history.exists() else None}


def scenario_pause(run):
    """Three wrong words pause the password; the fingerprint still unlocks."""
    locker = Locker(run, run.typing_niri, "pause")
    locker.wait_locked()
    for attempt in range(1, 4):
        locker.wait_report("Password.*Prompt", count=attempt)
        run.type(WRONG_WORD)
        run.press("Return")
        locker.wait_report(r"Password.*Finished\(Rejected\)", count=attempt)
    locker.wait_report("PasswordPaused")
    time.sleep(0.6)
    shot = run.shoot("password-pause")
    run.touch_sensor()
    code = locker.exit_code()
    shutil.rmtree(run.pam / "tally")
    (run.pam / "tally").mkdir()
    assert code == 0, f"exit {code}"
    return {"screenshot": shot, "exit": code}


def scenario_fingerprint(run):
    """The stand-in sensor unlocks, and agent keys are rejected while locked."""
    niri = run.agent_niri
    unlocks = niri.count("unlocking session")
    locker = Locker(run, niri, "fingerprint")
    locker.wait_locked()
    rejected = run.assert_agent_rejected("fingerprint run")
    locker.wait_report("Fingerprint.*Hint")
    run.touch_sensor()
    success = locker.wait_report(r"Fingerprint.*Finished\(Success\)")
    code = locker.exit_code()
    wait_for(lambda: niri.count("unlocking session") > unlocks, "niri unlocking")
    assert code == 0, f"exit {code}"
    return {"exit": code, "agent_key_while_locked": rejected, "success_to_unlocked_ms": locker.ms("event=unlocked") - int(success.group(2))}


def scenario_opening(run, rounds):
    """Frames from inside the unlock opening, as fast as grim allows, for the clock texts across t.

    Frames are raw PPM to keep each capture short. Only those from the greeting's end onward
    are kept, named by their offset after the fingerprint success.
    """
    out = run.shots / "opening"
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir(parents=True)
    kept, capture_ms, exits = [], [], []
    for index in range(rounds):
        locker = Locker(run, run.typing_niri, f"opening-{index}")
        locker.wait_locked()
        locker.wait_report("Fingerprint.*Hint")
        time.sleep(0.5)
        run.touch_sensor()
        success = int(locker.wait_report(r"Fingerprint.*Finished\(Success\)").group(2))
        while locker.process.poll() is None:
            before = time.time()
            path = out / f"capture-{index}.ppm"
            try:
                subprocess.run([run.grim, "-t", "ppm", str(path)], env=run.typing_niri.client_env,
                               check=True, stderr=subprocess.DEVNULL)
            except subprocess.CalledProcessError:
                break
            after = time.time()
            capture_ms.append(round((after - before) * 1000))
            offset = round(((before + after) / 2 - locker.started_wall) * 1000) - success
            if offset >= 900:
                kept.append(offset)
                path.rename(out / f"{index}-{offset:05}.ppm")
        path.unlink(missing_ok=True)
        exits.append(locker.exit_code())
        kept.append(f"round {index} unlocked at {locker.ms('event=unlocked') - success}")
    assert exits == [0] * rounds, f"exit codes {exits}"
    return {"kept_offsets_ms": kept, "median_capture_ms": sorted(capture_ms)[len(capture_ms) // 2],
            "exits": exits}


def scenario_dead_key(run):
    """A dead key composes into one character: the pill matches one plain character, not two."""
    niri = run.typing_niri
    locker = Locker(run, niri, "dead-key")
    locker.wait_locked()
    locker.wait_report("Password.*Prompt")
    output = next(iter(Peer(niri.socket).request("Outputs")["Outputs"].values()))["logical"]
    band = f"0,{round(output['height'] * 0.59)} {output['width']}x{round(output['height'] * 0.075)}"
    def pill(name):
        time.sleep(0.6)
        path = run.shots / f"{name}.png"
        subprocess.run([run.grim, "-g", band, str(path)], env=niri.client_env, check=True)
        return path.read_bytes()
    subprocess.run([run.wtype, "-k", "dead_acute", "-k", "e"], env=niri.client_env, check=True)
    composed = pill("dead-key-composed")
    run.press("Escape")
    run.type("x")
    one = pill("dead-key-one-char")
    run.type("x")
    two = pill("dead-key-two-chars")
    run.touch_sensor()
    code = locker.exit_code()
    assert composed == one, "dead_acute + e did not show exactly one dot"
    assert one != two, "the band does not see the dots"
    return {"composed_matches_one_char": composed == one, "exit": code}


def scenario_takeover(run):
    """SIGKILL keeps the session locked and a second locker takes over the dead lock."""
    niri = run.agent_niri
    replaced = niri.count("replacing existing dead lock")
    first = Locker(run, niri, "takeover-first")
    first.wait_locked()
    marker = run.marker.read_text().split()
    first.process.send_signal(signal.SIGKILL)
    first.exit_code()
    time.sleep(0.3)
    still_locked = run.assert_agent_rejected("after SIGKILL")
    shot = run.shoot("dead-locker", niri)
    second = Locker(run, niri, "takeover-second")
    second.wait_locked()
    wait_for(lambda: niri.count("replacing existing dead lock") > replaced, "niri takeover")
    assert run.marker.read_text().split()[0] == marker[0], "takeover lost the first lock time"
    run.touch_sensor()
    code = second.exit_code()
    assert code == 0, f"exit {code}"
    return {"first_exit": first.process.returncode, "agent_key_while_dead": still_locked,
            "marker_kept": marker[0], "second_exit": code, "screenshot": shot}


def scenario_change_vt(run):
    """ChangeVt stays allowed while locked. The winit backend ignores it, so this reads the source."""
    source = (ROOT / "vendor/niri/src/input/mod.rs").read_text()
    body = re.search(r"fn allowed_when_locked\(action: &Action\) -> bool \{(.*?)\n\}", source, re.S)
    assert body and "Action::ChangeVt(_)" in body.group(1), "ChangeVt is not allowed when locked"
    return {"allowed_when_locked": re.findall(r"Action::(\w+)", body.group(1))}


# libwayland's WAYLAND_DEBUG lines: "[HH:MM:SS.micro] -> request" and "[...] {queue} event", in UTC.
STAMP = r"^\[(\d\d):(\d\d):(\d\d\.\d+)\] (?:\{[^}]*\} )?"


def seconds(match):
    return int(match.group(1)) * 3600 + int(match.group(2)) * 60 + float(match.group(3))


def first(text, line):
    match = re.search(STAMP + line, text, re.M)
    return seconds(match) if match else None


def commits(text, surface=r"wl_surface#\d+"):
    return [seconds(m) for m in re.finditer(STAMP + rf" -> {surface}\.commit\(\)", text, re.M)]


def day_seconds(wall):
    return wall % 86400


def cpu_and_switches(pid):
    """CPU ticks and context switches over every thread, plus switches per thread name."""
    ticks, by_thread = 0, {}
    for task in Path(f"/proc/{pid}/task").glob("*"):
        stat = (task / "stat").read_text()
        name = stat[stat.index("(") + 1:stat.rindex(")")]
        fields = stat.rsplit(")", 1)[1].split()
        ticks += int(fields[11]) + int(fields[12])
        switches = sum(int(v) for v in re.findall(r"ctxt_switches:\s+(\d+)", (task / "status").read_text()))
        by_thread[name] = by_thread.get(name, 0) + switches
    return ticks, by_thread


def switch_delta(before, after):
    delta = {name: after.get(name, 0) - before.get(name, 0) for name in after}
    return {name: count for name, count in sorted(delta.items(), key=lambda kv: -kv[1]) if count}


def scenario_idle(run, align):
    """With the display static the locker commits only the minute tick, and nothing once it is dark."""
    while not align - 2 <= time.time() % 60 <= align + 2:
        time.sleep(0.2)  # 40 puts a minute boundary inside the static window, 10 keeps it out
    niri = run.typing_niri
    locker = Locker(run, niri, "idle", {"WAYLAND_DEBUG": "client"})
    locker.wait_locked()
    locked = first(locker.text(), r"ext_session_lock_v1#\d+\.locked\(\)")
    settled = locker.started_wall + (locked - day_seconds(locker.started_wall)) + 5.0
    time.sleep(max(0, settled + 0.3 - time.time()))
    dim = run.shoot("settled-dim")
    start = time.time()
    t0, s0 = cpu_and_switches(locker.process.pid)
    time.sleep(29.0)
    t1, s1 = cpu_and_switches(locker.process.pid)
    end = time.time()
    ticks = os.sysconf("SC_CLK_TCK")
    inside = [c for c in commits(locker.text()) if day_seconds(start) <= c <= day_seconds(end)]
    time.sleep(max(0, settled + 30.0 + 2.0 - time.time()))
    dark_start = time.time()
    d0, ds0 = cpu_and_switches(locker.process.pid)
    time.sleep(15.0)
    d1, ds1 = cpu_and_switches(locker.process.pid)
    dark = [c for c in commits(locker.text()) if c >= day_seconds(dark_start)]
    timeline = [round(c - locked, 2) for c in commits(locker.text()) if c >= locked - 1]
    hints = len(locker.reports("Fingerprint.*Hint"))
    run.press("Shift_L")
    locker.wait_report("Fingerprint.*Hint", count=hints + 1)
    awake = run.shoot("woken-after-display-off")
    run.touch_sensor()
    code = locker.exit_code()
    assert code == 0, f"exit {code}"
    assert len(dark) == 0, f"{len(dark)} commits while the display was off"
    return {"static_window_s": round(end - start, 2), "commits_static": len(inside),
            "commits_static_at_second": [round(c % 60, 2) for c in inside],
            "cpu_s_static": (t1 - t0) / ticks, "context_switches_static": switch_delta(s0, s1),
            "dark_window_s": 15.0, "commits_dark": len(dark), "cpu_s_dark": (d1 - d0) / ticks,
            "context_switches_dark": switch_delta(ds0, ds1), "commit_times_after_locked_s": timeline,
            "dim_screenshot": dim, "woken_screenshot": awake, "exit": code}


def scenario_startup(run, label, extra_env, rounds):
    """Per round: one start traced with WAYLAND_DEBUG for wire timings, one untraced for the locker's own."""
    samples = []
    for index in range(rounds):
        traced = Locker(run, run.agent_niri, f"startup-{label}-{index}-traced", {"WAYLAND_DEBUG": "client", **extra_env})
        traced.wait_locked()
        text = traced.text()
        started = day_seconds(traced.started_wall)
        lock_request = first(text, r" -> ext_session_lock_manager_v1#\d+\.lock\(")
        surface = re.search(r"get_lock_surface\(new id \S+, (wl_surface#\d+)", text).group(1)
        commit = commits(text, surface)[0]
        locked = first(text, r"ext_session_lock_v1#\d+\.locked\(\)")
        ms = lambda a, b: round((b - a) * 1000, 1)
        run.touch_sensor()
        assert traced.exit_code() == 0
        time.sleep(0.3)
        plain = Locker(run, run.agent_niri, f"startup-{label}-{index}", extra_env)
        plain.wait_locked()
        run.touch_sensor()
        assert plain.exit_code() == 0
        time.sleep(0.3)
        samples.append({
            "traced_spawn_to_first_commit_ms": ms(started, commit),
            "traced_spawn_to_locked_ms": ms(started, locked),
            "traced_lock_request_to_locked_ms": ms(lock_request, locked),
            "traced_commit_before_locked": commit <= locked,
            "main_to_first_render_ms": plain.ms("event=first-render"),
            "main_to_locked_ms": plain.ms("event=locked"),
        })
    return samples


def scenario_lost_connection(run):
    """Killing the compositor ends the event loop without an unlock: exit 75."""
    locker = Locker(run, run.agent_niri, "lost-connection")
    locker.wait_locked()
    os.kill(run.agent_niri.pid, signal.SIGKILL)
    code = locker.exit_code(timeout=15)
    assert code == 75, f"exit {code}, expected 75"
    return {"exit": code}


def stand_ins(directory):
    """Live processes running this run's stand-in PAM scripts."""
    found = []
    for proc in Path("/proc").iterdir():
        try:
            if proc.name.isdigit() and str(directory).encode() in (proc / "cmdline").read_bytes():
                found.append(int(proc.name))
        except OSError:
            pass
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", default=os.environ.get("NIRI_SOCKET"))
    parser.add_argument("--log", required=True)
    parser.add_argument("--locker", default=str(ROOT / "target/debug/dojo-lock"))
    parser.add_argument("--screenshots", default=str(ROOT / ".audit/e1"))
    parser.add_argument("--startup-rounds", type=int, default=5)
    parser.add_argument("--vk-driver-files", default=next(
        (str(p) for p in (Path("/run/opengl-driver/share/vulkan/icd.d/radeon_icd.x86_64.json"),
                          Path("/usr/share/vulkan/icd.d/radeon_icd.x86_64.json")) if p.exists()), None),
        help="VK_DRIVER_FILES for the pinned startup rounds")
    parser.add_argument("--idle-align", type=int, default=40,
                        help="Second of the minute to start the idle locker at")
    parser.add_argument("--wallpaper", help="A rendered lock-wallpaper.png to use instead of the theme's")
    parser.add_argument("--opening-rounds", type=int, default=3,
                        help="Unlocks to sample in the opening scenario, which runs only with --only")
    parser.add_argument("--only", nargs="*", help="Run only these scenarios")
    args = parser.parse_args()
    run = Run(args)
    scenarios = [
        ("change_vt", lambda: scenario_change_vt(run)),
        ("password", lambda: scenario_password(run)),
        ("fingerprint", lambda: scenario_fingerprint(run)),
        ("pause", lambda: scenario_pause(run)),
        ("dead_key", lambda: scenario_dead_key(run)),
        ("takeover", lambda: scenario_takeover(run)),
        ("startup_unpinned", lambda: scenario_startup(run, "unpinned", {}, args.startup_rounds)),
        ("startup_pinned", lambda: scenario_startup(
            run, "pinned", {"VK_DRIVER_FILES": args.vk_driver_files}, args.startup_rounds)
            if args.vk_driver_files else "skipped: no --vk-driver-files"),
        ("idle", lambda: scenario_idle(run, args.idle_align)),
        ("lost_connection", lambda: scenario_lost_connection(run)),
        ("opening", lambda: scenario_opening(run, args.opening_rounds)),
    ]
    try:
        for name, scenario in scenarios:
            if args.only and name not in args.only or not args.only and name == "opening":
                continue
            run.results[name] = scenario()
            print(json.dumps({name: run.results[name]}), flush=True)
    finally:
        run.typing_process.terminate()
        run.typing_process.wait(timeout=10)
    time.sleep(0.5)
    survivors = stand_ins(run.pam)
    for pid in survivors:
        os.kill(pid, signal.SIGKILL)
    assert not survivors, f"stand-in PAM processes outlived their lockers: {survivors}"
    secret_hits = [str(p) for p in run.evidence.rglob("*.log") if TEST_WORD in p.read_text(errors="replace")]
    assert not secret_hits, f"test word in logs: {secret_hits}"
    run.results["test_word_in_logs"] = secret_hits
    (run.evidence / "result.json").write_text(json.dumps(run.results, indent=2))
    print(json.dumps({"result": str(run.evidence / "result.json")}))


if __name__ == "__main__":
    main()
