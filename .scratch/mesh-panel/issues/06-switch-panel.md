# Switch panel redesign

Status: open

The panel from the spec: machine tiles, a window/control toggle in place of the checkbox, no key-hint line, one short status line, and the control-only mode visibly changing the panel (windows collapse, tiles and a keyboard glyph grow, warmer accent, "Take control").

## Acceptance

- `I`, the toggle and a click set the same mode; the transition runs in the reveal time and respects reduced motion.
- Behaviour after Enter is unchanged and its tests still pass.
- The model (`src/send_panel.rs`) keeps the mode as state with tests; rendering reads it.
