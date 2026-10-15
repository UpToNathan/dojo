#!/usr/bin/env python3
"""Audit outputs from the preset-export-audit Rust example without applying them."""
import json
import plistlib
import sys
import tomllib
from pathlib import Path


def contrast(foreground, background):
    def luminance(color):
        channels = [int(color[index:index + 2], 16) / 255 for index in (1, 3, 5)]
        linear = [v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in channels]
        return sum(v * w for v, w in zip(linear, (0.2126, 0.7152, 0.0722)))
    low, high = sorted((luminance(foreground), luminance(background)))
    return (high + 0.05) / (low + 0.05)


def audit(root):
    findings = []
    presets = []
    for directory in sorted(root.iterdir()):
        if not directory.is_dir():
            continue
        outputs = {}
        for file in directory.iterdir():
            if file.suffix == '.json':
                outputs[file.name] = json.loads(file.read_text())
            elif file.suffix == '.toml':
                outputs[file.name] = tomllib.loads(file.read_text())
            elif file.suffix == '.xml':
                plistlib.loads(file.read_bytes())
        palette = outputs['palette.json']
        roles = palette['colors']
        presets.append({'id': directory.name, 'appearance': palette['appearance'], 'authored': palette['authored'], 'files': len(list(directory.iterdir()))})

        def pair(target, state, foreground, background, minimum=None):
            if minimum is None:
                minimum = 3.0 if palette['authored'] else 4.5
            ratio = contrast(foreground, background)
            findings.append({'preset': directory.name, 'target': target, 'state': state, 'foreground': foreground, 'background': background, 'contrast': round(ratio, 3), 'minimum': minimum, 'pass': ratio + 0.001 >= minimum})

        for target, values in [('dojo', outputs['dojo.json']), ('nvim', roles)]:
            for foreground, background in [('foreground', 'background'), ('foreground', 'surface'), ('selection_foreground', 'background_highlight'), ('primary_foreground', 'primary')]:
                pair(target, background, values[foreground], values[background])
        t3 = outputs['t3code.json']['colors']
        for state in ['sidebarRowHover', 'sidebarRowActive', 'sidebarRowSelected']:
            pair('t3code', state, t3['sidebarForeground'], t3[state])
            pair('t3code', state + '-muted', t3['sidebarMutedForeground'], t3[state], 3.0)
        for foreground, background in [('toolbarControlForeground', 'toolbarControlHover'), ('terminalForeground', 'terminalSelection'), ('accentSurfaceForeground', 'accentSurface'), ('searchMatchForeground', 'searchMatchBackground'), ('messageActionForeground', 'messageActionHover')]:
            pair('t3code', background, t3[foreground], t3[background])
        pi = outputs['pi.json']['colors']
        pair('pi', 'selected', pi['text'], pi['selectedBg'])
        opencode = outputs['opencode.json'][palette['appearance']]
        pair('opencode', 'hovered', opencode['text']['action']['primary']['base'], opencode['background']['action']['primary']['$hovered'])
        pair('opencode', 'raised-max', opencode['text']['base'], opencode['background']['raised']['max'])
        pair('opencode', 'dialog-hovered', opencode['text']['action']['primary']['base'], opencode['@dialog']['background']['action']['primary']['$hovered'])
        for section, values in outputs['yazi.toml'].items():
            if isinstance(values, dict):
                for name, style in values.items():
                    if isinstance(style, dict) and 'fg' in style and 'bg' in style and not name.startswith('marker_'):
                        minimum = 3.0 if name in ['progress_normal', 'progress_error'] else None
                        pair('yazi', section + '.' + name, style['fg'], style['bg'], minimum)
        zen = outputs['zen.json']['colors']
        for prefix in ['popup_highlight', 'sidebar_highlight']:
            pair('zen', prefix, zen[prefix + '_text'], zen[prefix])
        pair('zen', 'button-hover', zen['icons'], zen['button_background_hover'])
        for name, separator in [('kitty.conf', ' '), ('ghostty.conf', ' = ')]:
            values = dict(line.split(separator, 1) for line in (directory / name).read_text().splitlines() if separator in line and not line.startswith('#'))
            key = 'selection_' if name == 'kitty.conf' else 'selection-'
            pair(name.split('.')[0], 'selection', values[key + 'foreground'], values[key + 'background'])
        for name in ['qt5.conf', 'qt6.conf']:
            values = next(line.split('=', 1)[1].split(', ') for line in (directory / name).read_text().splitlines() if line.startswith('active_colors='))
            pair('qt', name + '-highlight', values[13], values[12])
        helium = outputs['helium.json']['theme']['colors']
        rgb = lambda channels: '#' + ''.join(f'{channel:02x}' for channel in channels)
        pair('helium', 'toolbar', rgb(helium['tab_text']), rgb(helium['toolbar']))
        for name in ['gtk-3.0.css', 'gtk-4.0.css']:
            definitions = dict((line.split()[1], line.split()[2].rstrip(';')) for line in (directory / name).read_text().splitlines() if line.startswith('@define-color'))
            pair('gtk', name + '-selected', definitions['theme_selected_fg_color'], definitions['theme_selected_bg_color'])
        assert roles['background'] in (directory / 'tmtheme.xml').read_text()
        assert roles['primary'] in (directory / 'niri.kdl').read_text()
    return {'presets': presets, 'checks': findings, 'failures': [row for row in findings if not row['pass']]}


if __name__ == '__main__':
    report = audit(Path(sys.argv[1]))
    Path(sys.argv[2]).write_text(json.dumps(report, indent=2) + '\n')
    print(f"{len(report['presets'])} presets; {len(report['checks'])} color pairs; {len(report['failures'])} pairs below their contrast floor")
    counts = {}
    for row in report['failures']:
        counts[row['target']] = counts.get(row['target'], 0) + 1
    print(json.dumps(counts, sort_keys=True))
    raise SystemExit(bool(report['failures']))
