# niri-ipc

Types and helpers for interfacing with the [niri](https://github.com/niri-wm/niri) Wayland compositor.

## Backwards compatibility

This crate follows the niri version.
It is **not** API-stable in terms of the Rust semver.
In particular, expect new struct fields and enum variants to be added in patch version bumps.

Use an exact version requirement to avoid breaking changes:

```toml
[dependencies]
niri-ipc = "=26.4.0"
```

## Render-correlated captures in Dojo

Dojo's vendored compositor accepts `{"CaptureScene":{"output":"winit"}}` over
its existing IPC socket. `CapturedScene` returns an upright PNG as base64 and
window rectangles from the same compositor state and render. Rectangles use
output-relative logical coordinates; multiply by `logical_output.scale` to
locate pixels. The output transform is already applied to the image. The
response also carries its output descriptor, compositor PID and frame serial.
Clients must bind these observations to their own session and compositor
process generation before permitting effects. The serial grants no input
permission.

Both tiled and floating geometry use rendering's rounded tile/workspace
positions and shared window-geometry calculation. Captures refuse overview,
layout animations, workspace transitions, interactive moves, screen
transitions, locked sessions and the screenshot UI. `output_clip` describes
output clipping; other windows, popups and shell layers may still occlude it.
`blocked_out` records the compositor's screen-capture privacy rule.

Each capture is limited to 8,388,608 pixels, 32 MiB of PNG data and 4,096
window placements. Two global permits bound queued readbacks, encoder workers
and undelivered replies. PNG encoding runs outside the compositor callback;
GPU readback remains on its renderer thread. A client that stops receiving a
capture reply is disconnected after ten seconds. No render or capture work
runs when no client requests it.

Run the Rust renderer acceptance check inside Dojo's devenv after building the
compositor and `tools/agent-input` fixture:

```sh
cargo run --manifest-path vendor/niri/Cargo.toml --no-default-features \
  --example check_scene_capture
```

It launches and cleans up its own nested session, checks fixture pixel colors
against returned floating/tiled geometry, checks fractional scale and Winit's flipped
orientation, checks unchanged focus and privacy replacement, and retains evidence
under `/tmp/dojo-scene-capture-*`.
