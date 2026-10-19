#!/usr/bin/env python3
"""Exercise dojo-greeter against a fake display manager in a nested Niri.

Starts its own nested Niri as a client of the live display (never locking the live session),
a fake `dojo-dm` that speaks the greeter protocol on a private socket, and a `fake-pam-stack`
build of the greeter that reads private look snapshots. Keys are typed with wtype. Nothing
here touches PAM, logind or /etc, and power requests are only recorded.

  python3 tools/agent-input/run-greeter.py [--greeter target/debug/dojo-greeter] [--keep]

Prints what each scenario saw and where the evidence (dm.jsonl, greeter.log, screenshots) is.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time
import zlib

ROOT = Path(__file__).resolve().parents[2]
ANY = object()
TEST_WORD = "kumite"
SESSIONS = [("niri-uwsm", "Niri (UWSM)"), ("gnome", "GNOME")]
USERS = {"nathan": ("Nathan Duarte", False), "alyssa": ("Alyssa Park", True)}


def wait_for(check, description, timeout=10.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.02)
    raise AssertionError(f"Timed out waiting for {description}")


def text(value):
    data = value.encode()
    return struct.pack("<H", len(data)) + data


class Reader:
    def __init__(self, data):
        self.data = data

    def take(self, count):
        head, self.data = self.data[:count], self.data[count:]
        assert len(head) == count, "short datagram"
        return head

    def u8(self):
        return self.take(1)[0]

    def u32(self):
        return struct.unpack("<I", self.take(4))[0]

    def text(self):
        return self.take(struct.unpack("<H", self.take(2))[0]).decode()


def decode(datagram):
    """A greeter request as a dict, by the protocol's tag byte."""
    tag, reader = chr(datagram[0]), Reader(datagram[1:])
    if tag == "H":
        return {"hello": struct.unpack("<H", reader.take(2))[0]}
    if tag == "U":
        return {"select_user": reader.text()}
    if tag == "B":
        generation, method = reader.u32(), ("fingerprint", "password")[reader.u8()]
        return {"begin": generation, "method": method, "login": reader.text(), "session": reader.text()}
    if tag == "A":
        return {"answer": reader.u32(), "secret": reader.data.decode()}
    if tag == "C":
        return {"cancel": reader.u32()}
    if tag == "G":
        return {"start_session": reader.u32()}
    if tag == "W":
        return {"power": ("suspend", "hibernate", "restart", "shutdown")[reader.u8()]}
    raise AssertionError(f"unknown request {datagram!r}")


def init(last_login, last_session):
    body = b"\x01" + text(last_login) + b"\x01" + text(last_session) + bytes([len(SESSIONS)])
    for ident, name in SESSIONS:
        body += text(ident) + text(name)
    return b"i" + body


def user(login, display, fingerprint):
    return b"u" + text(login) + b"\x01" + text(display) + bytes([fingerprint])


def report(generation, method, kind, *fields):
    return b"r" + struct.pack("<IB", generation, ("fingerprint", "password").index(method)) + bytes([kind]) + b"".join(fields)


PROMPT, HINT, FINISHED = 0, 1, 4
SUCCESS, REJECTED = (0, 0), (1, 0)


def finished(outcome):
    return struct.pack("<Bi", *outcome)


class FakeDm:
    """The display manager's side of one greeter connection, recorded as JSON lines."""

    def __init__(self, path, log_path, fail_start):
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_SEQPACKET)
        self.server.bind(str(path))
        self.server.listen(1)
        self.log = log_path.open("a", buffering=1)
        self.fail_start = fail_start
        self.requests = []
        self.connection = None
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def say(self, datagram):
        self.connection.send(datagram)

    def serve(self):
        self.connection, _ = self.server.accept()
        while True:
            datagram = self.connection.recv(16 * 1024)
            if not datagram:
                self.record({"closed": True})
                return
            request = decode(datagram)
            self.record(request)
            self.react(request)

    def record(self, request):
        self.requests.append(request)
        self.log.write(json.dumps(request) + "\n")

    def react(self, request):
        if "hello" in request:
            self.say(init("nathan", "niri-uwsm"))
        elif "select_user" in request:
            login = request["select_user"]
            display, fingerprint = USERS.get(login, (None, False))
            self.say(user(login, display or login, fingerprint))
        elif "begin" in request and request["method"] == "password":
            self.say(report(request["begin"], "password", PROMPT, b"\x00"))
        elif "begin" in request:
            self.say(report(request["begin"], "fingerprint", HINT, b"\x00", text("Touch the stand-in sensor")))
        elif "answer" in request:
            outcome = SUCCESS if request["secret"] == TEST_WORD else REJECTED
            self.say(report(request["answer"], "password", FINISHED, finished(outcome)))
        elif "start_session" in request:
            if self.fail_start == "hold":
                return
            self.say(b"f\x01" if self.fail_start else b"s")

    def seen(self, **match):
        def holds(request):
            return all((k in request) if v is ANY else request.get(k) == v for k, v in match.items())
        return [r for r in self.requests if holds(r)]


def write_png(path, width, height, pixel):
    rows = b"".join(b"\x00" + b"".join(bytes(pixel(x, y)) for x in range(width)) for y in range(height))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(rows, 6)) + chunk(b"IEND", b""))


def gradient(a, b):
    def pixel(x, y):
        t = (x / 319 + y / 179) / 2
        return [round(a[i] * (1 - t) + b[i] * t) for i in range(3)]
    return pixel


def prepare_snapshots(state):
    """nathan keeps the theme's palette; alyssa gets the same with its channels rotated."""
    theme = Path.home() / ".local/state/dojo/themes/current/palette.json"
    palette = theme.read_text()
    rotated = re.sub(r"#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})", r"#\3\1\2", palette)
    for login, (name, _) in USERS.items():
        directory = state / "users" / login
        directory.mkdir(parents=True)
        (directory / "palette.json").write_text(palette if login == "nathan" else rotated)
        colours = ((0x2a, 0x3a, 0x6a), (0x6a, 0x3a, 0x5a)) if login == "nathan" else ((0x6a, 0x4a, 0x2a), (0x2a, 0x6a, 0x4a))
        write_png(directory / "lock-wallpaper.png", 320, 180, gradient(*colours))
        (directory / "identity").write_text(name + "\n")
        (directory / "session.toml").write_text(
            "[session]\ntransition = true\ngreeting = true\nfarewell = true\nreduced_motion = false\n")


class Nested:
    def __init__(self, evidence):
        host = os.environ["WAYLAND_DISPLAY"]
        self.log_path = evidence / "niri.log"
        env = {k: v for k, v in os.environ.items() if k not in ("NIRI_SOCKET", "WAYLAND_SOCKET")}
        env.update({"WAYLAND_DISPLAY": host, "DOJO_AGENT_INPUT": "0", "RUST_LOG": "niri=info"})
        binary = os.environ.get("NIRI_BINARY", str(ROOT / "vendor/niri/target/debug/niri"))
        self.process = subprocess.Popen([binary, "--config", str(ROOT / "tools/agent-input/nested.kdl")],
                                        env=env, stdout=self.log_path.open("w"), stderr=subprocess.STDOUT)

        def ready():
            log = self.log_path.read_text(errors="replace")
            display = re.findall(r"listening on Wayland socket: (\S+)", log)
            socket_path = re.findall(r"IPC listening on: (\S+)", log)
            if self.process.poll() is not None:
                raise AssertionError(f"niri exited:\n{log}")
            return (display[-1], socket_path[-1]) if display and socket_path and Path(socket_path[-1]).is_socket() else None

        self.display, self.socket = wait_for(ready, "the nested niri")
        assert self.display != host, "the nested Niri must not be the live display"
        self.env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_SOCKET", "DISPLAY", "WAYLAND_DEBUG")}
        self.env.update({"WAYLAND_DISPLAY": self.display, "NIRI_SOCKET": self.socket})

    def stop(self):
        self.process.terminate()
        self.process.wait(timeout=10)


class Run:
    def __init__(self, args):
        self.evidence = Path(tempfile.mkdtemp(prefix="dojo-greeter-smoke."))
        self.binary = Path(args.greeter).resolve()
        self.state = self.evidence / "state"
        prepare_snapshots(self.state)
        self.nested = Nested(self.evidence)
        self.wtype = shutil.which("wtype") or str(Path(subprocess.check_output(
            ["nix", "build", "--no-link", "--print-out-paths", "nixpkgs#wtype"], text=True).strip()) / "bin/wtype")
        self.grim = shutil.which("grim") or str(Path(subprocess.check_output(
            ["nix", "build", "--no-link", "--print-out-paths", "nixpkgs#grim"], text=True).strip()) / "bin/grim")
        self.results = {"evidence": str(self.evidence)}

    def type(self, value):
        subprocess.run([self.wtype, "-d", "15", value], env=self.nested.env, check=True)

    def press(self, *keys):
        """`press("ctrl", "Tab")` holds the modifiers, taps the last key and lets go."""
        *modifiers, key = keys
        command = [self.wtype]
        for modifier in modifiers:
            command += ["-M", modifier]
        command += ["-k", key]
        for modifier in reversed(modifiers):
            command += ["-m", modifier]
        subprocess.run(command, env=self.nested.env, check=True)

    def shoot(self, name):
        path = self.evidence / f"{name}.png"
        subprocess.run([self.grim, str(path)], env=self.nested.env, check=True)
        return str(path)

    def start(self, name, fail_start=False, extra=()):
        socket_path = self.evidence / f"{name}.sock"
        dm = FakeDm(socket_path, self.evidence / f"{name}-dm.jsonl", fail_start)
        log_path = self.evidence / f"{name}.log"
        env = dict(self.nested.env)
        env.update({"DOJO_DM_SOCKET": str(socket_path), "DOJO_DM_STATE": str(self.state)})
        process = subprocess.Popen([str(self.binary), *extra], env=env, stdout=subprocess.DEVNULL,
                                   stderr=log_path.open("w"))
        return Greeter(self, name, dm, process, log_path)

    def finish(self):
        self.nested.stop()


class Greeter:
    def __init__(self, run, name, dm, process, log_path):
        self.run, self.name, self.dm, self.process, self.log_path = run, name, dm, process, log_path

    def text(self):
        return self.log_path.read_text(errors="replace")

    def alive_or_fail(self, waiting_for):
        if self.process.poll() is not None:
            raise AssertionError(f"{self.name} exited with {self.process.returncode} before {waiting_for}:\n{self.text()}")
        return False

    def wait_log(self, needle, timeout=10.0):
        return wait_for(lambda: needle in self.text() or self.alive_or_fail(needle), f"{self.name} log {needle!r}", timeout)

    def wait_request(self, timeout=10.0, count=1, **match):
        def found():
            seen = self.dm.seen(**match)
            return seen[count - 1] if len(seen) >= count else self.alive_or_fail(str(match))
        return wait_for(found, f"{self.name} request #{count} {match}", timeout)

    def wait_exit(self, timeout=15.0):
        try:
            return self.process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            self.process.kill()
            raise AssertionError(f"{self.name} did not exit:\n{self.text()}")

    def stop(self):
        if self.process.poll() is None:
            self.process.terminate()
            self.process.wait(timeout=10)


def scenario_login(run):
    """Edit the user, change the session, wrong then right password, greeting, hand over."""
    greeter = run.start("login")
    greeter.wait_log("event=locked")
    greeter.wait_request(begin=ANY, method="password", login="nathan", session="niri-uwsm")
    time.sleep(1.0)
    shots = {"resting": run.shoot("login-resting")}

    run.press("Tab")
    run.type("alyssa")
    time.sleep(0.2)
    shots["editing"] = run.shoot("login-editing")
    run.press("Return")
    greeter.wait_request(select_user="alyssa")
    greeter.wait_request(begin=ANY, method="password", login="alyssa", session="niri-uwsm")
    # Alyssa has a print: the sensor starts once the display manager says so.
    greeter.wait_request(begin=ANY, method="fingerprint", login="alyssa", session="niri-uwsm")
    time.sleep(0.6)
    shots["alyssa"] = run.shoot("login-alyssa")

    run.press("ctrl", "Tab")
    greeter.wait_request(begin=ANY, method="password", login="alyssa", session="gnome")
    greeter.wait_request(begin=ANY, method="fingerprint", login="alyssa", session="gnome")
    time.sleep(0.3)
    shots["gnome"] = run.shoot("login-gnome")

    run.type("nope")
    run.press("Return")
    greeter.wait_request(answer=ANY)
    time.sleep(0.15)
    shots["wrong"] = run.shoot("login-wrong")
    greeter.wait_request(begin=ANY, method="password", login="alyssa", session="gnome", count=2)
    time.sleep(0.5)
    run.type(TEST_WORD)
    time.sleep(0.2)
    run.press("Return")
    time.sleep(0.45)
    shots["greeting"] = run.shoot("login-greeting")
    started = greeter.wait_request(start_session=ANY)
    code = greeter.wait_exit()
    assert code == 0, f"the greeter exited {code} after SessionStarted:\n{greeter.text()}"
    return {"requests": len(greeter.dm.requests), "start": started, "exit": code, "shots": shots}


def scenario_failed_start(run):
    """A session that does not start asks for the password again, with the reason as the hint."""
    greeter = run.start("failed", fail_start=True)
    greeter.wait_log("event=locked")
    greeter.wait_request(begin=ANY, method="password")
    time.sleep(0.5)
    run.type(TEST_WORD)
    run.press("Return")
    greeter.wait_request(start_session=ANY)
    greeter.wait_request(begin=ANY, method="password", count=2)
    time.sleep(0.6)
    shot = run.shoot("failed-hint")
    assert greeter.process.poll() is None, "the greeter must stay up after a failed start"
    greeter.stop()
    return {"shot": shot}


def cpu_seconds(pid):
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")


def scenario_handover_idle(run):
    """While the display manager has not answered StartSession the closed frame is still and costs nothing."""
    greeter = run.start("handover", fail_start="hold")
    greeter.wait_log("event=locked")
    greeter.wait_request(begin=ANY, method="password")
    time.sleep(0.5)
    run.type(TEST_WORD)
    run.press("Return")
    greeter.wait_request(start_session=ANY)
    time.sleep(1.0)
    before = cpu_seconds(greeter.process.pid)
    time.sleep(4.0)
    busy = (cpu_seconds(greeter.process.pid) - before) / 4.0
    shot = run.shoot("handover")
    greeter.dm.say(b"s")
    code = greeter.wait_exit()
    assert code == 0, code
    assert busy < 0.03, f"the greeter used {busy:.1%} of a core while waiting"
    return {"cpu_while_waiting": f"{busy:.2%}", "shot": shot}


def scenario_power(run):
    """Ctrl+Alt+P plays the farewell and then asks for the shutdown; nothing is carried out here."""
    greeter = run.start("power")
    greeter.wait_log("event=locked")
    greeter.wait_request(begin=ANY, method="password")
    time.sleep(0.5)
    run.press("ctrl", "alt", "p")
    time.sleep(0.5)
    shot = run.shoot("power-farewell")
    greeter.wait_request(power="shutdown")
    assert greeter.dm.seen(cancel=ANY), "the conversation is cancelled when the farewell starts"
    greeter.stop()
    return {"shot": shot}


def scenario_lost_dm(run):
    """A display manager that goes away makes the greeter exit non-zero, so dojo-dm restarts it."""
    greeter = run.start("lost")
    greeter.wait_log("event=locked")
    greeter.wait_request(begin=ANY, method="password")
    greeter.dm.connection.shutdown(socket.SHUT_RDWR)
    code = greeter.wait_exit()
    assert code != 0, "a lost display manager must not be a clean exit"
    return {"exit": code}


def scenario_refused(run):
    """No socket at all: the greeter exits at once with the unavailable status."""
    env = dict(run.nested.env)
    env["DOJO_DM_SOCKET"] = str(run.evidence / "nobody.sock")
    process = subprocess.run([str(run.binary)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                             text=True, timeout=20)
    assert process.returncode == 69, process.returncode
    return {"exit": process.returncode, "stderr": process.stderr.strip()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--greeter", default=str(ROOT / "target/debug/dojo-greeter"))
    parser.add_argument("--keep", action="store_true", help="leave the evidence directory in place")
    args = parser.parse_args()
    if not os.environ.get("WAYLAND_DISPLAY"):
        raise SystemExit("run this from a Wayland session: it starts a nested Niri as a client of it")
    run = Run(args)
    try:
        for scenario in (scenario_refused, scenario_login, scenario_failed_start, scenario_handover_idle, scenario_power, scenario_lost_dm):
            run.results[scenario.__name__] = scenario(run)
        print(json.dumps(run.results, indent=2))
    finally:
        run.finish()
        if args.keep:
            print(f"evidence kept in {run.evidence}")
        else:
            shutil.rmtree(run.evidence, ignore_errors=True)


if __name__ == "__main__":
    main()
