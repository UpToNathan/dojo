# 04 — Exact Niri geometry and native Rust capture

Type: task
Status: needs-triage
Blocked by: 02, 03
Effort: large

## Brief

Extend Niri IPC with trustworthy positioned tiled/floating geometry and identity/generation evidence needed by captures, AT-SPI and browser bindings. Inspect compositor layout/scene callers before changing behavior. Include active-workspace, output transforms/scales, clipping and popup/occlusion coverage; unavailable evidence must stay unavailable rather than guessed.

Reuse `platform-linux/src/wayland/ext_screencopy.rs`, but fix first-advertised-output capture and deadline loops using potentially blocking roundtrips. Implement named output selection, cancellable bounded dispatch, region/scale handling, image-size limits, stride/format/alpha/transform handling and connection lifetime. Preserve visible-composited capture semantics and post-capture geometry validation. Retain grim as an explicitly selected migration backend until parity; no Python runtime.

## Acceptance

- Unit/interface tests for geometry and image mapping; protocol versions handle older compositor cleanly.
- Nested tiled/floating captures on multiple outputs/scales/transforms; compare native capture and grim.
- Hung/missing Wayland protocol, disconnect, output removal and timeout stop promptly without orphan workers.
- No focus, clipboard, layout or human-pointer changes; no idle capture polling.
- Builds do not install or restart the compositor; deployment uses existing backed-up installer separately.

## Comments
