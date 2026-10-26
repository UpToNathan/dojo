# Mesh panel: window, cards and health

Status: resolved

Blocked by: 02

`src/mesh_ui.rs`: the layer-shell panel in the monitor panel's pattern, opened by a top-bar button and `dojo-desktop --mesh`, refreshed while open. Cards, health badge, agent lock glyph. No actions yet.

## Acceptance

- Opens, closes on Esc or a click outside, reveals in 150 ms like the monitor panel.
- Reads the same data as `dojo-mesh status` and `doctor`.
- Does not scroll: it is sized for its content.
