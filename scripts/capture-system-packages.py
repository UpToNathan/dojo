#!/usr/bin/env python3
"""Capture the Arch package baseline and enabled system units without sudo."""

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


PACKAGE = re.compile(r"[A-Za-z0-9@._+-]+")
UNIT = re.compile(r"[A-Za-z0-9_.:@\\-]+")


def read_command(arguments):
    result = subprocess.run(arguments, check=False, capture_output=True, text=True, env={**os.environ, "LC_ALL": "C"})
    if result.returncode:
        no_matches = arguments in (["pacman", "-Qqe"], ["pacman", "-Qqd"], ["pacman", "-Qqm"])
        if not (no_matches and result.returncode == 1 and not result.stdout and not result.stderr):
            raise subprocess.CalledProcessError(result.returncode, arguments, result.stdout, result.stderr)
    return result.stdout


def package_names(output):
    names = set()
    for line in output.splitlines():
        if not PACKAGE.fullmatch(line) or line in names:
            raise ValueError(f"Invalid or duplicate package name: {line!r}")
        names.add(line)
    return names


def installed_packages(output):
    packages = {}
    for line in output.splitlines():
        fields = line.split()
        if len(fields) != 2 or not PACKAGE.fullmatch(fields[0]) or fields[0] in packages:
            raise ValueError(f"Invalid or duplicate package record: {line!r}")
        name, version = fields
        if not version.isascii() or not all("!" <= char <= "~" for char in version):
            raise ValueError(f"Invalid package version: {version!r}")
        packages[name] = version
    if not packages:
        raise ValueError("pacman returned no installed packages")
    return packages


def enabled_units(output):
    units = set()
    for line in output.splitlines():
        fields = line.split()
        if len(fields) not in (2, 3) or not UNIT.fullmatch(fields[0]) or fields[1] != "enabled" or fields[0] in units:
            raise ValueError(f"Invalid or duplicate enabled system unit: {line!r}")
        units.add(fields[0])
    return units


def names_text(names):
    return "".join(f"{name}\n" for name in sorted(names))


def inventory_text(packages, names):
    return "".join(f"{name}\t{packages[name]}\n" for name in sorted(names))


def capture():
    packages = installed_packages(read_command(["pacman", "-Q"]))
    explicit = package_names(read_command(["pacman", "-Qqe"]))
    dependencies = package_names(read_command(["pacman", "-Qqd"]))
    foreign = package_names(read_command(["pacman", "-Qqm"]))
    names = set(packages)
    if explicit & dependencies or explicit | dependencies != names or not foreign <= names:
        raise ValueError("pacman package queries disagree; the package database may have changed during capture")
    unit_arguments = ["list-unit-files", "--state=enabled", "--no-pager", "--no-legend"]
    system_units = enabled_units(read_command(["systemctl", *unit_arguments]))
    user_units = enabled_units(read_command(["systemctl", "--user", *unit_arguments]))
    return {
        "system-config/packages/installed.tsv": inventory_text(packages, names),
        "system-config/packages/official-explicit.txt": names_text(explicit - foreign),
        "system-config/packages/dependencies.txt": names_text(dependencies),
        "system-config/packages/foreign.tsv": inventory_text(packages, foreign),
        "system-config/packages/foreign-explicit.txt": names_text(explicit & foreign),
        "system-config/services/system-enabled.txt": names_text(system_units),
        "system-config/services/user-enabled.txt": names_text(user_units),
    }


def atomic_write(path, content):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as output:
            os.fchmod(output.fileno(), 0o644)
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--check", action="store_true", help="Report baseline drift without writing")
    arguments = parser.parse_args(argv)
    try:
        manifests = capture()
        changed = []
        for relative, content in manifests.items():
            path = arguments.repo / relative
            if not path.exists() or path.read_bytes() != content.encode("utf-8"):
                changed.append(relative)
        if arguments.check:
            for relative in changed:
                print(f"Drift: {relative}")
            if not changed:
                print("System package and service baseline matches")
            return int(bool(changed))
        for relative in changed:
            atomic_write(arguments.repo / relative, manifests[relative])
            print(f"Captured: {relative}")
        if not changed:
            print("System package and service baseline is unchanged")
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"System baseline capture failed: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
