#!/usr/bin/env python3
"""Throwaway spike: race N independent PAM conversations, first success wins.

Usage (parent):
  python3 -I dojo_pam_race.py --confdir DIR --user nathan \
      --worker fp=dojo-lock-fingerprint --worker pw=dojo-lock-password \
      [--window SECS] [--reply-env VAR --reply-delay SECS --reply-worker NAME]
      [--reply-tty]

Each worker is a separate OS process (own session / process group) running
pam_start_confdir + pam_authenticate with a custom conversation function.  The
worker speaks JSON lines on its stdout (events) and reads JSON lines on stdin
(prompt replies).  The parent never logs reply contents.

Exit code: 0 = some worker succeeded, 1 = all failed/crashed, 2 = window expired.
"""
import argparse, ctypes, ctypes.util, getpass, json, os, selectors, signal, subprocess, sys, termios, threading, time

PAM_NAMES = {0: "PAM_SUCCESS", 1: "PAM_OPEN_ERR", 3: "PAM_SERVICE_ERR", 4: "PAM_SYSTEM_ERR", 5: "PAM_BUF_ERR", 6: "PAM_PERM_DENIED",
             7: "PAM_AUTH_ERR", 8: "PAM_CRED_INSUFFICIENT", 9: "PAM_AUTHINFO_UNAVAIL", 10: "PAM_USER_UNKNOWN", 11: "PAM_MAXTRIES",
             13: "PAM_ACCT_EXPIRED", 19: "PAM_CONV_ERR", 20: "PAM_AUTHTOK_ERR", 26: "PAM_ABORT", 31: "PAM_INCOMPLETE"}
STYLES = {1: "PROMPT_ECHO_OFF", 2: "PROMPT_ECHO_ON", 3: "ERROR_MSG", 4: "TEXT_INFO"}


# --------------------------------------------------------------------------- worker
def run_worker(service, user, confdir):
    out = os.fdopen(os.dup(1), "w", buffering=1)   # protocol fds are CLOEXEC dups, so
    inp = os.fdopen(os.dup(0), "r")                # PAM-spawned children do not inherit them
    devnull = os.open(os.devnull, os.O_RDWR)
    os.dup2(devnull, 0); os.dup2(devnull, 1)

    def emit(**kw):
        kw["t"] = round(time.monotonic(), 4)
        out.write(json.dumps(kw) + "\n")

    libc = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
    libc.calloc.restype = ctypes.c_void_p
    libc.calloc.argtypes = [ctypes.c_size_t, ctypes.c_size_t]
    libc.strdup.restype = ctypes.c_void_p
    libc.strdup.argtypes = [ctypes.c_char_p]
    pam = ctypes.CDLL("libpam.so.0", use_errno=True)

    class PamMessage(ctypes.Structure):
        _fields_ = [("msg_style", ctypes.c_int), ("msg", ctypes.c_char_p)]

    class PamResponse(ctypes.Structure):
        _fields_ = [("resp", ctypes.c_void_p), ("resp_retcode", ctypes.c_int)]

    CONV = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_int, ctypes.POINTER(ctypes.POINTER(PamMessage)),
                            ctypes.POINTER(ctypes.POINTER(PamResponse)), ctypes.c_void_p)

    class PamConv(ctypes.Structure):
        _fields_ = [("conv", CONV), ("appdata_ptr", ctypes.c_void_p)]

    def conv(n, msgs, resp_out, _appdata):
        resp = libc.calloc(n, ctypes.sizeof(PamResponse))
        arr = ctypes.cast(resp, ctypes.POINTER(PamResponse))
        for i in range(n):
            m = msgs[i].contents
            style = STYLES.get(m.msg_style, str(m.msg_style))
            text = m.msg.decode("utf-8", "replace") if m.msg else ""
            if m.msg_style in (1, 2):
                emit(ev="prompt", style=style, text=text)       # UI must answer
                line = inp.readline()
                if not line:                                    # parent gone / cancelled
                    return 19                                   # PAM_CONV_ERR
                reply = json.loads(line)["reply"]
                arr[i].resp = libc.strdup(reply.encode())
                del reply
            else:
                emit(ev="msg", style=style, text=text)
        resp_out[0] = arr
        return 0

    conv_c = CONV(conv)
    pc = PamConv(conv_c, None)
    handle = ctypes.c_void_p()
    pam.pam_start_confdir.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.POINTER(PamConv),
                                      ctypes.c_char_p, ctypes.POINTER(ctypes.c_void_p)]
    pam.pam_authenticate.argtypes = [ctypes.c_void_p, ctypes.c_int]
    pam.pam_end.argtypes = [ctypes.c_void_p, ctypes.c_int]
    pam.pam_strerror.argtypes = [ctypes.c_void_p, ctypes.c_int]
    pam.pam_strerror.restype = ctypes.c_char_p
    rc = pam.pam_start_confdir(service.encode(), user.encode(), ctypes.byref(pc), confdir.encode(), ctypes.byref(handle))
    emit(ev="started", pid=os.getpid(), rc=rc)
    if rc != 0:
        emit(ev="result", rc=rc, name=PAM_NAMES.get(rc, str(rc)))
        return 3
    rc = pam.pam_authenticate(handle, 0)        # PAM_DISALLOW_NULL_AUTHTOK not set
    err = pam.pam_strerror(handle, rc).decode()
    emit(ev="result", rc=rc, name=PAM_NAMES.get(rc, str(rc)), strerror=err)
    pam.pam_end(handle, rc)
    return 0 if rc == 0 else 1


# --------------------------------------------------------------------------- parent
class W:
    def __init__(self, name, service, args):
        self.name, self.service = name, service
        self.p = subprocess.Popen(
            [sys.executable, "-I", os.path.abspath(__file__), "--worker-mode", service, "--user", args.user, "--confdir", args.confdir],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            start_new_session=True, text=True, bufsize=1)
        self.pgid = self.p.pid
        self.result = None   # (rc, name)
        self.eof = False
        self.pending_prompt = False
        self.buf = b""

    def lines(self):
        """Raw read (TextIOWrapper buffering would hide already-read lines from select()). None on EOF."""
        chunk = os.read(self.p.stdout.fileno(), 65536)
        if not chunk:
            return None
        self.buf += chunk
        *done, self.buf = self.buf.split(b"\n")
        return [d.decode("utf-8", "replace") for d in done]

    def reply(self, text):
        try:
            self.p.stdin.write(json.dumps({"reply": text}) + "\n"); self.p.stdin.flush()
        except (BrokenPipeError, ValueError):
            pass
        self.pending_prompt = False


def proc_table():
    out = {}
    for l in subprocess.run(["ps", "-e", "-o", "pid=,ppid=,pgid=,stat="], capture_output=True, text=True).stdout.splitlines():
        f = l.split()
        if len(f) == 4 and not f[3].startswith("Z"):
            out[int(f[0])] = (int(f[1]), int(f[2]))
    return out


def tree(root_pid, pgid):
    """Worker pid + all descendants + same-pgrp members. NOTE: pam_exec children call setsid(), so a
    plain killpg(worker pgrp) misses them; the ppid walk finds them (while the worker is still alive)."""
    t = proc_table()
    found = {root_pid} if root_pid in t else set()
    changed = True
    while changed:
        changed = False
        for pid, (ppid, pg) in t.items():
            if pid not in found and (ppid in found or pg == pgid):
                found.add(pid); changed = True
    return found


def alive(pids):
    t = proc_table()
    return sorted(p for p in pids if p in t)


def kill_tree(w):
    """SIGTERM then SIGKILL the worker and every descendant. Returns (surviving pids, tree size)."""
    victims = tree(w.p.pid, w.pgid)
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for pid in alive(victims):
            try: os.kill(pid, sig)
            except ProcessLookupError: pass
        deadline = time.monotonic() + 0.5
        while time.monotonic() < deadline:
            try: w.p.wait(timeout=0.02)
            except subprocess.TimeoutExpired: pass
            if not alive(victims): break
        if not alive(victims): break
    try: w.p.wait(timeout=1)
    except subprocess.TimeoutExpired: pass
    return alive(victims), len(victims)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--confdir", required=True)
    ap.add_argument("--user", default=os.environ.get("USER", "nathan"))
    ap.add_argument("--worker", action="append", default=[], help="NAME=SERVICE")
    ap.add_argument("--window", type=float, default=0, help="overall seconds before giving up (0 = none)")
    ap.add_argument("--reply-env", help="env var holding the answer for password prompts (never printed)")
    ap.add_argument("--reply-worker", help="only answer prompts from this worker (default: all)")
    ap.add_argument("--reply-delay", type=float, default=0.0)
    ap.add_argument("--reply-tty", action="store_true", help="ask on the controlling tty with echo off")
    ap.add_argument("--worker-mode", help=argparse.SUPPRESS)
    a = ap.parse_args()
    if a.worker_mode:
        sys.exit(run_worker(a.worker_mode, a.user, a.confdir))

    t0 = time.monotonic()
    def log(msg):
        print(f"[{time.monotonic()-t0:6.3f}s] {msg}", flush=True)

    tty_state = None
    if a.reply_tty:
        try:
            tty_state = termios.tcgetattr(0)
        except termios.error:
            pass
    workers = []
    sel = selectors.DefaultSelector()
    for spec in a.worker:
        name, service = spec.split("=", 1)
        w = W(name, service, a); workers.append(w)
        sel.register(w.p.stdout, selectors.EVENT_READ, w)
        log(f"spawn {name}: service={service} pid={w.p.pid} (own session/pgrp)")
    wake_r, wake_w = os.pipe()
    sel.register(wake_r, selectors.EVENT_READ, "wake")
    tty_answers = []

    def tty_ask(w, text):
        try:
            pw = getpass.getpass(f"[{w.name}] {text}")
        except Exception:
            pw = None
        tty_answers.append((w, pw)); os.write(wake_w, b"x")

    def handle_prompt(w, text):
        w.pending_prompt = True
        if a.reply_env and (not a.reply_worker or a.reply_worker == w.name):
            def later():
                time.sleep(a.reply_delay); w.reply(os.environ.get(a.reply_env, ""))
            threading.Thread(target=later, daemon=True).start()
        elif a.reply_tty:
            threading.Thread(target=tty_ask, args=(w, text), daemon=True).start()

    winner = None
    end = time.monotonic() + a.window if a.window else None
    while winner is None and any(not w.eof for w in workers):
        timeout = None if end is None else max(0, end - time.monotonic())
        if end is not None and timeout == 0:
            break
        for key, _ in sel.select(timeout):
            if key.data == "wake":
                os.read(wake_r, 100)
                while tty_answers:
                    w, pw = tty_answers.pop()
                    if pw is not None and not w.eof:
                        w.reply(pw)
                continue
            w = key.data
            lines = w.lines()
            if lines is None:
                w.eof = True; sel.unregister(w.p.stdout)
                try: w.p.wait(timeout=2)
                except subprocess.TimeoutExpired: pass
                if w.result is None:
                    log(f"{w.name}: CRASHED/EXITED without result (returncode={w.p.returncode})")
                continue
            for line in lines:
                ev = json.loads(line)
                if ev["ev"] == "msg":
                    log(f"{w.name}: {ev['style']} {ev['text']!r}")
                elif ev["ev"] == "prompt":
                    log(f"{w.name}: {ev['style']} PROMPT {ev['text']!r} -> forwarded to UI, worker blocked")
                    handle_prompt(w, ev["text"])
                elif ev["ev"] == "started":
                    log(f"{w.name}: pam_start_confdir rc={ev['rc']}")
                elif ev["ev"] == "result":
                    w.result = (ev["rc"], ev["name"])
                    log(f"{w.name}: pam_authenticate -> {ev['name']} ({ev['rc']})")
                    if ev["rc"] == 0:
                        winner = w; break
            if winner:
                break

    expired = winner is None and any(not w.eof for w in workers)
    # cancel everyone else (and reap the winner's group for stray helpers)
    for w in workers:
        if w is winner:
            continue
        if w.p.poll() is None or tree(w.p.pid, w.pgid):
            t1 = time.monotonic()
            survivors, n = kill_tree(w)
            log(f"cancel {w.name}: killed worker+descendants ({n} procs) in {time.monotonic()-t1:.3f}s; "
                f"returncode={w.p.returncode}; survivors={survivors or 'none'}")
    if winner:
        try: winner.p.wait(timeout=2)
        except subprocess.TimeoutExpired: pass
        left, _ = kill_tree(winner)
        log(f"WINNER: {winner.name} ({winner.service}); stray winner helpers after cleanup={left or 'none'}")
        rc = 0
    elif expired:
        log("WINDOW EXPIRED: no worker succeeded"); rc = 2
    else:
        log("ALL FAILED: " + ", ".join(f"{w.name}={w.result[1] if w.result else 'crash'}" for w in workers)); rc = 1
    if tty_state:
        termios.tcsetattr(0, termios.TCSADRAIN, tty_state)
    sys.exit(rc)


if __name__ == "__main__":
    main()
