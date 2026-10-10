#!/usr/bin/env python3
"""Inspect the actual light colors in the standalone design study."""

import json
import re
from pathlib import Path


def parse_object(value):
    value = re.sub(r"(?<=[{,])([A-Za-z][A-Za-z0-9_]*):", r'"\1":', value)
    return json.loads(value.replace("'", '"'))


def luminance(color):
    channels = [int(color[i:i + 2], 16) / 255 for i in (1, 3, 5)]
    channels = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4
                for v in channels]
    return sum(v * weight for v, weight in zip(channels, (0.2126, 0.7152, 0.0722)))


def contrast(a, b):
    values = sorted((luminance(a), luminance(b)))
    return (values[1] + 0.05) / (values[0] + 0.05)


source = (Path(__file__).resolve().parents[1] / 'docs/design/theme-study.html').read_text()
palettes = parse_object(re.search(r'const palettes=(.*?);', source).group(1))
generated = parse_object(re.search(r'const generatedPalettes=(.*?);', source).group(1))
recipe = re.search(r"Object.assign\(p,mode==='dark'\?(\{.*?\}):(\{.*?\})\);", source)
for name, overrides in [('static', {}), ('generated', parse_object(recipe.group(2)))]:
    palette = (palettes if name == 'static' else generated)['light'] | overrides
    print(name)
    for role in ('canvas', 'panel', 'hover', 'selected'):
        print(f'  {role:8} {palette[role]} luminance={luminance(palette[role]):.3f}')
    for a, b in [('text', 'panel'), ('muted-text', 'panel'), ('panel', 'hover'), ('hover', 'selected')]:
        print(f'  {a}/{b}: {contrast(palette[a], palette[b]):.2f}:1')
