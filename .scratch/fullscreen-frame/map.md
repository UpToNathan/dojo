# Fullscreen frame

## Notes

Fullscreen clients cover the desktop frame. See [the spec](spec.md).

## Decisions-so-far

- [Frame layer](issues/01-fullscreen-visibility.md): Top allows compositor fullscreen stacking to cover the surround. Real screenshot comparisons fail with Overlay and pass with Top; the frame returns on exit.

## Fog

The live checks use one output. Layer placement is per output; multi-output fullscreen still needs a dedicated scenario.
