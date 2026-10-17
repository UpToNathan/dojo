#!/usr/bin/env python3
"""Check the model picker's onboarding and sign-in panel against a fake voice service.

Needs wtype and wlrctl on PATH and a compositor without agent input, for example:
DOJO_NESTED_AGENT_INPUT=0 nix shell nixpkgs#wtype nixpkgs#wlrctl -c devenv shell -- tools/agent-input/run-nested.sh --model-picker
"""
import argparse
import os
from pathlib import Path
import subprocess
import time

from run import Peer, screenshot, wait_for

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    directory = Path(args.log).parent
    runtime = directory / "shell-runtime"
    runtime.mkdir(mode=0o700)
    env = dict(os.environ)
    env["WAYLAND_DISPLAY"] = str(Path(os.environ["XDG_RUNTIME_DIR"]) / os.environ["WAYLAND_DISPLAY"])
    env["XDG_RUNTIME_DIR"] = str(runtime)
    env["XDG_STATE_HOME"] = str(directory / "shell-state")
    env["XDG_CONFIG_HOME"] = str(directory / "shell-config")
    env["XDG_DATA_HOME"] = str(directory / "shell-data")
    env["FAKE_VOICE_LOG"] = str(directory / "fake-voice.log")
    query = Peer(os.environ["NIRI_SOCKET"])
    binary = os.environ.get("DOJO_BINARY", str(root / "target/debug/dojo"))
    procs = []

    def layers():
        return {item["namespace"] for item in query.request("Layers")["Layers"]}

    def shot(name):
        time.sleep(0.4)
        return screenshot(query, str(directory / f"{name}.png"), show_pointer=False)

    def log():
        path = directory / "fake-voice.log"
        return path.read_text().splitlines() if path.exists() else []

    def type_text(text):
        result = subprocess.run(["wtype", text], env=env, timeout=10, capture_output=True, text=True)
        assert result.returncode == 0, f"wtype failed: {result.stderr}"
        time.sleep(0.3)

    def tap(key):
        name = {"enter": "Return", "esc": "Escape", "backspace": "BackSpace", "up": "Up", "down": "Down", "pagedown": "Next", "home": "Home"}[key]
        subprocess.run(["wtype", "-k", name], env=env, check=True, timeout=10)
        time.sleep(0.3)

    def click(x, y):
        pointer = ["wlrctl", "pointer"]
        subprocess.run(pointer + ["move", "-5000", "-5000"], env=env, check=True, timeout=10)
        subprocess.run(pointer + ["move", str(x), str(y)], env=env, check=True, timeout=10)
        time.sleep(0.2)
        subprocess.run(pointer + ["click", "left"], env=env, check=True, timeout=10)
        time.sleep(0.5)

    try:
        with open(directory / "shell.log", "w") as shell_log, open(directory / "fake.log", "w") as fake_log:
            procs.append(subprocess.Popen(["node", str(Path(__file__).parent / "fake-voice.mjs")], env=env, stdout=fake_log, stderr=fake_log))
            wait_for(lambda: (runtime / "dojo-voice.sock").exists(), "fake voice socket")
            shell = subprocess.Popen([binary, "--enable"], env=env, stdout=shell_log, stderr=shell_log)
            procs.append(shell)
            wait_for(lambda: any(n.startswith("dojo-frame-") for n in layers()), "shell frame")
            output_name, output_info = next(iter(query.request("Outputs")["Outputs"].items()))
            output = output_name
            size = output_info["logical"]
            print("output", output, size, flush=True)
            # The glyph asks for setup once the first catalog read finds no account.
            wait_for(lambda: "models" in log(), "initial catalog read")
            shot("01-bar-needs-setup")
            subprocess.run([binary, "--models"], env=env, timeout=5, check=True)
            wait_for(lambda: "dojo-model-picker" in layers(), "picker")
            shot("02-accounts-needs-setup")
            tap("enter")  # OpenCode Go: Sign in, opens the key editor
            shot("03-key-editor")
            type_text("bad")
            shot("03b-key-typed-masked")
            tap("enter")
            wait_for(lambda: "submit-opencode-key opencode-go" in log(), "rejected key submission")
            shot("04-key-rejected")
            type_text("goodkey")
            tap("enter")
            wait_for(lambda: log().count("submit-opencode-key opencode-go") == 2, "accepted key submission")
            shot("05-accounts-signed-in")
            tap("down")
            tap("down")
            tap("enter")  # ChatGPT: Sign in (device code)
            wait_for(lambda: "begin-chatgpt-login device_code" in log(), "ChatGPT login start")
            shot("06a-starting")
            time.sleep(2.2)
            shot("06-device-code")
            tap("esc")  # cancel the login
            wait_for(lambda: "cancel-login" in log(), "login cancellation")
            shot("07-cancelled")
            tap("enter")  # sign in again and let the fake service complete it
            wait_for(lambda: log().count("begin-chatgpt-login device_code") == 2, "second ChatGPT login")
            time.sleep(8)
            shot("08-both-signed-in")
            tap("up")
            tap("up")
            tap("enter")  # OpenCode Go models
            shot("09-models")
            type_text("kimi")
            shot("09b-filtered")
            type_text("zz")
            shot("09c-no-match")
            tap("esc")  # clears the filter, staying on the page
            tap("pagedown")
            shot("09d-page-down")
            tap("home")
            tap("down")
            tap("down")
            tap("enter")  # choose a second model and close
            wait_for(lambda: "select-model" in " ".join(log()), "model selection")
            wait_for(lambda: "dojo-model-picker" not in layers(), "picker closed after choosing")
            subprocess.run([binary, "--models"], env=env, timeout=5, check=True)
            wait_for(lambda: "dojo-model-picker" in layers(), "picker reopened")
            shot("10-accounts-after-choice")
            tap("enter")  # OpenCode Go models, cursor on the model in use (second row)
            for _ in range(3):
                tap("up")  # model, Back, then wraps to the footer: Sign out
            shot("11-sign-out-row")
            tap("enter")
            wait_for(lambda: "sign-out opencode-go" in log(), "sign out")
            shot("12-signed-out-one")
    finally:
        subprocess.run([binary, "--quit"], env=env, timeout=5, check=False)
        for proc in procs:
            proc.terminate()
        for proc in procs:
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
        query.close()


if __name__ == "__main__":
    main()
