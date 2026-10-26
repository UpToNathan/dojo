#!/usr/bin/env python3
"""Install or update the T3 Code app from its GitHub releases.

The `t3` wrapper that Home Manager installs runs the app in ~/.local/opt/t3code; this puts the
app there. See .scratch/t3-install/spec.md.

    scripts/install-t3code.py [--channel nightly|preview|stable] [--version TAG] [--force] [--dry-run]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request

REPOSITORY = "pingdotgg/t3code"
CHANNELS = ("nightly", "preview", "stable")
MARKER = ".dojo-install.json"
ARCHITECTURES = {"x86_64": "x86_64", "amd64": "x86_64", "aarch64": "arm64", "arm64": "arm64"}


class InstallError(Exception):
    pass


def channel_of(tag):
    """The channel a release tag belongs to: v0.1.3-nightly.2026… is nightly, v0.1.3 is stable."""
    match = re.search(r"-(nightly|preview)\.", tag)
    return match.group(1) if match else "stable"


def pick_release(releases, channel, version=None):
    """The release to install: the exact tag when one is asked for, else the newest on the channel."""
    if version:
        wanted = {version, f"v{version}"}
        matches = [release for release in releases if release["tag_name"] in wanted]
    else:
        matches = [release for release in releases if channel_of(release["tag_name"]) == channel and not release.get("draft")]
    if not matches:
        raise InstallError(f"no release found for {version or 'channel ' + channel}")
    return max(matches, key=lambda release: release["published_at"])


def pick_asset(release, machine):
    arch = ARCHITECTURES.get(machine)
    if arch is None:
        raise InstallError(f"no T3 Code build for {machine}")
    for asset in release["assets"]:
        if re.fullmatch(rf"T3-Code-.+-{arch}\.AppImage", asset["name"]):
            return asset
    raise InstallError(f"{release['tag_name']} has no {arch} AppImage")


def parse_sums(text):
    """`SHA256SUMS` lines (`<hex>  <name>`) as a name-to-digest mapping."""
    sums = {}
    for line in text.splitlines():
        parts = line.split(None, 1)
        if len(parts) == 2 and re.fullmatch(r"[0-9a-f]{64}", parts[0]):
            sums[parts[1].strip().lstrip("*")] = parts[0]
    return sums


def expected_digests(asset, sums):
    """Every digest the asset must match: what GitHub reports and what the release lists."""
    found = set()
    reported = asset.get("digest") or ""
    if reported.startswith("sha256:"):
        found.add(reported.removeprefix("sha256:"))
    if asset["name"] in sums:
        found.add(sums[asset["name"]])
    if not found:
        raise InstallError(f"no checksum is published for {asset['name']}")
    return found


def render_desktop_entry(text, prefix):
    """The entry shipped inside the app, with `Exec` and `Icon` made absolute."""
    lines = []
    for line in text.splitlines():
        if line.startswith("Exec=") and not line.startswith("Exec=/"):
            line = f"Exec={prefix}/{line.removeprefix('Exec=')}"
        elif line == "Icon=t3code":
            line = f"Icon={prefix}/t3code.png"
        lines.append(line)
    return "\n".join(lines) + "\n"


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def fetch(url, accept=None):
    request = urllib.request.Request(url, headers={"User-Agent": "dojo-install-t3code", **({"Accept": accept} if accept else {})})
    return urllib.request.urlopen(request, timeout=60)


def installed_tag(prefix):
    try:
        return json.loads((prefix / MARKER).read_text()).get("tag")
    except (OSError, ValueError):
        return None


def write_atomically(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.new")
    temporary.write_text(text)
    os.replace(temporary, path)


def install(args, say=print):
    prefix = Path(args.prefix).absolute()
    releases = json.load(fetch(f"https://api.github.com/repos/{REPOSITORY}/releases?per_page=30", "application/vnd.github+json"))
    release = pick_release(releases, args.channel, args.version)
    tag = release["tag_name"]
    asset = pick_asset(release, platform.machine())
    current = installed_tag(prefix)
    if current == tag and not args.force:
        say(f"T3 Code {tag} is already installed in {prefix}.")
        return
    say(f"T3 Code {current or 'unknown version'} -> {tag} ({asset['name']}, {asset['size'] >> 20} MiB)")
    if args.dry_run:
        return
    sums_asset = next((item for item in release["assets"] if item["name"] == "SHA256SUMS"), None)
    sums = parse_sums(fetch(sums_asset["browser_download_url"]).read().decode()) if sums_asset else {}
    wanted = expected_digests(asset, sums)

    prefix.parent.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix=".t3code-install-", dir=prefix.parent))
    try:
        image = work / asset["name"]
        with fetch(asset["browser_download_url"]) as response, open(image, "wb") as out:
            shutil.copyfileobj(response, out)
        actual = sha256_of(image)
        if {actual} != wanted:
            raise InstallError(f"{asset['name']} has checksum {actual}, expected {', '.join(sorted(wanted))}")
        image.chmod(0o755)
        subprocess.run([str(image), "--appimage-extract"], cwd=work, check=True, stdout=subprocess.DEVNULL)
        tree = work / "squashfs-root"
        for needed in ("t3code", "AppRun", "resources/app.asar", "t3code.desktop"):
            if not (tree / needed).exists():
                raise InstallError(f"the extracted app has no {needed}")
        (tree / MARKER).write_text(json.dumps({"tag": tag, "asset": asset["name"], "sha256": actual}, indent=2) + "\n")
        old = work / "previous"
        if prefix.exists():
            os.rename(prefix, old)
        try:
            os.rename(tree, prefix)
        except OSError:
            if old.exists():
                os.rename(old, prefix)
            raise
    finally:
        shutil.rmtree(work, ignore_errors=True)

    applications = Path(args.applications_dir).absolute()
    write_atomically(applications / "t3code.desktop", render_desktop_entry((prefix / "t3code.desktop").read_text(), prefix))
    say(f"Installed T3 Code {tag} in {prefix}. A running T3 Code keeps its old files until it is restarted.")


def main(argv=None):
    data_home = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--channel", choices=CHANNELS, default="nightly", help="release channel to follow (default: nightly)")
    parser.add_argument("--version", help="install this exact release tag instead of the newest on the channel")
    parser.add_argument("--force", action="store_true", help="reinstall even if this release is installed")
    parser.add_argument("--dry-run", action="store_true", help="say what would be installed and change nothing")
    parser.add_argument("--prefix", default=str(Path.home() / ".local/opt/t3code"), help="where the app goes (default: ~/.local/opt/t3code)")
    parser.add_argument("--applications-dir", default=str(data_home / "applications"), help="where the desktop entry goes")
    args = parser.parse_args(argv)
    try:
        install(args)
    except (InstallError, OSError, subprocess.CalledProcessError, ValueError) as error:
        print(f"install-t3code: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
