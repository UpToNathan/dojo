# Desktop surround

Dojo draws a dark surround around a rounded transparent opening so the desktop
appears framed, while leaving the tiled application area visible through the
center.

## Sub-features

- `surround-fill` paints a 64 px left panel, 24 px top bar, and 12 px right and
  bottom bands.
- `rounded-opening` leaves the inner desktop visible with 28 px rounded corners.
- `frame-layer` keeps the full-output visual surface out of Niri's positive
  exclusive zones.
- `pointer-pass-through` leaves the frame and reservation surfaces with empty
  input regions.

## How to get to it (user POV)

- Start Dojo from the configured devenv environment while logged into Niri.
- The frame appears on each active output. `cargo run -- --toggle` toggles it; no Niri keybinding is configured yet.

## Driving it with GPUI and Niri

Preconditions:

- Follow the `Launch` and `Doctor` sections in the parent `SKILL.md`.
- The target output name is available from `niri msg outputs`.

- **Start.** Run the documented foreground `devenv shell -- cargo run` command.
  The launch terminal remains attached and reports any renderer or layer-shell error.
- **Layer mapping.** Run `niri msg layers`. Each active output lists one
  `dojo-frame-*` overlay and four `dojo-reservation-*` top-layer surfaces.
- **Visual result.** Run `grim -o OUTPUT "$EVIDENCE/frame.png"`. The screenshot
  shows a dark outside surround, a transparent rounded desktop opening, and no
  gray outline.
- **Input pass-through.** Click inside the transparent opening. The underlying
  window receives focus; an IPC report alone is not proof of pointer delivery.

## Gotchas

- The frame uses the devenv-provided Vulkan/EGL libraries; running the binary
  outside devenv can fail to create a render surface.
- A full-screen frame with a positive exclusive zone can be constrained by the
  reservations; it must keep `exclusive_zone = -1`.
- Transparent pixels show the compositor contents underneath; they do not dim
  or blur the desktop.
- Niri layer rules and output hotplug can change where surfaces appear. Check
  each active output instead of assuming the primary display.
