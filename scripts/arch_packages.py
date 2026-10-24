"""Build a coherent dated pacman repository configuration."""

import re


# Omarchy's stable channel is one delayed snapshot of the official repositories (about two days
# behind current when measured, with no published guarantee). The URL is pinned here so this
# checkout needs no Omarchy installation. It carries no dated history, so it has no exact age.
OMARCHY_STABLE = "https://stable-mirror.omarchy.org/$repo/os/$arch"


def archive_config(text, date):
    """Use one dated Arch Linux Archive snapshot for every official repository."""
    return snapshot_config(text, f"https://archive.archlinux.org/repos/{date}/$repo/os/$arch")


def omarchy_stable_config(text):
    """Use Omarchy's stable snapshot for every official repository."""
    return snapshot_config(text, OMARCHY_STABLE)


def snapshot_config(text, server):
    """Keep pacman's options and repo order, replacing official repo mirrors."""
    official = {"core", "extra", "multilib", "core-testing", "extra-testing", "multilib-testing",
                "core-staging", "extra-staging", "multilib-staging"}
    section = None
    output = []
    for line in text.splitlines():
        active = line.split("#", 1)[0].strip()
        header = re.fullmatch(r"\[([^]]+)\]", active)
        if header:
            section = header[1]
            if section != "options" and section not in official:
                raise ValueError(f"repository {section!r} has no supported dated snapshot")
            output.append(line)
            if section != "options":
                output.append(f"Server = {server}")
        elif active and re.match(r"(?:Include|Server|CacheServer)\s*=", active):
            if section == "options":
                raise ValueError("included pacman options require manual review for dated snapshots")
            if section is None:
                raise ValueError("pacman mirror directive outside a repository")
        else:
            output.append(line)
    if f"Server = {server}" not in output:
        raise ValueError("no official Arch repositories configured")
    return "\n".join(output) + "\n"

