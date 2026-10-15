# Preset export audit

The 41 presets share application exporters, but application theme schemas do not
all represent selection in the same way. A terminal selection fill can be pale
even in a dark preset. Using that fill with normal pale text makes the row unreadable.

| Consumer | Interaction mapping |
| --- | --- |
| Dojo | Selection fills use selection_foreground; generic panels use surfaces |
| Kitty, Ghostty | Preserve the terminal selection fill and its foreground |
| Neovim | Visual and PmenuSel use the selection pair; supported families use native themes |
| T3 Code | Sidebar and toolbar states retain normal text, so choose a compatible existing surface |
| OpenCode | Hover and highest raised surfaces retain normal text, so choose a compatible surface |
| Pi | selectedBg retains text and muted colors, so choose a compatible surface |
| Yazi | Selection states use selection_foreground; status fills have paired labels |
| Zen | Popup/sidebar selection has a pair; button hovers retain icon text |
| Helium | Background and toolbar text map directly to canvas and foreground |
| GTK | Selected and accent fills use primary_foreground |
| Qt | Highlight and HighlightedText use the primary pair |
| Niri | Focus ring and agent cursor use primary; borders retain their opacity |

Run the audit from the repository inside devenv:

```sh
devenv shell -- cargo run --example preset-export-audit -- /tmp/dojo-preset-exports
python3 scripts/audit-preset-exports.py /tmp/dojo-preset-exports /tmp/dojo-preset-report.json
```

The JSON report records each preset, output count, inspected pair, actual colors,
contrast, and applicable floor. The script fails when a checked pair falls below
its floor. Adapted text uses 4.5:1; authored pairs retain the existing 3:1 acceptance
floor. Muted text and graphical indicators use 3:1. These checks cover exported
mapping values and format parsing, rather than every application pixel or syntax
token. Authored low-contrast comments and terminal ANSI colors remain authored.

The October 2026 audit exported 19 formats for each of 41 presets and checked
2,378 pairs. It found 428 failures before the mapping corrections and zero after
them. All exported palette.json files remained byte-identical. Regression tests
cover Kanagawa Dragon's actual T3 and Yazi values and unpaired selection fills
across every preset. T3's sidebar and Dojo's power menu were verified live.

Publication and reload are separate from visual verification. Inspect
`dojo-theme status` for each application's result. Pi needs Dojo selected once;
Zen needs its browser bridge. An application marked
ready may need to be opened before its live behavior can be checked.

Zed and Delta have been removed from the installed desktop and active exporters.
The earlier 19-format audit includes their now-retired exports; current audits
produce 17 formats per preset. Stored selections discard the retired targets
when loaded, so switching or rolling back an old theme does not restore them.
The reduced exporter sweep passes all 2,337 checked pairs across the 41 presets.
