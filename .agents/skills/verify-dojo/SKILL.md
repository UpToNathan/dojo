---
name: verify-dojo
description: Verify Dojo's GPUI Wayland desktop frame and Niri layout integration in a live Niri session; use after changes to the frame, input pass-through, output handling, or column resizing.
---

# Verify Dojo

## Launch

Dojo needs the shared devenv environment for its Fontconfig, EGL, Vulkan, Mesa,
Wayland, and xkbcommon runtime libraries. Do not launch `target/debug/dojo`
outside that environment.

1. Confirm that Niri is running and note active outputs with `niri msg outputs`.
2. Confirm there is no existing Dojo process or `dojo-frame-*` layer; never
   start two instances because each instance changes every tiled column width.
3. Choose an output name from `niri msg outputs`, then create an evidence folder:

   ```sh
   RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
   EVIDENCE="/tmp/opencode/dojo-verification/$RUN_ID"
   mkdir -p "$EVIDENCE"
   ```

4. Save the current tiled-column widths before starting Dojo:

   ```sh
   cd /home/nathan/Dojo
   devenv shell -- /home/nathan/Dojo/.agents/skills/verify-dojo/scripts/layout-widths.sh \
     snapshot "$EVIDENCE/window-widths.json"
   ```

5. Capture the pre-run screen, replacing `HDMI-A-1` with the active output:

   ```sh
   grim -o HDMI-A-1 "$EVIDENCE/before.png"
   ```

6. Start Dojo in its own process group:

   ```sh
   .agents/skills/verify-dojo/scripts/start.sh "$EVIDENCE"
   ```

The command records the process-group ID and log under `$EVIDENCE`. The
`dojo-frame-*` top-layer frame and four `dojo-reservation-*` surfaces in
`niri msg layers` indicate that the layer-shell surfaces have mapped. Dojo's
CLI toggle is `cargo run --manifest-path /home/nathan/Dojo/Cargo.toml --
--toggle` inside devenv; no Niri keybinding is configured yet.

## Doctor

Before driving, confirm all of the following:

- `niri msg outputs` shows the target output as active.
- `pgrep -a -x dojo` reports exactly one process.
- `niri msg layers` shows one `dojo-frame-*` top-layer frame and four
  `dojo-reservation-*` top-layer surfaces for each active display.
- `layout-widths.sh check "$EVIDENCE/window-widths.json"` inside devenv reports
  every saved tiled column 76 logical pixels narrower.
- The foreground terminal has no layer-shell or renderer error.

If any check fails, do not call the frame verified. Capture the terminal output
and Niri state, then clean up before another attempt.

## Drive

See the feature map first:

- [Desktop surround](features/desktop-surround.md)
- [Niri scrolling layout](features/niri-scrolling-layout.md)

After frame-layer changes, run the isolated fullscreen regression inside devenv:

```sh
devenv shell -- tools/agent-input/run-nested.sh --fullscreen
```

It creates a separate nested compositor and an isolated Dojo control socket and
state directory. It requires all eight fullscreen edge crops to match the
frameless screenshot, then requires the frame to return after exiting
fullscreen. The nested shell changes only its own compositor's layout; the
host shell can remain running. Retain the printed evidence directory.

Use actual user-visible output, not internal setters, as evidence. With Dojo
running, capture the result:

```sh
grim -o HDMI-A-1 "$EVIDENCE/frame.png"
niri msg layers > "$EVIDENCE/layers.txt"
niri msg --json windows > "$EVIDENCE/windows.json"
cd /home/nathan/Dojo
devenv shell -- /home/nathan/Dojo/.agents/skills/verify-dojo/scripts/layout-widths.sh \
  check "$EVIDENCE/window-widths.json" > "$EVIDENCE/width-check.txt"
```

Visually inspect `frame.png` for the dark outside surround, rounded transparent
opening, and absence of the removed gray stroke. Compare `before.png` with
`frame.png`; verify the workspace remains horizontally scrollable and the tiled
columns fit within the reserved viewport. To verify pointer pass-through, click
inside the transparent opening and confirm the underlying window receives
focus. If no human pointer is available, report that check as unverified rather
than inferring it from the empty input region.

## Evidence

Keep the complete evidence folder under `/tmp/opencode/dojo-verification/`:
it contains the before/after screenshots, Niri layer/window state, saved widths,
and resize check. Screenshots alone do not prove Niri reservations or scrolling;
retain the corresponding IPC output and the launch terminal log as well.

## Cleanup

1. Stop only the process group created by `scripts/start.sh`, then restore the
   recorded tiled-column widths:

   ```sh
   .agents/skills/verify-dojo/scripts/stop.sh "$EVIDENCE"
   ```

2. Confirm `pgrep -a -x dojo` is empty, `niri msg layers` has no Dojo
   namespaces, and `niri msg windows` reports the original widths.
3. Keep the evidence folder; cleanup must not remove proof artifacts.

The restore helper targets only the recorded process group, never a process
name, and restores widths even when the app was terminated before its normal
shutdown path ran.
