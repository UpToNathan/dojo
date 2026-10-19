#!/usr/bin/env python3
"""Exercise the real dojo-dm and the real dojo-greeter against each other, as a normal user.

Each scenario starts its own nested Niri (a client of the live display, which it never locks),
a `fake-pam-stack` build of `dojo-dm` with its test options, and the `fake-pam-stack` build of
`dojo-greeter` as the dm's greeter command. A recording proxy sits on the greeter socket and
writes every datagram, decoded, to transcript.jsonl, so message order is read off the wire.

What is real: both programs, the protocol, `dojo-dm-pam` and libpam against private PAM stacks
(a known test password, and a wrong one), look snapshots written and read through the state
directory, the session process and its environment, the greeter's teardown and restart.
What stays fake: the VT switch (`--vt N` only names the VT), logind (no greeter session, so the
seat is always free), the greeter's own Niri and logind session (the nested Niri stands in), the
identity drop (as the current user), and power (a log, never `systemctl`). Nothing touches /etc,
the live Niri or a real power action.

  NIRI_BINARY=... python3 tools/agent-input/run-login.py [--only NAME ...] [--keep] [--evidence DIR]

Build first: devenv shell -- cargo build --features fake-pam-stack --bin dojo-dm --bin dojo-dm-pam --bin dojo-greeter
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("run_greeter", ROOT / "tools/agent-input/run-greeter.py")
G = importlib.util.module_from_spec(spec)
spec.loader.exec_module(G)

ANY = G.ANY
SECRET = "fake-login-secret-0123456789"
WRONG = "not-the-password"
FINGER_HINT = "Place your finger on the reader (stand-in)"
VT = 3
SESSION_IDS = ["gnome-test", "niri-test"]
REMEMBERED_SESSION = "niri-test"
SECOND_SESSION = "gnome-test"
METHODS = ("fingerprint", "password")
REASONS = ("refused", "spawn_failed", "unknown_session", "helper_crashed", "exited")

CHECK_PASSWORD = f"""#!/bin/bash
IFS= read -r -d '' pw
[ "$pw" = "{SECRET}" ]
"""

HOOK = """#!/bin/bash
case "$PAM_TYPE" in
  open_session) echo "open_session desktop=$XDG_SESSION_DESKTOP vtnr=$XDG_VTNR tty=$PAM_TTY" ;;
  *) echo "$PAM_TYPE" ;;
esac >> "$DIR/pam-phases.log"
[ "$PAM_TYPE" = open_session ] && [ -e "$DIR/refuse-session" ] && exit 1
exit 0
"""

SESSION = """#!/bin/bash
{
  echo "pid=$$"
  echo "cwd=$(pwd -P)"
  echo "started_at=$(date +%s.%N)"
  for p in $(cat "$DIR/greeter.pids" 2>/dev/null); do kill -0 "$p" 2>/dev/null && echo "greeter_alive=$p"; done
  for d in /proc/[0-9]*; do tr '\0' ' ' < "$d/cmdline" 2>/dev/null | grep -q "$DIR/release/" && echo "greeter_alive=${d#/proc/}"; done
  env
} > "$DIR/session.out.tmp"
mv "$DIR/session.out.tmp" "$DIR/session.out"
for _ in $(seq 1 1200); do [ -e "$DIR/session.release" ] && break; sleep 0.1; done
rm -f "$DIR/session.release"
"""

SESSION_SHELL = """#!/bin/sh
exec /bin/sh -c "$3"
"""

# The dm's greeter command. It outlives the dm when the dm is killed (so the greeter's own reaction
# to a lost dm can be seen), records how each greeter ended, and relays the dm's stop signal.
WRAPPER = """import json, os, signal, subprocess, sys, ctypes
config = json.load(open(sys.argv[1]))
ctypes.CDLL(None).prctl(1, 0, 0, 0, 0)
directory = config["dir"]
n = len([f for f in os.listdir(directory) if f.startswith("greeter-") and f.endswith(".log")]) + 1
log = open(f"{directory}/greeter-{n}.log", "w")
child = subprocess.Popen(config["command"], env=config["env"],
                         stdin=subprocess.DEVNULL, stdout=log, stderr=log)
open(f"{directory}/greeter.pids", "w").write(f"{os.getpid()}\\n{child.pid}\\n")
open(f"{directory}/greeter-{n}.pid", "w").write(str(child.pid))
signal.signal(signal.SIGTERM, lambda *_: child.terminate())
code = child.wait()
open(f"{directory}/greeter-{n}.exit", "w").write(str(code))
sys.exit(0 if code == 0 else 1)
"""


def fail(message):
    raise AssertionError(message)


def check(condition, message):
    if not condition:
        fail(message)


class Reader(G.Reader):
    def flag(self):
        return self.u8() != 0

    def i32(self):
        return struct.unpack("<i", self.take(4))[0]


def decode_report(reader):
    kind = reader.u8()
    if kind == 0:
        return {"prompt": {"echo": reader.flag()}}
    if kind == 1:
        error = reader.flag()
        return {"hint": reader.text(), "error": error}
    if kind == 2:
        return {"password_locked": True}
    if kind == 3:
        return {"password_paused": reader.u32()}
    if kind == 4:
        outcome, code = reader.u8(), reader.i32()
        return {"finished": ("success", "rejected", "unavailable", "max_tries", f"other({code})")[outcome]}
    if kind == 5:
        return {"crashed": True}
    fail(f"unknown report {kind}")


def decode_event(datagram):
    tag, reader = chr(datagram[0]), Reader(datagram[1:])
    if tag == "i":
        last_login = reader.text() if reader.flag() else None
        last_session = reader.text() if reader.flag() else None
        sessions = [{"id": reader.text(), "name": reader.text()} for _ in range(reader.u8())]
        return {"init": True, "last_login": last_login, "last_session": last_session, "sessions": sessions}
    if tag == "u":
        login = reader.text()
        display = reader.text() if reader.flag() else None
        return {"user": login, "display_name": display, "fingerprint": reader.flag()}
    if tag == "r":
        generation, method = reader.u32(), METHODS[reader.u8()]
        return {"report": generation, "method": method, **decode_report(reader)}
    if tag == "s":
        return {"session_started": True}
    if tag == "f":
        return {"session_failed": REASONS[reader.u8()]}
    fail(f"unknown event {datagram!r}")


def decode_request(datagram):
    request = G.decode(datagram)
    if "secret" in request:
        secret = request.pop("secret")
        request["secret"] = "right" if secret == SECRET else "wrong" if secret == WRONG else f"other({len(secret)})"
    return request


def matches(message, fields):
    return all((key in message) if value is ANY else message.get(key) == value for key, value in fields.items())


class Proxy:
    """Sits between the greeter and the dm, passing every datagram on and writing it down."""

    def __init__(self, listen_path, dm_path, transcript_path):
        self.dm_path = dm_path
        self.entries = []
        self.lock = threading.Lock()
        self.file = transcript_path.open("w", buffering=1)
        self.t0 = time.monotonic()
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        self.server.bind(str(listen_path))
        self.server.listen(4)
        self.connections = 0
        threading.Thread(target=self.accept, daemon=True).start()

    def note(self, direction, message):
        with self.lock:
            entry = {"n": len(self.entries), "ms": round((time.monotonic() - self.t0) * 1000), "dir": direction,
                     "conn": self.connections, **message}
            self.entries.append(entry)
            self.file.write(json.dumps(entry) + "\n")

    def accept(self):
        while True:
            try:
                greeter, _ = self.server.accept()
            except OSError:
                return
            self.connections += 1
            upstream = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
            try:
                upstream.connect(str(self.dm_path))
            except OSError as error:
                self.note("proxy", {"upstream_refused": str(error)})
                greeter.close()
                continue
            for source, sink, direction, decode in ((greeter, upstream, "g>d", decode_request),
                                                    (upstream, greeter, "d>g", decode_event)):
                threading.Thread(target=self.pump, args=(source, sink, direction, decode), daemon=True).start()

    def pump(self, source, sink, direction, decode):
        while True:
            try:
                datagram = source.recv(16 * 1024)
            except OSError:
                datagram = b""
            if not datagram:
                self.note(direction, {"closed": True})
                for end in (source, sink):
                    try:
                        end.shutdown(socket.SHUT_RDWR)
                    except OSError:
                        pass
                return
            self.note(direction, decode(datagram))
            try:
                sink.send(datagram)
            except OSError:
                return

    def seen(self, direction, start=0, **fields):
        with self.lock:
            return [e for e in self.entries[start:] if e["dir"] == direction and matches(e, fields)]

    def wait(self, direction, count=1, start=0, timeout=20.0, **fields):
        def found():
            seen = self.seen(direction, start, **fields)
            return seen[count - 1] if len(seen) >= count else None

        return G.wait_for(found, f"{direction} {fields} #{count}", timeout)

    def mark(self):
        with self.lock:
            return len(self.entries)


def mean_colour(path):
    """Average RGB of a binary PPM."""
    data = path.read_bytes()
    header = data.split(b"\n", 3)
    width, height = map(int, header[1].split())
    body = header[3]
    totals = [0, 0, 0]
    count = 0
    for i in range(0, width * height * 3, 3 * 7):
        for c in range(3):
            totals[c] += body[i + c]
        count += 1
    return tuple(round(t / count) for t in totals)


def apart(a, b):
    return sum(abs(x - y) for x, y in zip(a, b))


def process_children(pid):
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text()
            ppid = int(stat.rsplit(")", 1)[1].split()[1])
            if ppid == pid:
                found.append((int(entry.name), (entry / "cmdline").read_bytes().replace(b"\0", b" ").decode()))
        except (OSError, ValueError, IndexError):
            pass
    return found


def alive(pid):
    try:
        os.kill(pid, 0)
        stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
        return stat != "Z"
    except (OSError, ValueError):
        return False


class DmNiri:
    """The Niri that the dm itself starts (the installed greeter.kdl), found through its log."""

    def __init__(self, lab):
        self.lab = lab

    @property
    def env(self):
        logs = sorted(self.lab.dir.glob("greeter-*.log"), key=lambda p: int(p.stem.split("-")[1]))
        text = logs[-1].read_text(errors="replace")
        display = re.findall(r"listening on Wayland socket: (\S+)", text)
        socket_path = re.findall(r"IPC listening on: (\S+)", text)
        check(display and socket_path, f"the dm's Niri has not announced its sockets:\n{text}")
        env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_SOCKET", "DISPLAY", "WAYLAND_DEBUG")}
        env.update({"WAYLAND_DISPLAY": display[-1], "NIRI_SOCKET": socket_path[-1]})
        return env

    def stop(self):
        pass


class Lab:
    """One nested Niri, one proxy, one dojo-dm, as many greeters as the dm starts."""

    def __init__(self, run, name, memory=("nathan", REMEMBERED_SESSION), fingerprints=(), session_shell=None,
                 dm_starts_niri=False, greeter_socket=None):
        self.run, self.name = run, name
        # A client of the dm's own Niri has no stderr of its own; Niri says when it takes the lock.
        self.locked_marker = "locking session" if dm_starts_niri else "event=locked"
        self.dir = run.evidence / name
        self.dir.mkdir()
        self.state = self.dir / "state"
        G.prepare_snapshots(self.state)
        self.state.chmod(0o755)
        if memory:
            for file, value in zip(("last-user", "last-session"), memory):
                (self.state / file).write_text(value + "\n")
        for sub in ("pam.d", "sessions"):
            (self.dir / sub).mkdir()
        (self.dir / "sessions").chmod(0o755)
        root = str(self.dir)
        finger = (f"auth optional pam_echo.so file={root}/finger.txt\nauth required pam_permit.so\n"
                  f"account required pam_exec.so quiet {root}/hook.sh\nsession required pam_exec.so quiet {root}/hook.sh\n")
        password = (f"auth [success=1 default=ignore] pam_exec.so quiet expose_authtok {root}/check_password.sh\n"
                    f"auth requisite pam_deny.so\nauth required pam_permit.so\n"
                    f"account required pam_exec.so quiet {root}/hook.sh\nsession required pam_exec.so quiet {root}/hook.sh\n")
        (self.dir / "pam.d/dojo-login").write_text(password)
        (self.dir / "pam.d/dojo-login-fingerprint").write_text(finger)
        (self.dir / "finger.txt").write_text(FINGER_HINT)
        for file, body in (("check_password.sh", CHECK_PASSWORD), ("hook.sh", HOOK), ("session.sh", SESSION),
                           ("shell.sh", SESSION_SHELL)):
            path = self.dir / file
            path.write_text(body.replace("$DIR", root) if file != "check_password.sh" else body)
            path.chmod(0o755)
        for ident, title in (("gnome-test", "GNOME"), ("niri-test", "Niri (UWSM)")):
            entry = self.dir / "sessions" / f"{ident}.desktop"
            entry.write_text(f"[Desktop Entry]\nName={title}\nExec={root}/session.sh %U\n"
                             f"DesktopNames=niri;GNOME;\nType=Application\n")
            entry.chmod(0o644)

        self.proxy = Proxy(self.dir / "proxy.sock", self.dir / "dm.sock", self.dir / "transcript.jsonl")
        sockets = {"DOJO_DM_SOCKET": greeter_socket or str(self.dir / "proxy.sock"), "DOJO_DM_STATE": str(self.state)}
        if dm_starts_niri:
            # What the installer puts in a release: the greeter's Niri config and an environment.sh.
            release = self.dir / "release"
            release.mkdir()
            (release / "environment.sh").write_text("".join(f"export {k}='{v}'\n" for k, v in sockets.items()))
            (release / "dojo-greeter").symlink_to(run.greeter)
            (release / "niri").symlink_to(run.niri)
            config = (ROOT / "system-config/dojo-dm/greeter.kdl").read_text().replace("@RELEASE@", str(release))
            (release / "greeter.kdl").write_text(config)
            host = {k: v for k, v in os.environ.items() if k not in ("NIRI_SOCKET", "WAYLAND_SOCKET")}
            host.update({"DOJO_AGENT_INPUT": "0", "RUST_LOG": "niri=info"})
            command = [str(run.niri), "--config", str(release / "greeter.kdl")]
            self.nested, env = DmNiri(self), host
        else:
            self.nested = G.Nested(self.dir)
            command, env = [str(run.greeter)], {**self.nested.env, **sockets}
        (self.dir / "wrapper.json").write_text(json.dumps({"dir": root, "command": command, "env": env}))
        (self.dir / "wrapper.py").write_text(WRAPPER)
        self.power_log = self.dir / "power.log"
        arguments = [str(run.dm), "--socket", str(self.dir / "dm.sock"), "--state", str(self.state),
                     "--sessions", str(self.dir / "sessions"), "--helper", str(run.helper),
                     "--pam-confdir", str(self.dir / "pam.d"), "--session-shell",
                     session_shell or str(self.dir / "shell.sh"), "--vt", str(VT),
                     "--power-log", str(self.power_log)]
        for login in fingerprints:
            arguments += ["--fingerprint", login]
        arguments += ["--greeter-command", sys.executable, str(self.dir / "wrapper.py"), str(self.dir / "wrapper.json")]
        self.dm_log = self.dir / "dm.log"
        self.dm = subprocess.Popen(arguments, env={"PATH": "/usr/local/bin:/usr/bin"}, stdin=subprocess.DEVNULL,
                                   stdout=subprocess.DEVNULL, stderr=self.dm_log.open("w"))

    def dm_text(self):
        return self.dm_log.read_text(errors="replace")

    def greeter_log(self, n):
        path = self.dir / f"greeter-{n}.log"
        return path.read_text(errors="replace") if path.exists() else ""

    def greeter_pid(self, n):
        return G.wait_for(lambda: int((self.dir / f"greeter-{n}.pid").read_text())
                          if (self.dir / f"greeter-{n}.pid").exists() else None, f"greeter {n} to start")

    def wait_locked(self, n, timeout=30.0):
        def ready():
            if self.dm.poll() is not None:
                fail(f"dojo-dm exited {self.dm.returncode}:\n{self.dm_text()}")
            return self.locked_marker in self.greeter_log(n)
        G.wait_for(ready, f"greeter {n} to lock", timeout)

    def wait_greeter_log(self, n, needle, timeout=10.0):
        G.wait_for(lambda: needle in self.greeter_log(n), f"greeter {n} to log {needle!r}:\n{self.greeter_log(n)}", timeout)

    def wait_greeter_exit(self, n, timeout=20.0):
        path = self.dir / f"greeter-{n}.exit"
        return int(G.wait_for(lambda: path.read_text() if path.exists() else None,
                              f"greeter {n} to end:\n{self.greeter_log(n)}", timeout))

    def type(self, value):
        subprocess.run([self.run.wtype, "-d", "15", value], env=self.nested.env, check=True)

    def press(self, *keys):
        *modifiers, key = keys
        command = [self.run.wtype]
        for modifier in modifiers:
            command += ["-M", modifier]
        command += ["-k", key]
        for modifier in reversed(modifiers):
            command += ["-m", modifier]
        subprocess.run(command, env=self.nested.env, check=True)

    def shoot(self, name, full=False):
        path = self.dir / (f"{name}.png" if full else f"{name}.ppm")
        command = [self.run.grim] + ([] if full else ["-t", "ppm", "-s", "0.2"]) + [str(path)]
        subprocess.run(command, env=self.nested.env, check=True)
        return path if full else mean_colour(path)

    def session_out(self):
        path = self.dir / "session.out"
        G.wait_for(path.exists, "the session command to run", 30.0)
        values = {}
        alive_pids = []
        for line in path.read_text().splitlines():
            key, _, value = line.partition("=")
            if key == "greeter_alive":
                alive_pids.append(value)
            else:
                values[key] = value
        values["greeter_alive"] = alive_pids
        return values

    def release_session(self):
        (self.dir / "session.out").unlink(missing_ok=True)
        (self.dir / "session.release").write_text("")

    def stop(self):
        (self.dir / "session.release").write_text("")
        if self.dm.poll() is None:
            self.dm.send_signal(signal.SIGTERM)
            try:
                self.dm.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.dm.kill()
                self.dm.wait()
        self.nested.stop()


def log_in(lab, password=SECRET):
    lab.type(password)
    lab.press("Return")


def environment_of(lab, session_id, expected_login="nathan"):
    env = lab.session_out()
    home = os.path.expanduser(f"~{expected_login}")
    check(env["USER"] == expected_login, f"USER={env['USER']}")
    check(env["HOME"] == home, f"HOME={env['HOME']}, expected {home}")
    check(env["XDG_SESSION_DESKTOP"] == session_id, f"XDG_SESSION_DESKTOP={env['XDG_SESSION_DESKTOP']}")
    check(env["XDG_VTNR"] == str(VT), f"XDG_VTNR={env['XDG_VTNR']}")
    check(env["XDG_SESSION_TYPE"] == "wayland", env["XDG_SESSION_TYPE"])
    check(env["XDG_SESSION_CLASS"] == "user", env["XDG_SESSION_CLASS"])
    check(env["XDG_SEAT"] == "seat0", env["XDG_SEAT"])
    check(env["XDG_CURRENT_DESKTOP"] == "niri:GNOME", env["XDG_CURRENT_DESKTOP"])
    check(env["cwd"] == os.path.realpath(home), f"cwd={env['cwd']}")
    return env


def scenario_login(lab):
    """Init and prefill, typing, wrong then right, user and session chips, StartSession, teardown, return."""
    p = lab.proxy
    lab.wait_locked(1)
    init = p.wait("d>g", init=True)
    check(init["last_login"] == "nathan" and init["last_session"] == REMEMBERED_SESSION, f"init: {init}")
    check([s["id"] for s in init["sessions"]] == SESSION_IDS, f"sessions: {init['sessions']}")
    hello = p.wait("g>d", hello=ANY)
    first = p.wait("g>d", begin=ANY, method="password")
    check(hello["n"] < init["n"] < first["n"], "order: hello, init, then the first begin")
    check((first["login"], first["session"]) == ("nathan", REMEMBERED_SESSION), f"prefill: {first}")
    check(p.seen("g>d", select_user="nathan"), "the first user is asked about")
    time.sleep(1.0)
    nathan_look = lab.shoot("resting")
    lab.shoot("resting", full=True)

    prompt = p.wait("d>g", report=first["begin"], prompt=ANY)
    lab.type(WRONG)
    lab.press("Return")
    answer = p.wait("g>d", answer=first["begin"])
    check(answer["secret"] == "wrong", f"typing reached the password: {answer}")
    rejected = p.wait("d>g", report=first["begin"], finished="rejected")
    second = p.wait("g>d", begin=ANY, method="password", start=rejected["n"])
    check(second["begin"] != first["begin"], "a refused conversation is replaced by a new generation")
    p.wait("d>g", report=second["begin"], prompt=ANY)
    stale_cancel = p.seen("g>d", start=rejected["n"], cancel=first["begin"])

    mark = p.mark()
    lab.press("Tab")
    lab.type("alyssa")
    lab.press("Return")
    p.wait("g>d", start=mark, select_user="alyssa")
    user = p.wait("d>g", start=mark, user="alyssa")
    check(user["fingerprint"] is False, "no reader and no print for alyssa")
    cancelled = p.wait("g>d", start=mark, cancel=second["begin"])
    alyssa_begin = p.wait("g>d", start=mark, begin=ANY, login="alyssa")
    check(alyssa_begin["session"] == REMEMBERED_SESSION, "the session choice survives a user change")
    time.sleep(1.2)
    alyssa_look = lab.shoot("alyssa")
    check(apart(alyssa_look, nathan_look) > 12, f"alyssa's palette differs: {nathan_look} -> {alyssa_look}")
    alyssa_prompt = p.seen("d>g", start=mark, report=alyssa_begin["begin"], prompt=ANY)
    check(alyssa_prompt, "a name with no account is asked for a password like any other, then refused")
    lab.type(WRONG)
    lab.press("Return")
    p.wait("d>g", start=mark, report=alyssa_begin["begin"], finished="rejected")
    again = p.wait("g>d", start=mark, begin=ANY, login="alyssa", count=2)
    check(again["begin"] != alyssa_begin["begin"], "refused again with a fresh conversation")

    mark = p.mark()
    lab.press("Tab")
    lab.type("nathan")
    lab.press("Return")
    p.wait("g>d", start=mark, select_user="nathan")
    p.wait("d>g", start=mark, user="nathan")
    back = p.wait("g>d", start=mark, begin=ANY, login="nathan")
    time.sleep(1.2)
    restored = lab.shoot("nathan-again")
    check(apart(restored, nathan_look) < 12, f"nathan's look returns: {nathan_look} -> {restored}")
    p.wait("d>g", start=back["n"], report=back["begin"], prompt=ANY)

    mark = p.mark()
    lab.press("ctrl", "Tab")
    switched = p.wait("g>d", start=mark, begin=ANY, method="password", login="nathan")
    check(switched["session"] == SECOND_SESSION, f"the session chip cycles: {switched}")
    check(p.seen("g>d", start=mark, cancel=back["begin"]), "the conversation begun for the old session is cancelled")
    p.wait("d>g", start=switched["n"], report=switched["begin"], prompt=ANY)

    log_in(lab)
    answer = p.wait("g>d", start=switched["n"], answer=switched["begin"])
    check(answer["secret"] == "right", "the test password went to the live conversation")
    p.wait("d>g", report=switched["begin"], finished="success")
    start = p.wait("g>d", start_session=ANY)
    check(start["start_session"] == switched["begin"], f"StartSession names the winning conversation: {start}")
    check(start["n"] > p.seen("d>g", report=switched["begin"], finished="success")[0]["n"], "start after the verdict")
    env = environment_of(lab, SECOND_SESSION)
    check(env["greeter_alive"] == [], f"the greeter was torn down before the session started: {env['greeter_alive']}")
    closed = p.wait("d>g", start=start["n"], closed=True)
    check(not p.seen("d>g", start=start["n"], session_started=True),
          "the dm tears the greeter down before the helper starts the session, so SessionStarted never reaches it")
    exit_code = lab.wait_greeter_exit(1)
    check((lab.state / "last-user").read_text().strip() == "nathan"
          and (lab.state / "last-session").read_text().strip() == SECOND_SESSION, "last user and session remembered")
    lab.release_session()
    mark = p.mark()
    init2 = p.wait("d>g", start=mark, init=True)
    check((init2["last_login"], init2["last_session"]) == ("nathan", SECOND_SESSION), f"second init: {init2}")
    lab.wait_locked(2)
    p.wait("g>d", start=mark, begin=ANY, method="password", login="nathan", session=SECOND_SESSION)
    time.sleep(1.0)
    shot = lab.shoot("returned", full=True)
    check(lab.dm.poll() is None, "dojo-dm is still running")
    phases = (lab.dir / "pam-phases.log").read_text().splitlines()
    check(phases[-3:] == ["account", f"open_session desktop={SECOND_SESSION} vtnr={VT} tty=tty{VT}", "close_session"],
          f"PAM ran account, open_session and close_session once: {phases}")
    return {"first_greeter_exit": exit_code, "stale_cancel_after_rejection": bool(stale_cancel),
            "snapshot_modes": snapshot_modes(lab.state / "users/nathan"), "returned": str(shot),
            "unexpected_dm_log": ignored(lab)}


def snapshot_modes(directory):
    return {"dir": oct(directory.stat().st_mode & 0o7777),
            **{f.name: oct(f.stat().st_mode & 0o7777) for f in sorted(directory.iterdir())}}


def ignored(lab):
    return [line for line in lab.dm_text().splitlines() if "ignored a request" in line or "refused" in line
            or "undecodable" in line]


def scenario_fingerprint(lab):
    """A user with a print: the User reply says so, the sensor runs, a finger wins and starts the session."""
    p = lab.proxy
    lab.wait_locked(1)
    user = p.wait("d>g", user="nathan")
    check(user["fingerprint"] is True, f"the fingerprint flag reaches the greeter: {user}")
    finger = p.wait("g>d", begin=ANY, method="fingerprint", login="nathan")
    password = p.wait("g>d", begin=ANY, method="password", login="nathan")
    p.wait("d>g", report=finger["begin"], finished="success")
    start = p.wait("g>d", start_session=ANY)
    check(start["start_session"] == finger["begin"], f"the finger's conversation starts the session: {start}")
    check(p.seen("g>d", cancel=password["begin"]), "the password conversation is cancelled")
    environment_of(lab, REMEMBERED_SESSION)
    check(not p.seen("g>d", answer=ANY), "no password was typed or sent")
    return {"cancelled_password_gen": bool(p.seen("g>d", cancel=password["begin"])), "unexpected_dm_log": ignored(lab)}


def scenario_failed_session(lab):
    """A helper that dies after the verdict, then sessions that fail to start after the greeter is gone."""
    p = lab.proxy
    lab.wait_locked(1)
    first = p.wait("g>d", begin=ANY, method="password")
    p.wait("d>g", report=first["begin"], prompt=ANY)
    time.sleep(0.5)
    mark = p.mark()
    log_in(lab)
    G.wait_for(lambda: "account" in ((lab.dir / "pam-phases.log").read_text().split() if (lab.dir / "pam-phases.log").exists() else []),
               "the account phase")
    time.sleep(0.25)
    helpers = [pid for pid, command in process_children(lab.dm.pid) if "dojo-dm-pam" in command]
    check(helpers, "the winning helper is waiting for StartSession")
    for pid in helpers:
        os.kill(pid, signal.SIGKILL)
    failure = p.wait("d>g", start=mark, session_failed="helper_crashed")
    check(not p.seen("g>d", start=mark, start_session=ANY), "no session was started")
    retry = p.wait("g>d", start=failure["n"], begin=ANY, method="password", login="nathan")
    check(retry["begin"] != first["begin"], "the screen asks again on a new conversation")
    p.wait("d>g", start=retry["n"], report=retry["begin"], prompt=ANY)
    time.sleep(0.8)
    hint_shot = lab.shoot("helper-crashed", full=True)
    log_in(lab)
    start = p.wait("g>d", start=retry["n"], start_session=retry["begin"])
    environment_of(lab, REMEMBERED_SESSION)
    lab.release_session()
    mark = p.mark()
    p.wait("d>g", start=mark, init=True)
    lab.wait_locked(2)
    check(not p.seen("d>g", start=mark, session_failed=ANY), "a session that ended normally leaves no failure behind")
    begin = p.wait("g>d", start=mark, begin=ANY, method="password")
    p.wait("d>g", start=begin["n"], report=begin["begin"], prompt=ANY)

    (lab.dir / "refuse-session").write_text("")
    mark = p.mark()
    time.sleep(0.5)
    log_in(lab)
    p.wait("g>d", start=mark, start_session=begin["begin"])
    lab.wait_greeter_exit(2)
    mark_next = p.wait("d>g", start=mark, init=True)
    lab.wait_locked(3)
    reason = p.wait("d>g", start=mark_next["n"], session_failed="refused")
    check(reason["n"] > mark_next["n"], "the reason follows the fresh Init")
    time.sleep(1.0)
    refused_shot = lab.shoot("refused", full=True)
    (lab.dir / "refuse-session").unlink()
    mark = p.mark()
    begin = p.wait("g>d", start=reason["n"], begin=ANY, method="password")
    lab.wait_greeter_log(3, "event=dm-session-failed reason=Refused")
    return {"hint_shots": [str(hint_shot), str(refused_shot)], "unexpected_dm_log": ignored(lab)}


def scenario_failed_spawn(lab):
    """The session command cannot be created: only the next greeter can say so, once."""
    p = lab.proxy
    lab.wait_locked(1)
    begin = p.wait("g>d", begin=ANY, method="password")
    p.wait("d>g", report=begin["begin"], prompt=ANY)
    time.sleep(0.5)
    log_in(lab)
    p.wait("g>d", start_session=begin["begin"])
    lab.wait_greeter_exit(1)
    init = p.wait("d>g", init=True, count=2)
    failure = p.wait("d>g", start=init["n"], session_failed="spawn_failed")
    lab.wait_locked(2)
    lab.wait_greeter_log(2, "event=dm-session-failed reason=SpawnFailed")
    time.sleep(1.0)
    shot = lab.shoot("spawn-failed", full=True)
    # Killing this greeter must not replay the reason to the one after it.
    os.kill(lab.greeter_pid(2), signal.SIGKILL)
    init3 = p.wait("d>g", init=True, count=3)
    lab.wait_locked(3)
    time.sleep(0.5)
    check(not p.seen("d>g", start=init3["n"], session_failed=ANY), "a reason is told once")
    return {"shot": str(shot), "unexpected_dm_log": ignored(lab)}


def scenario_power(lab):
    """Ctrl+Alt+P plays the farewell, then Power{shutdown} is recorded by the fake runner and nothing else happens."""
    p = lab.proxy
    lab.wait_locked(1)
    begin = p.wait("g>d", begin=ANY, method="password")
    p.wait("d>g", report=begin["begin"], prompt=ANY)
    time.sleep(0.5)
    lab.press("ctrl", "alt", "p")
    power = p.wait("g>d", power="shutdown")
    check(p.seen("g>d", cancel=begin["begin"]), "the conversation is cancelled when the farewell starts")
    G.wait_for(lab.power_log.exists, "the power log")
    check(lab.power_log.read_text() == "PowerOff\n", lab.power_log.read_text())
    check(lab.dm.poll() is None and not (lab.dir / "session.out").exists(), "nothing else happened")
    return {"power_log": lab.power_log.read_text().split(), "unexpected_dm_log": ignored(lab)}


def scenario_dm_killed(lab):
    """SIGKILL on the dm closes the greeter socket; the greeter, which outlives the dm here, must exit non-zero."""
    lab.wait_locked(1)
    lab.proxy.wait("g>d", begin=ANY, method="password")
    lab.dm.kill()
    lab.dm.wait()
    code = lab.wait_greeter_exit(1)
    check(code != 0, "a lost display manager must not be a clean greeter exit")
    return {"greeter_exit": code, "greeter_log": lab.greeter_log(1).strip().splitlines()[-2:]}


def scenario_greeter_killed(lab):
    """SIGKILL on the greeter: the dm starts another, which connects, locks and offers the same login."""
    p = lab.proxy
    lab.wait_locked(1)
    first = p.wait("g>d", begin=ANY, method="password")
    p.wait("d>g", report=first["begin"], prompt=ANY)
    pid = lab.greeter_pid(1)
    os.kill(pid, signal.SIGKILL)
    lab.wait_greeter_exit(1)
    mark = p.mark()
    p.wait("d>g", start=mark, init=True)
    lab.wait_locked(2)
    begin = p.wait("g>d", start=mark, begin=ANY, method="password", login="nathan")
    p.wait("d>g", start=begin["n"], report=begin["begin"], prompt=ANY)
    check(lab.dm.poll() is None, "dojo-dm kept running")
    time.sleep(0.5)
    shot = lab.shoot("restarted", full=True)
    log_in(lab)
    p.wait("g>d", start=begin["n"], start_session=begin["begin"])
    environment_of(lab, REMEMBERED_SESSION)
    return {"shot": str(shot), "unexpected_dm_log": ignored(lab)}


def processes_with(argv0):
    """Pids whose first argument is `argv0`."""
    found = []
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit():
            try:
                if (entry / "cmdline").read_bytes().split(b"\0")[0] == str(argv0).encode():
                    found.append(int(entry.name))
            except OSError:
                pass
    return found


def scenario_installed_greeter_config(lab):
    """The dm starts Niri with the installed greeter.kdl, as it does on a boot; the greeter is Niri's client."""
    p = lab.proxy
    lab.wait_locked(1)
    first = p.wait("g>d", begin=ANY, method="password")
    p.wait("d>g", report=first["begin"], prompt=ANY)
    greeters = G.wait_for(lambda: processes_with(lab.dir / "release/dojo-greeter"), "the greeter process")
    for pid in greeters:
        os.kill(pid, signal.SIGKILL)
    # The dm watches Niri, not the greeter: Niri has to go with its only client or nothing restarts.
    code = lab.wait_greeter_exit(1, timeout=15.0)
    mark = p.mark()
    p.wait("d>g", start=mark, init=True)
    lab.wait_locked(2)
    begin = p.wait("g>d", start=mark, begin=ANY, method="password")
    p.wait("d>g", start=begin["n"], report=begin["begin"], prompt=ANY)
    time.sleep(0.8)
    shot = lab.shoot("second-niri", full=True)
    log_in(lab)
    p.wait("g>d", start=begin["n"], start_session=begin["begin"])
    environment_of(lab, REMEMBERED_SESSION)
    code2 = lab.wait_greeter_exit(2)
    return {"niri_exit_after_greeter_killed": code, "niri_exit_after_handover": code2, "shot": str(shot),
            "unexpected_dm_log": ignored(lab)}


def scenario_greeter_that_cannot_connect(lab):
    """A greeter that exits before it connects must still count as a failed start (three in 30 s, then 69)."""
    started = time.monotonic()
    code = lab.dm.wait(timeout=60)
    took = time.monotonic() - started
    check(code == 69, f"dojo-dm should give up with 69, not {code}:\n{lab.dm_text()}")
    check(took < 30, f"the failed starts were counted inside their 30 s window ({took:.0f} s)")
    starts = len(list(lab.dir.glob("greeter-*.log")))
    check(starts == 3, f"three starts, not {starts}")
    return {"dm_exit": code, "seconds": round(took, 1), "starts": starts}


SCENARIOS = {
    "login": (scenario_login, {}),
    "fingerprint": (scenario_fingerprint, {"fingerprints": ["nathan"]}),
    "failed_session": (scenario_failed_session, {}),
    "failed_spawn": (scenario_failed_spawn, {"session_shell": "/nonexistent/shell"}),
    "power": (scenario_power, {}),
    "dm_killed": (scenario_dm_killed, {}),
    "greeter_killed": (scenario_greeter_killed, {}),
    "installed_greeter_config": (scenario_installed_greeter_config, {"dm_starts_niri": True}),
    "greeter_that_cannot_connect": (scenario_greeter_that_cannot_connect,
                                    {"dm_starts_niri": True, "greeter_socket": "/nonexistent/greeter.sock"}),
}


class Run:
    def __init__(self, args):
        self.evidence = Path(args.evidence or tempfile.mkdtemp(prefix="dojo-login-smoke."))
        self.evidence.mkdir(exist_ok=True)
        target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")) / "debug"
        self.dm, self.helper, self.greeter = (Path(args.bin_dir or target) / name for name in
                                              ("dojo-dm", "dojo-dm-pam", "dojo-greeter"))
        self.niri = Path(os.environ.get("NIRI_BINARY", ROOT / "vendor/niri/target/debug/niri"))
        for binary in (self.dm, self.helper, self.greeter):
            check(binary.exists(), f"missing {binary}; see the build line in this script's docstring")
        self.wtype = shutil.which("wtype") or str(Path(subprocess.check_output(
            ["nix", "build", "--no-link", "--print-out-paths", "nixpkgs#wtype"], text=True).strip()) / "bin/wtype")
        self.grim = shutil.which("grim") or str(Path(subprocess.check_output(
            ["nix", "build", "--no-link", "--print-out-paths", "nixpkgs#grim"], text=True).strip()) / "bin/grim")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--only", nargs="*", choices=sorted(SCENARIOS))
    parser.add_argument("--bin-dir", help="directory holding the fake-pam-stack dojo-dm, dojo-dm-pam and dojo-greeter")
    parser.add_argument("--evidence", help="directory for logs, transcripts and screenshots")
    parser.add_argument("--keep", action="store_true", help="leave the evidence directory in place")
    args = parser.parse_args()
    if not os.environ.get("WAYLAND_DISPLAY"):
        raise SystemExit("run this from a Wayland session: it starts nested Niris as clients of it")
    run = Run(args)
    results, failed = {"evidence": str(run.evidence)}, False
    try:
        for name, (scenario, options) in SCENARIOS.items():
            if args.only and name not in args.only:
                continue
            lab = Lab(run, name, **options)
            try:
                results[name] = scenario(lab)
            except BaseException as error:
                failed = True
                results[name] = {"FAILED": f"{type(error).__name__}: {error}"}
                (lab.dir / "dm.log.tail").write_text(lab.dm_text()[-4000:])
                print(f"--- {name} failed; dm log tail:\n{lab.dm_text()[-1500:]}", file=sys.stderr)
            finally:
                lab.stop()
        print(json.dumps(results, indent=2))
    finally:
        if args.keep or failed:
            print(f"evidence kept in {run.evidence}")
        else:
            shutil.rmtree(run.evidence, ignore_errors=True)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
