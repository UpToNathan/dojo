"""Build a coherent dated pacman repository configuration."""

import re


def archive_config(text, date):
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
                output.append(f"Server = https://archive.archlinux.org/repos/{date}/$repo/os/$arch")
        elif active and re.match(r"(?:Include|Server|CacheServer)\s*=", active):
            if section == "options":
                raise ValueError("included pacman options require manual review for dated snapshots")
            if section is None:
                raise ValueError("pacman mirror directive outside a repository")
        else:
            output.append(line)
    if not any(line.startswith("Server = https://archive.archlinux.org/") for line in output):
        raise ValueError("no official Arch repositories configured")
    return "\n".join(output) + "\n"

