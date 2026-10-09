# Niri scrolling layout

Dojo reserves a 64 px left panel, a 24 px top bar, and 12 px on the right and
bottom of each active output. It narrows each existing tiled column by the
combined 76 px horizontal reservation. It keeps workspace and column order
intact so Niri's horizontal scrolling remains available.

## Sub-features

- `edge-reservations` adds one top-layer exclusive-zone surface per output edge.
- `existing-columns` reduces one Niri window per workspace/column by 76 px.
- `scroll-order` preserves each window's workspace and scrolling column index.
- `restore-widths` restores the pre-run pixel widths after a normal exit or via
  the verification cleanup helper.

## How to get to it (user POV)

- Start Dojo in Niri; reservations apply to every active output.
- Existing tiled columns shrink while their order and off-screen scroll behavior
  remain available.

## Driving it with Niri IPC

Preconditions:

- Save the tiled widths before launch with `scripts/layout-widths.sh snapshot` inside devenv.
- Record `niri msg --json windows` before launch.

- **Reservation.** Start Dojo and run `niri msg layers`. Each output has four
  Dojo reservations; the visual frame is a separate overlay surface.
- **Width change.** Run `scripts/layout-widths.sh check WIDTHS.json` inside devenv. Every
  saved tiled column is 76 logical pixels narrower.
- **Scroll behavior.** Compare `pos_in_scrolling_layout` from `niri msg --json
  windows` before and after. Workspace and column indices remain unchanged.
  Focus a column and scroll horizontally; off-screen columns remain reachable.
- **Restore.** Stop Dojo in its launch terminal, then run
  `scripts/layout-widths.sh restore WIDTHS.json` inside devenv. Niri reports the saved widths.

## Gotchas

- Niri deliberately preserves existing column sizes when new windows open;
  exclusive zones alone do not resize existing columns.
- Dojo makes existing pixel widths fixed while enabled. The process snapshots
  the visible widths for restoration; always run the cleanup restore after a
  forced termination.
- Resize is applied once at startup. Newly opened windows use the reserved
  viewport; they are not part of the startup snapshot.
- Do not start two Dojo processes: each instance would apply another 76 px
  reduction to the same columns.
