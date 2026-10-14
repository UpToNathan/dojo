#!/usr/bin/env python3
"""Check selected wallpaper stacking and rendered frame colors on every output."""
import argparse
import json
import os
from pathlib import Path
import subprocess


def query(kind):
    return json.loads(subprocess.check_output(["niri", "msg", "--json", kind]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    outputs, layers = query("outputs"), query("layers")
    (args.evidence / "outputs.json").write_text(json.dumps(outputs, indent=2))
    (args.evidence / "layers.json").write_text(json.dumps(layers, indent=2))
    state = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state")))
    current = state / "dojo/themes/current"
    palette = json.loads((current / "dojo.json").read_text())
    assert (current / "wallpaper.txt").is_file(), "Select a wallpaper before this check"
    color = [int(palette["background"][i:i + 2], 16) for i in (1, 3, 5)]
    pixel_diff = Path(__file__).resolve().parents[1] / "tools/agent-input/target/debug/pixel-diff"
    ranks = {"Background": 0, "Bottom": 1, "Top": 2, "Overlay": 3}
    failures = []
    for name, output in outputs.items():
        if output["logical"] is None:
            continue
        image = args.evidence / f"{name}.png"
        subprocess.run(["grim", "-o", name, str(image)], check=True, timeout=5)
        surfaces = [surface for surface in layers if surface["output"] == name]
        wallpaper = next((surface for surface in surfaces
                          if surface["namespace"].startswith("dojo-wallpaper-")), None)
        if wallpaper is None:
            failures.append(f"{name}: selected wallpaper is missing")
        else:
            for fallback in (surface for surface in surfaces if surface["namespace"] == "btrbg"):
                selected_rank, fallback_rank = ranks[wallpaper["layer"]], ranks[fallback["layer"]]
                if selected_rank < fallback_rank or (selected_rank == fallback_rank
                        and surfaces.index(wallpaper) < surfaces.index(fallback)):
                    failures.append(f"{name}: fallback covers selected wallpaper")
        scale = output["logical"]["scale"]
        radius = max(1, round(3 * scale))
        result = json.loads(subprocess.check_output([
            str(pixel_diff), "--color", str(image), str(round(100 * scale)),
            str(round(8 * scale)), str(radius), *map(str, color),
        ]))
        if result["matching_pixels"] != (2 * radius) ** 2:
            failures.append(f"{name}: rendered frame differs from selected background {palette['background']}")
        print(json.dumps({"output": name, "frame_color": palette["background"],
                          "wallpaper_layer": wallpaper["layer"] if wallpaper else None,
                          "image": str(image)}), flush=True)
    assert not failures, "; ".join(failures)


if __name__ == "__main__":
    main()
