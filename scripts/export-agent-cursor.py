#!/usr/bin/env python3
"""Export nf-fa-mouse_pointer from Dojo's bundled Nerd Font. Requires Pillow and fonttools."""
import argparse
from pathlib import Path

from PIL import Image, ImageFont
from fontTools.ttLib import TTFont


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--font", type=Path, default=root / "assets/fonts/VictorMonoNerdFontMono-Regular.ttf")
    parser.add_argument("--output", type=Path, default=root / "vendor/niri/resources/agent-pointer.png")
    args = parser.parse_args()
    with TTFont(args.font) as source:
        if 0xF245 not in source.getBestCmap():
            raise ValueError("The font must contain nf-fa-mouse_pointer (U+F245)")
    font = ImageFont.truetype(str(args.font), 192)
    glyph = font.getmask("\uf245", mode="L")
    mask = Image.new("L", glyph.size)
    mask.putdata(glyph)
    bounds = mask.getbbox()
    if bounds is None:
        raise ValueError("The font must contain nf-fa-mouse_pointer (U+F245)")
    mask = mask.crop(bounds)
    height = 96
    width = round(mask.width * height / mask.height)
    mask = mask.resize((width, height), Image.Resampling.LANCZOS)
    pointer = Image.new("RGBA", mask.size, (40, 200, 240, 255))
    pointer.putalpha(mask)
    pointer.save(args.output)
    print(f"Exported nf-fa-mouse_pointer: {width}x{height}, 24 logical pixels at buffer scale 4")


if __name__ == "__main__":
    main()
