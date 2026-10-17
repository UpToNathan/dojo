# Dojo verification map

This map covers the user-visible desktop frame and its Niri scrolling-layout
integration. Read the relevant feature recipe before driving Dojo.

## Baseline preconditions

- Run inside the user's Niri session; layer-shell verification is compositor-specific.
- Use `~/Dojo` as the devenv root for builds and runs.
- Capture tiled widths before launch with `scripts/layout-widths.sh snapshot` inside devenv.
- Run a single Dojo instance. Multiple instances each resize all tiled columns.
- `cargo run -- --toggle` toggles the frame while Dojo is running; no Niri keybinding is configured yet.

## Driving conventions

- Choose an active output from `niri msg outputs`; verify surfaces on every active output.
- Use `niri msg layers` and `niri msg --json windows` as read-only compositor evidence.
- Check visual output with `grim`; use the actual pointer to verify click-through.
- Restore captured widths after stopping Dojo, even if its process was terminated.

## Proof standards

- Compare before and after screenshots from the same output.
- Verify all four reservation surfaces and the full-output top-layer frame in Niri IPC.
- Verify each saved tiled column is reduced by 76 logical pixels without changing
  its workspace or scrolling column index.
- Mark pointer pass-through unverified if it was not exercised by a real pointer click.
- Keep proof under `/tmp/opencode/dojo-verification/`; do not remove it during cleanup.

## Features

- [Desktop surround](./desktop-surround.md) covers the filled outer surround,
  rounded transparent opening, and pointer pass-through.
- [Niri scrolling layout](./niri-scrolling-layout.md) covers edge reservations,
  per-column resizing, and preserving horizontal scroll order.
