# Audit preset mappings and fix interaction color pairs

Status: resolved
Type: task

Audit all 41 presets across application exporters after pale dark-preset selections
made T3 sidebar labels unreadable. Preserve preset palette values and wallpaper.

## Comments

The installed T3 client uses sidebarForeground on selected and hovered rows; its
schema does not expose a separate selected-row foreground. Mapping a terminal
selection fill to those rows can make text disappear. OpenCode, Pi, Zed, and Zen
button hovers have the same constraint. Their exporters now choose an existing
surface compatible with normal and muted text. Yazi supports foreground/background
pairs and now receives selection_foreground. Status-filled labels use readable
foregrounds; progress bars use the canvas rather than the terminal selection fill.

The audit exports all preset variants without applying them and parses their
structured output. It checks the actual exported text/fill pairs, using 4.5:1 for
adapted text, the existing 3:1 authored floor, and 3:1 for muted text and indicators.
Shell Wi-Fi, Bluetooth, display controls, and the power menu were also inspected
for the same mismatch. Generic containers now use surfaces; selected controls use
the paired foreground.

Final audit: all 41 presets, 19 formats per preset, and 2,378 inspected pairs.
428 pairs failed their applicable floors before the exporter changes; none fail
after them. All 41 palette.json exports are byte-identical before and after.
The 29 theme tests, the shell preset roundtrip test, and seven final exporter
tests passed. The release desktop and theme command were built and deployed.

Reapplied Kanagawa Dragon with its existing wallpaper. The actual T3 sidebar
and power menu were visually inspected with readable selection labels. Evidence:
`/tmp/dojo-theme-t3-after.png`, `/tmp/dojo-preset-power-after.png`, and the before/
after JSON reports under `/tmp/dojo-preset-audit-*.json`. Current publication
reports Pi needs selection, Zen needs setup, and Delta is unsupported. Other
targets publish or report ready/reload requested; they are not all visually verified.
