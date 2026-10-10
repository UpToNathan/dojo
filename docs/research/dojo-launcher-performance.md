# Launcher opening latency

Measured on 2026-10-09 on the live Niri session. These are diagnosis samples,
not a Fuzzel comparison or a statistically robust release benchmark.

## Before graphics dependency optimization

- Three `dojo --launcher` invocations acknowledged the control command in
  7.62–7.90 ms. Three direct Unix-socket requests acknowledged in 0.21–0.29 ms.
- Observing `dojo-launcher` in `niri msg --json layers` took 18.03–32.94 ms.
  This is **not** a rendered or presented frame measurement: Wayland creates
  the layer-surface role before the renderer is ready.
- Four screenshot probes detected the panel's normal background color after
  408.76–461.92 ms. The region was checked against a pre-open baseline with zero
  matching panel-color fraction. Capture overhead and any presentation effects
  are included; this is not an exact first-presentation timestamp.
- Temporary instrumentation around `Launcher::toggle` measured **259.14 ms
  inside GPUI window creation**, before the method returned.

The pinned GPUI Wayland implementation creates a new `WgpuRenderer` in
`WaylandWindowState::new` for each window. Its WGPU renderer creates shader
modules and render pipelines during initialization. The global GPU context
is reused, but this is not equivalent to reusing the per-window renderer.
Sources:

- [Wayland window initialization](https://github.com/zed-industries/zed/blob/9ab0715969e9854f2efe61a9d782e7698e5fe6d2/crates/gpui_linux/src/linux/wayland/window.rs)
- [WGPU renderer initialization and create_pipelines](https://github.com/zed-industries/zed/blob/9ab0715969e9854f2efe61a9d782e7698e5fe6d2/crates/gpui_wgpu/src/wgpu_renderer.rs)
- Local `src/launcher.rs`, `src/control.rs`, and `src/shell.rs`.

The initial diagnosis therefore targets unoptimized development-build shader/
pipeline initialization, not catalogue discovery or matching. Development
profile overrides optimize `naga`, `wgpu-core`, and `gpui_wgpu` while leaving
Dojo's own development code debuggable. Improvement must be measured after
building; the profile change alone is not evidence of a fix.

## After graphics dependency optimization

With `naga`, `wgpu-core`, and `gpui_wgpu` optimized in the development profile,
four live `open_window` samples measured **78.12, 129.49, 156.93, and 124.80 ms**.
The same screenshot probe measured **251.47, 325.19, 329.08, and 288.97 ms** until
normal panel appearance. The CLI remained about 7.6–7.8 ms; direct IPC about
0.23–0.25 ms. Temporary timing instrumentation was removed after measurement.

This is an improvement, **not an instant launcher or evidence of Fuzzel parity**.
Fresh per-invocation renderer/window initialization remains expensive. Safely
reusing a prepared GPUI launcher surface requires show/hide (or unmap/remap) and
keyboard-interactivity lifecycle support; those are not currently exposed by
the pinned Wayland window interface. Do not emulate hiding with transparency
while retaining exclusive keyboard focus.

## Measurement limitations and next steps

- Measure opening separately from fuzzy matching (`--launcher-benchmark`).
- Capture time is an upper-bound observation, not precise presentation timing.
- Do not compare a warm resident Dojo invocation with cold Fuzzel without
  identifying their different process lifetimes.
- Renderer reuse or safe layer-shell unmap/remap would avoid more repeated work,
  but requires correct GPUI platform support and keyboard-focus release. Leaving
  an invisible exclusive-keyboard surface alive is not an acceptable shortcut.
- Virtual-keyboard shortcut injection did not reproduce the physical shortcut
  reliably in this session; those attempts are not a trustworthy keybinding
  latency measurement. Direct binary and Unix-socket tests drive the same host
  launcher-opening path without that uncertainty.
