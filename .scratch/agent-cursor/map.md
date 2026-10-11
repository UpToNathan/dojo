# Nerd Font agent cursor

## Notes

Replace the drawn arrow with `nf-fa-mouse_pointer`. See [the spec](spec.md).

## Decisions-so-far

- [Font icon](issues/01-font-icon.md): embed a high resolution glyph asset from the bundled font, preserving its license and exporter. This keeps the fork independent of host fonts. Tests and rendered screenshots at 1.75 scale pass, including idle cursor cleanup.

## Fog

The installed cursor replacement needs the next Niri login. Smooth movement is separate work.
