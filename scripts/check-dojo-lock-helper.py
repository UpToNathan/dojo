#!/usr/bin/env python3
"""Check that dojo-lock-pam starts under the host loader and can load libpam.

dojo-lock spawns the helper the way src/lock/worker/mod.rs does: the host's
dynamic loader with the host library directory ahead of the binary's RUNPATH.
Nothing else loads libpam, so a missing or ABI-incompatible libpam would only
show when the screen is already locked. This runs the same command at install
time without touching the real PAM stack:

1. With no arguments the helper must refuse with its usage status (64).
2. With --check it dlopens libpam, resolves the symbols it needs, prints "ok"
   and exits 0. It opens no PAM service and no socket, so no user namespace is
   needed. Any other status carries the failing dlopen or dlsym step on stderr.
"""

import argparse
import subprocess

LOADER = "/usr/lib64/ld-linux-x86-64.so.2"
LIBRARIES = "/usr/lib"
EXIT_USAGE = 64
EXIT_UNAVAILABLE = 69
TIMEOUT = 15


def run_usage(command):
    result = subprocess.run(
        command,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={"PATH": "/usr/bin"},
        timeout=TIMEOUT,
    )
    if result.returncode != EXIT_USAGE:
        detail = result.stderr.decode(errors="replace").strip()
        raise SystemExit(
            f"dojo-lock-pam cannot start under {command[0]} with library path {command[2]}"
            f" (exit {result.returncode}, expected {EXIT_USAGE}). {detail}"
        )


def run_check(command):
    return subprocess.run(
        command + ["--check"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={"PATH": "/usr/bin"},
        timeout=TIMEOUT,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("helper")
    parser.add_argument("--loader", default=LOADER)
    parser.add_argument("--library-path", default=LIBRARIES)
    arguments = parser.parse_args()

    command = [arguments.loader, "--library-path", arguments.library_path, arguments.helper]
    run_usage(command)
    result = run_check(command)
    detail = result.stderr.decode(errors="replace").strip()
    if result.returncode == EXIT_UNAVAILABLE:
        raise SystemExit(
            f"libpam cannot be loaded under {arguments.loader} with library path"
            f" {arguments.library_path}: install the pam package or fix the library path."
            f" {detail}"
        )
    if result.returncode != 0 or result.stdout.strip() != b"ok":
        raise SystemExit(
            f"dojo-lock-pam --check failed (exit {result.returncode},"
            f" stdout {result.stdout.strip()!r}). {detail}"
        )
    print("dojo-lock-pam starts under the host loader and libpam loads")


if __name__ == "__main__":
    main()
