#!/usr/bin/env python3
"""termfilechooser 1.4.3 wrapper: multiple directory save path out debug.

One Kitty process per request; Q/window close cancels, q accepts a directory,
s accepts a save name (relative to the browsed directory). Never create a
save destination. The calling application owns overwrite policy; the portal
owns the output file.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def main(args):
    if len(args) != 6 or any(flag not in ("0", "1") for flag in args[:3]):
        raise ValueError("expected: multiple directory save path out debug")
    multiple, directory, save = (flag == "1" for flag in args[:3])
    suggested, output, _debug = args[3:]
    out = Path(output)
    out.write_text("")  # stale results must never survive cancellation
    with tempfile.TemporaryDirectory(prefix="dojo-yazi-chooser-") as tmp:
        root = Path(tmp)
        config = root / "config"
        config.mkdir()
        original = Path(os.environ.get("YAZI_CONFIG_HOME", str(Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))) / "yazi")))
        # Keep previews, theme and user settings, but chooser keys are isolated.
        if original.is_dir():
            for item in original.iterdir():
                if item.name not in ("keymap.toml", "plugins"):
                    (config / item.name).symlink_to(item.resolve())
        plugins = config / "plugins"
        plugins.mkdir()
        if (original / "plugins").is_dir():
            for item in (original / "plugins").iterdir():
                if item.name != "dojo-chooser.yazi":
                    (plugins / item.name).symlink_to(item.resolve())
        plugin = plugins / "dojo-chooser.yazi"
        plugin.mkdir()
        shutil.copyfile(os.environ["DOJO_YAZI_CHOOSER_PLUGIN"], plugin / "main.lua")
        action = "plugin dojo-chooser" if directory else "app:quit --no-cwd-file"
        (config / "keymap.toml").write_text(
            '[mgr]\nprepend_keymap = [\n'
            f'{{ on = "q", run = "{action}", desc = "Accept directory / cancel" }},\n'
            '{ on = "Q", run = "app:quit --no-cwd-file", desc = "Cancel chooser" },\n'
            + ('{ on = "s", run = "plugin dojo-chooser", desc = "Save as (new filename allowed)" },\n' if save else '')
            + ']\n'
        )
        result = root / "selection"
        start = Path(suggested).expanduser()
        name = start.name if save and not start.is_dir() else ""
        if save and not start.is_dir():
            start = start.parent
        while not start.exists() and start != start.parent:
            start = start.parent
        env = dict(os.environ, YAZI_CONFIG_HOME=str(config),
                   DOJO_CHOOSER_OUTPUT=str(result), DOJO_CHOOSER_SAVE="1" if save else "0",
                   DOJO_CHOOSER_NAME=name)
        status = subprocess.run([
            os.environ["DOJO_KITTY"], "--title", "Yazi chooser — Q cancel; q directory; s save as",
            os.environ["DOJO_YAZI"], f"--chooser-file={result}", str(start),
        ], env=env, check=False).returncode
        if status != 0 or not result.exists():
            return 0
        paths = result.read_text().splitlines()
        # The backend API is newline-delimited: reject relative/invalid results.
        if not paths or any(not p or not Path(p).is_absolute() for p in paths):
            return 0
        if directory:
            paths = [p for p in paths if Path(p).is_dir()]
        elif save:
            paths = [p for p in paths if not Path(p).is_dir() and Path(p).parent.is_dir()]
        else:
            paths = [p for p in paths if Path(p).is_file()]
        if not multiple or save:
            paths = paths[:1]
        if paths:
            out.write_text("\n".join(paths) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
