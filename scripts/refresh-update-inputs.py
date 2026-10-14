#!/usr/bin/env python3
"""Refresh explicit GitHub input revisions without reformatting devenv.yaml."""

import argparse
import datetime
import fcntl
import json
import os
from pathlib import Path
import re
import stat
import sys
import tempfile
from urllib.parse import urlencode
from urllib.request import Request, urlopen


INPUT = re.compile(r"^  ([A-Za-z0-9_-]+):\s*(?:#.*)?$")
URL = re.compile(r"^(    url:\s*)([^\r\n]*)(\r?\n)?$")
PIN = re.compile(r"github:([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)/([0-9a-fA-F]{40})(\?[^\s'\"]*)?")
UNBUFFERED = {"pi", "opencode", "codex", "claude", "t3code"}


def parse_pins(source):
    lines = source.decode("utf-8").splitlines(keepends=True)
    active = False
    name = None
    names = set()
    urls = set()
    pins = []
    for index, line in enumerate(lines):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if not line[0].isspace():
            active = stripped == "inputs:"
            name = None
            continue
        if not active:
            continue
        match = INPUT.fullmatch(line.rstrip("\r\n"))
        if match:
            name = match[1]
            if name in names:
                raise ValueError(f"Duplicate input {name}")
            names.add(name)
            continue
        match = URL.fullmatch(line)
        if not match:
            if stripped.startswith("url:"):
                raise ValueError("Input URLs must use a four-space indentation")
            continue
        if name is None or name in urls:
            raise ValueError("Missing input name or duplicate input URL")
        urls.add(name)
        value = match[2].strip()
        if value.startswith(("'", '"')):
            marker = value[0]
            closing = value.find(marker, 1)
            if closing == -1 or value[closing + 1:].strip().split("#", 1)[0]:
                raise ValueError(f"Invalid quoted URL for {name}")
            value = value[1:closing]
        else:
            value = value.split(" #", 1)[0].rstrip()
        pin = PIN.fullmatch(value)
        if pin:
            owner, repository, revision, _ = pin.groups()
            start = line.index(value) + len(f"github:{owner}/{repository}/")
            pins.append((name, index, start, owner, repository, revision))
        elif value.startswith("github:"):
            raise ValueError(f"Input {name} is not an explicit 40-character GitHub revision")
    if not pins:
        raise ValueError("No explicit GitHub input revisions found")
    return lines, pins


def github_json(endpoint):
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "dojo-update"}
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    request = Request(f"https://api.github.com/{endpoint}", headers=headers)
    with urlopen(request, timeout=30) as response:
        payload = response.read(1_048_577)
    if len(payload) > 1_048_576:
        raise ValueError("GitHub response exceeds one megabyte")
    value = json.loads(payload)
    return value


def default_revision(owner, repository, cutoff):
    parameters = {"per_page": "1"}
    if cutoff is not None:
        parameters["until"] = cutoff.isoformat().replace("+00:00", "Z")
    commits = github_json(f"repos/{owner}/{repository}/commits?{urlencode(parameters)}")
    if not isinstance(commits, list) or len(commits) != 1 or not isinstance(commits[0], dict):
        raise ValueError(f"GitHub returned no eligible commit for {owner}/{repository}")
    commit = commits[0]
    revision = commit.get("sha")
    if not isinstance(revision, str) or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError(f"GitHub returned an invalid revision for {owner}/{repository}")
    if cutoff is not None:
        try:
            timestamp = commit["commit"]["committer"]["date"]
            if not isinstance(timestamp, str):
                raise ValueError("Timestamp must be a string")
            committed = datetime.datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
            if committed.tzinfo is None or committed > cutoff:
                raise ValueError("Timestamp does not satisfy the age buffer")
        except (KeyError, TypeError, ValueError) as error:
            raise ValueError(f"GitHub returned an ineligible commit date for {owner}/{repository}") from error
    return revision


def snapshot(path):
    with path.open("rb") as source:
        metadata = os.fstat(source.fileno())
        content = source.read()
    return content, (metadata.st_dev, metadata.st_ino, metadata.st_mtime_ns, metadata.st_size), metadata.st_mode


def require_unchanged(path, original, identity):
    current, current_identity, _ = snapshot(path)
    if current != original or current_identity != identity:
        raise ValueError("devenv.yaml changed during refresh; refusing to overwrite it")


def replace_source(path, original, replacement, identity, mode, state):
    require_unchanged(path, original, identity)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    backup_fd, backup_name = tempfile.mkstemp(prefix=f"devenv.yaml-{stamp}-", dir=state)
    with os.fdopen(backup_fd, "wb") as backup:
        backup.write(original)
        backup.flush()
        os.fsync(backup.fileno())
    temporary = None
    try:
        temporary_fd, temporary = tempfile.mkstemp(prefix=".devenv.yaml-update-", dir=path.parent)
        with os.fdopen(temporary_fd, "wb") as output:
            os.fchmod(output.fileno(), stat.S_IMODE(mode))
            output.write(replacement)
            output.flush()
            os.fsync(output.fileno())
        require_unchanged(path, original, identity)
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            Path(temporary).unlink(missing_ok=True)
    return backup_name


def utc_now():
    return datetime.datetime.now(datetime.timezone.utc)


def refresh(repo, dry_run=False, buffer_days=3):
    repo = Path(repo).resolve()
    path = repo / "devenv.yaml"
    if path.is_symlink():
        raise ValueError("devenv.yaml must be a regular file, not a symlink")
    state = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state"))) / "dojo-update"
    state = state.resolve()
    if state == repo or repo in state.parents:
        raise ValueError("Update backups must live outside the repository")
    lock = None
    try:
        if not dry_run:
            state.mkdir(parents=True, exist_ok=True)
            lock = (state / "inputs.lock").open("a")
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise ValueError("Another input refresh is running") from None
        original, identity, mode = snapshot(path)
        lines, pins = parse_pins(original)
        cutoff = utc_now() - datetime.timedelta(days=buffer_days)
        revisions = {}
        changes = []
        for name, index, start, owner, repository, old in pins:
            input_cutoff = None if name in UNBUFFERED else cutoff
            key = (owner, repository, input_cutoff)
            if key not in revisions:
                revisions[key] = default_revision(*key)
            new = revisions[key]
            lines[index] = lines[index][:start] + new + lines[index][start + 40:]
            changes.append((name, old, new))
        replacement = "".join(lines).encode("utf-8")
        require_unchanged(path, original, identity)
        backup = None
        if not dry_run and replacement != original:
            backup = replace_source(path, original, replacement, identity, mode, state)
        for name, old, new in changes:
            print(f"{name}: {old} -> {new}" if old != new else f"{name}: unchanged ({old})")
        if dry_run:
            print("Dry run: devenv.yaml was not changed")
        elif backup:
            print(f"Backup: {backup}")
        else:
            print("All input revisions are unchanged")
    finally:
        if lock is not None:
            lock.close()


def nonnegative_days(value):
    days = int(value)
    if days < 0:
        raise argparse.ArgumentTypeError("buffer days must be nonnegative")
    return days


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--buffer-days", type=nonnegative_days, default=3)
    arguments = parser.parse_args(argv)
    try:
        refresh(arguments.repo, arguments.dry_run, arguments.buffer_days)
    except (OSError, ValueError, OverflowError) as error:
        print(f"Input refresh failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
