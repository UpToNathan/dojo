#!/usr/bin/env python3
"""Builds private confdirs under data/ and checks race/cancel semantics with stand-in PAM stacks."""
import os, subprocess, sys, time, shutil
ROOT = "/tmp/dojo-lock-pam-spike"
H = f"{ROOT}/bin/helpers"
HARNESS = f"{ROOT}/bin/dojo_pam_race.py"
DATA = f"{ROOT}/data/scenarios"
shutil.rmtree(DATA, ignore_errors=True)
os.makedirs(DATA)
open(f"{DATA}/finger-msg.txt", "w").write("Place your finger on the reader (stand-in)")

def stack(conf, service, text):
    os.makedirs(conf, exist_ok=True)
    open(f"{conf}/{service}", "w").write(text)

def slow_fp(secs, marker, name="fp"):
    return (f"auth optional pam_echo.so file={DATA}/finger-msg.txt\n"
            f"auth required pam_exec.so quiet {H}/sleep_ok.sh {secs} {marker} {name}\n")
PW = f"auth requisite pam_exec.so quiet expose_authtok {H}/check_fake_pw.sh\n"
DENY = "auth required pam_deny.so\n"
CRASH = f"auth optional pam_echo.so file={DATA}/finger-msg.txt\nauth required pam_exec.so quiet {H}/crash_parent.sh\n"
SUCCEED = "auth requisite pam_succeed_if.so user = nathan\n"
FAILIF = "auth requisite pam_succeed_if.so user = nobody\n"

def run(name, fp, pw, args=(), env=None, expect_rc=None, expect=(), absent=(), post=None):
    conf = f"{DATA}/{name}"; marker = f"{conf}/markers"; os.makedirs(marker, exist_ok=True)
    stack(conf, "svc-fp", fp(marker) if callable(fp) else fp)
    stack(conf, "svc-pw", pw)
    cmd = [sys.executable, "-I", HARNESS, "--confdir", conf, "--user", "nathan",
           "--worker", "fp=svc-fp", "--worker", "pw=svc-pw", *args]
    e = dict(os.environ, **(env or {}))
    print(f"\n=== {name}")
    p = subprocess.run(cmd, capture_output=True, text=True, env=e)
    print(p.stdout.rstrip()); print(f"exit={p.returncode}")
    ok = (expect_rc is None or p.returncode == expect_rc) and all(s in p.stdout for s in expect) and not any(s in p.stdout for s in absent)
    if post:
        time.sleep(post[0]); ok = ok and post[1](marker)
    print("PASS" if ok else "FAIL")
    return ok

res = []
FAKE = {"SPIKE_PW": "fake-spike-secret"}
WRONG = {"SPIKE_PW": "wrong"}
no = lambda f: (lambda m: not os.path.exists(f"{m}/{f}"))
yes = lambda f: (lambda m: os.path.exists(f"{m}/{f}"))

# A: password wins while fingerprint worker mid-wait -> fp worker (and its sleeping helper) killed
res.append(run("A-password-wins", lambda m: slow_fp(4, m), PW, ["--reply-env", "SPIKE_PW", "--reply-delay", "1"], FAKE, 0,
     ["WINNER: pw", "cancel fp", "survivors=none"], [], (4.5, lambda m: yes("fp.started")(m) and no("fp.completed")(m))))
# B: fingerprint wins while password worker blocked on prompt -> pw worker killed
res.append(run("B-fingerprint-wins", lambda m: slow_fp(1.5, m), PW, [], None, 0,
     ["WINNER: fp", "PROMPT 'Password: '", "cancel pw", "survivors=none"]))
# C: both fail (fp denied, wrong password)
res.append(run("C-both-fail", DENY, PW, ["--reply-env", "SPIKE_PW", "--reply-delay", "0.5"], WRONG, 1, ["ALL FAILED"]))
# D: fp worker hard-crashes, password then wins
res.append(run("D-fp-crash-pw-wins", CRASH, PW, ["--reply-env", "SPIKE_PW", "--reply-delay", "1.5"], FAKE, 0,
     ["fp: CRASHED/EXITED", "WINNER: pw"]))
# E: both crash/die -> all failed
res.append(run("E-both-crash", CRASH, CRASH, [], None, 1, ["fp: CRASHED", "pw: CRASHED", "ALL FAILED"]))
# F: window expires, both blocked -> everything killed
res.append(run("F-window-expires", lambda m: slow_fp(30, m), PW, ["--window", "2"], None, 2, ["WINDOW EXPIRED", "cancel fp", "cancel pw"],
     [], (0, lambda m: no("fp.completed")(m))))
# G: pam_succeed_if stand-ins: instant success vs instant failure
res.append(run("G-succeed_if", SUCCEED, FAILIF, [], None, 0, ["WINNER: fp"]))
# H: fingerprint fails fast, password still can win afterwards (failure of one does not cancel other)
res.append(run("H-fp-fails-pw-wins", DENY, PW, ["--reply-env", "SPIKE_PW", "--reply-delay", "1"], FAKE, 0, ["fp: pam_authenticate -> PAM_AUTH_ERR", "WINNER: pw"]))
print("\nSUMMARY:", "ALL PASS" if all(res) else "FAILURES", res)
sys.exit(0 if all(res) else 1)
