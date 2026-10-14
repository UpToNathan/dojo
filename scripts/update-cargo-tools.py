#!/usr/bin/env python3
"""Update installed crates.io Cargo binaries after a publication buffer."""

import argparse
import datetime as dt
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from urllib.error import URLError
from urllib.parse import parse_qs, quote, urljoin, urlsplit
from urllib.request import Request, urlopen


CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
VERSION = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:\+[0-9A-Za-z.-]+)?$")
PRERELEASE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-[0-9A-Za-z.-]+(?:\+[0-9A-Za-z.-]+)?$")
INSTALL_KEY = re.compile(r"^(.+?) (\S+) \((.+)\)$")


def parse_version(value):
    match = VERSION.fullmatch(value)
    if not match:
        raise ValueError(f"unsupported non-stable or non-numeric version {value!r}")
    return tuple(int(part) for part in match.groups())


def cargo_home():
    return Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")))


def load_installs(home):
    import tomllib

    legacy_path = home / ".crates.toml"
    modern_path = home / ".crates2.json"
    legacy = tomllib.loads(legacy_path.read_text(encoding="utf-8"))
    entries = legacy.get("v1")
    if not isinstance(entries, dict):
        raise ValueError(".crates.toml has no valid [v1] install table")

    modern = {"installs": {}}
    if modern_path.exists():
        modern = json.loads(modern_path.read_text(encoding="utf-8"))
    modern_installs = modern.get("installs")
    if not isinstance(modern_installs, dict):
        raise ValueError(".crates2.json has no valid installs table")

    parsed_entries = []
    counts = {}
    for key, bins in entries.items():
        match = INSTALL_KEY.fullmatch(key)
        if not match:
            print(f"manual: cannot parse Cargo install entry {key!r}")
            continue
        name, installed, source = match.groups()
        counts[name] = counts.get(name, 0) + 1
        parsed_entries.append((key, name, installed, source, bins))

    crates = {}
    ambiguous = {name for name, count in counts.items() if count > 1}
    for name in sorted(ambiguous):
        print(f"manual: {name} has multiple installed Cargo versions or sources")
    for key, name, installed, source, bins in parsed_entries:
        if name in ambiguous:
            continue
        if source != CRATES_IO:
            print(f"manual: {name} {installed} comes from {source}")
            continue
        try:
            installed_key = parse_version(installed)
        except ValueError as error:
            print(f"manual: {name} {installed}: {error}")
            continue
        metadata = modern_installs.get(key)
        if not isinstance(metadata, dict):
            print(f"manual: {name} {installed} has no .crates2.json metadata")
            continue
        try:
            features = metadata.get("features", [])
            if (not isinstance(features, list) or not all(isinstance(item, str) for item in features)
                    or not isinstance(metadata.get("all_features", False), bool)
                    or not isinstance(metadata.get("no_default_features", False), bool)):
                raise ValueError("feature metadata is malformed")
            if not isinstance(bins, list) or not all(isinstance(item, str) for item in bins):
                raise ValueError("binary metadata is malformed")
            crate_bins = metadata.get("bins", bins)
            if not isinstance(crate_bins, list) or not all(isinstance(item, str) for item in crate_bins):
                raise ValueError("binary metadata is malformed")
            if metadata.get("profile", "release") != "release":
                raise ValueError("non-release profile cannot be preserved safely")
            if metadata.get("target") and metadata.get("target") != host_target(metadata):
                raise ValueError("cross-compiled target cannot be preserved safely")
        except ValueError as error:
            print(f"manual: {name} {installed}: {error}")
            continue
        crates[name] = {
            "installed": installed,
            "installed_key": installed_key,
            "features": features,
            "all_features": metadata.get("all_features", False),
            "no_default_features": metadata.get("no_default_features", False),
            "bins": crate_bins,
        }
    return crates


def host_target(metadata):
    # Cargo records the host even when no explicit target was requested. The
    # rustc metadata supplies the same value for comparison where available.
    rustc = metadata.get("rustc", "")
    match = re.search(r"host: (\S+)", rustc)
    return match.group(1) if match else None


def crates_io_versions(name):
    endpoint = f"https://crates.io/api/v1/crates/{quote(name, safe='')}/versions"
    next_url = endpoint + "?per_page=100"
    seen_urls = set()
    versions = []
    expected_total = None
    page_count = 0
    while next_url is not None:
        if next_url in seen_urls:
            raise ValueError("crates.io returned a repeated pagination cursor")
        seen_urls.add(next_url)
        request = Request(next_url, headers={"User-Agent": "dojo-update (https://github.com/dojo)"})
        with urlopen(request, timeout=30) as response:
            payload = response.read(4_194_305)
        if len(payload) > 4_194_304:
            raise ValueError("crates.io response exceeds four megabytes")
        result = json.loads(payload)
        page = result.get("versions") if isinstance(result, dict) else None
        meta = result.get("meta") if isinstance(result, dict) else None
        if not isinstance(page, list) or not isinstance(meta, dict):
            raise ValueError("crates.io returned invalid versions pagination metadata")
        if "total" not in meta or "next_page" not in meta:
            raise ValueError("crates.io omitted versions pagination metadata")
        total = meta.get("total")
        cursor = meta.get("next_page")
        if not isinstance(total, int) or isinstance(total, bool) or total < 0:
            raise ValueError("crates.io returned an invalid versions total")
        if expected_total is None:
            expected_total = total
        elif total != expected_total:
            raise ValueError("crates.io versions total changed during pagination")
        page_count += 1
        if len(page) > 100 or len(versions) + len(page) > total:
            raise ValueError("crates.io returned more versions than its pagination metadata allows")
        versions.extend(page)
        if cursor is None:
            if len(versions) != total:
                raise ValueError("crates.io ended pagination before all versions were read")
            next_url = None
            continue
        if not isinstance(cursor, str):
            raise ValueError("crates.io returned an invalid next_page cursor")
        resolved = urljoin(endpoint, cursor)
        parsed = urlsplit(resolved)
        params = parse_qs(parsed.query, strict_parsing=True)
        if (parsed.scheme != "https" or parsed.netloc != "crates.io" or parsed.path != urlsplit(endpoint).path
                or parsed.fragment or set(params) != {"per_page", "seek"}
                or params["per_page"] != ["100"] or len(params["seek"]) != 1 or not params["seek"][0]):
            raise ValueError("crates.io returned an unsupported next_page cursor")
        if len(versions) >= total:
            raise ValueError("crates.io returned a cursor after all versions were read")
        if not page or page_count >= (total + 99) // 100:
            raise ValueError("crates.io pagination cursor is inconsistent with the reported total")
        next_url = resolved
    return versions


def eligible_version(name, buffer_days, now):
    cutoff = now - dt.timedelta(days=buffer_days)
    candidates = []
    for version in crates_io_versions(name):
        if not isinstance(version, dict) or not isinstance(version.get("yanked"), bool):
            raise ValueError("crates.io returned a version with invalid yanked metadata")
        if version["yanked"]:
            continue
        value = version.get("num")
        published = version.get("created_at")
        if not isinstance(value, str) or not isinstance(published, str):
            raise ValueError("crates.io returned a version with incomplete publication metadata")
        if PRERELEASE.fullmatch(value):
            continue
        try:
            parsed = parse_version(value)
            timestamp = dt.datetime.fromisoformat(published.replace("Z", "+00:00"))
            if timestamp.tzinfo is None:
                raise ValueError("publication timestamp has no timezone")
        except (ValueError, OverflowError) as error:
            raise ValueError("crates.io returned invalid version or publication metadata") from error
        if timestamp <= cutoff:
            candidates.append((parsed, value))
    if not candidates:
        return None
    return max(candidates)[1]


def install_command(name, info, version):
    command = ["cargo", "install", name, "--version", f"={version}", "--locked"]
    features = info["features"]
    if features:
        command.extend(["--features", ",".join(features)])
    if info["all_features"]:
        command.append("--all-features")
    if info["no_default_features"]:
        command.append("--no-default-features")
    bins = info["bins"]
    for binary in bins:
        command.extend(["--bin", binary])
    return command


def update(home, buffer_days=3, dry_run=False, now=None):
    installs = load_installs(home)
    now = now or dt.datetime.now(dt.timezone.utc)
    failures = 0
    for name, info in sorted(installs.items()):
        try:
            available = eligible_version(name, buffer_days, now)
            if available is None:
                print(f"{name}: no stable, non-yanked version is old enough")
                continue
            if parse_version(available) <= info["installed_key"]:
                print(f"{name}: current ({info['installed']})")
                continue
            command = install_command(name, info, available)
            if dry_run:
                print(f"{name}: planned {subprocess.list2cmdline(command)}")
                continue
            print(f"{name}: installing {available}")
            subprocess.run(command, check=True)
            print(f"{name}: updated to {available}")
        except (OSError, URLError, ValueError, json.JSONDecodeError, subprocess.CalledProcessError) as error:
            failures += 1
            print(f"{name}: failed: {error}", file=sys.stderr)
    return 1 if failures else 0


def nonnegative_days(value):
    try:
        days = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("buffer days must be an integer") from error
    if days < 0 or days > 36500:
        raise argparse.ArgumentTypeError("buffer days must be between 0 and 36500")
    return days


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo-home", type=Path, default=cargo_home())
    parser.add_argument("--buffer-days", type=nonnegative_days, default=3)
    parser.add_argument("--dry-run", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        return update(arguments.cargo_home, arguments.buffer_days, arguments.dry_run)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"Cargo update failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
