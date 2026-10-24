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

## Window reuse: shared pipelines and a hidden launcher surface

Measured on 2026-10-23 in a nested Niri (`tools/agent-input/run-nested.sh
--launcher-open`), on the debug profile with `naga`, `wgpu-core` and `gpui_wgpu`
optimized. This is **not** the live session or a release build. The absolute
numbers differ from the live samples above (the same code measured 23–25 ms in
`open_window` here, against 78–157 ms live), so compare only the interleaved
before/after pairs below. The machine's frequency governor is `powersave`, and a
second set of runs later the same day was about 2.3 times slower across the board
(baseline first present 80–89 ms instead of 35–39 ms) while keeping the same ratio.

Temporary timing lines (`eprintln!` around `Window::new`, `WgpuRenderer::new`,
`WgpuRendererCore::new`, `create_pipelines`, the first `queue.present` and
`Launcher::toggle`) were removed after measuring. The committed probe reports their
output when a build prints them and always reports the screenshot columns.

### Where `open_window` went

Median of seven openings in the baseline (a window created for each opening):

| Step | ms |
| --- | --- |
| `Launcher::toggle`'s `open_window` | 23.3–25.3 |
| of which `WgpuRendererCore::new` | 15.4–16.1 (bind group layouts 0.01) |
| of which surface and context | 2.2 |
| first draw of the scene | 1.3 |
| command to first presented frame | 35.4–38.8 |

Creating the render pipelines is about 65 % of `open_window` and 43 % of the time to
the first frame. The first launcher opening of a process costs more: about 35 ms in
`open_window` and 43–57 ms to the first frame.

### What changed

- `gpui_wgpu` keeps the bind group layouts and render pipelines on the `WgpuContext`,
  keyed by target format, alpha mode, path sample count, dual-source blending and the
  WebGL flag. Every window on a device shares them, and a device recovery replaces the
  context and so drops them. `open_window` falls to 9.4–11.2 ms and the first frame
  to 23–29 ms for a window that must still be created (any other panel).
- The launcher dismisses to an unmapped layer surface and maps it again for the next
  opening (`PlatformWindow::hide` and `show`; see
  `vendor/zed/DOJO-WINDOW-REUSE.md`). Reopening presents its first frame **7–8 ms** after
  the command in the fast runs and 15–20 ms in the slow set, against 35–39 ms and 80–89 ms
  for creating the window. The search is empty and the catalogue is refreshed on every
  opening, as before.

### Time until the panel is on screen

One screenshot per trial, taken a fixed delay after the command, counting trials whose
panel colour fills a square at the top of the panel (a dedicated probe, because polling
screenshots back to back cannot resolve less than one screenshot, about 130 ms here):

| Delay after the command | baseline | hidden surface reused |
| --- | --- | --- |
| 50 ms | 0 of 5 and 0 of 5 | 4–5 of 5 (swapchain kept); 5 of 5 and 2 of 5 (final) |
| 60 ms | 0 of 5 and 0 of 5 | 5 of 5 (swapchain kept); 5 of 5 and 3 of 5 (final) |
| 80 ms | 3 of 5 and 4 of 5 | 5 of 5 |
| 100 ms | 5 of 5 | 5 of 5 |
| slow set, 60 ms | 0 of 8 (never within 120 ms) | 4 of 8 and 5 of 8 |
| slow set, 70 ms | 0 of 8 | 8 of 8 |

The panel appears about 25–30 ms sooner in the fast runs. What remains is mostly the 150 ms
reveal's ease-in: its top eight pixels take about 33 ms to appear whatever the window cost.
`dojo --launcher` from a Niri bind also pays `niri msg --json focused-output`, a child
process that takes about 16 ms (`shell::focused_display_id`) and is now the largest cost
left before the window; asking Niri in-process through `niri-ipc` would remove it.

### Memory

The resident launcher releases its swapchain and path textures when hidden and creates a new
surface on the same device when shown. A first version that kept them held 100 MB more video
memory while hidden (247 MB against 147 MB for the whole shell here) and reopened in 4–5 ms
instead of 7–8 ms; it was not kept.

| Idle shell, nested, 60 s | RSS | PSS | video memory | CPU |
| --- | --- | --- | --- | --- |
| baseline | 158.7–162.7 MB | 110.0–113.7 MB | 147.3 MB | 0.9 %, 2.1 % |
| shared pipelines, hidden launcher | 150.5–153.8 MB | 102.6–106.3 MB | 147.3 MB | 2.3 %, 1.7 % |

The lower resident memory comes from sharing pipelines between the shell's resident windows (a
build with only that change read 153 MB); a hidden launcher that kept its swapchain added about
6 MB back.
The idle CPU readings are noise: the same binary read 0.2–1.4 % in ten-second windows. Over 40
openings the resident set grew 9.0 MB in the baseline and 7.6 MB with reuse (first-use caches),
and video memory did not move. The live shell's figures from 2026-10-23 (99 MB RSS, 53 MB PSS,
0.4 % CPU, release build) are a different build and are not comparable.

### Not done or not verified

- The live session and a release build were not measured. The nested session has a single
  output, so removing an output while the launcher is hidden (Niri then closes the surface and
  the next toggle opens a new one) was checked by reading, not run. A scale change while hidden
  was run.
- Wayland compositors other than Niri were not tried. After a null commit Niri resets a layer
  surface to the background layer, which the hidden launcher reset by sending the layer again, so
  a compositor that implements layer shell version 1 cannot hide a surface this way and the
  launcher destroys it instead.
- A persistent Vulkan pipeline cache is not used. The pinned wgpu exposes
  `Features::PIPELINE_CACHE` (Vulkan only) with an `unsafe` `Device::create_pipeline_cache`, but
  GPUI does not request the feature, desktop drivers keep their own shader cache, and with
  pipelines shared the cost is paid once per process (15 ms here).
- The other panels (Wi-Fi, Bluetooth, displays, theme and model pickers, system menu, quick
  terminal, shortcuts, session overlay) still create and destroy their windows; they gain only
  the shared pipelines.
