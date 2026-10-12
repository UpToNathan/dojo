# Niri logout on 2026-10-11

At 18:11:02 EDT, Niri PID 591673 aborted inside Mesa's AMD driver. The kernel
reported `amdgpu_vm_validate() failed` and `Not enough memory for command
submission!`. Systemd stopped the graphical session, and Ly closed the login
session at 18:11:04. No logout command was needed to produce this sequence.

The crashed executable was the installed Dojo fork at
`~/.local/share/dojo/niri/91de956b9ee4ee61/niri`. Its aborting thread was
`niri:cs0`, with `amdgpu_ctx_set_sw_reset_status` and `amdgpu_cs_submit_ib` in
Mesa 26.2.2 on the stack. The core dump remains in systemd's coredump storage.
The machine runs kernel `7.2.8-arch1-2` and exposes an 8 GiB AMD VRAM budget.

## Confirmed leak

Before the fix, `FrameView::render` called `TrayItem::render_image` for every
visible tray icon on every frame. That method creates a new `RenderImage`, which assigns a fresh
image ID even for identical pixels. GPUI's `Window::paint_image` caches each
ID in the GPU atlas. Dropping a `RenderImage` does not remove that atlas entry;
removal requires `Window::drop_image`.

The live tray provides two pixmaps, 250 by 250 and 256 by 256 pixels. A bounded
live test with two speech bubbles increased Dojo's GPU allocation from 958532
KiB to 1351748 KiB in 30 seconds, a 384 MiB increase. Niri's allocation rose
initially for blur and then stayed at 480936 KiB. Eight screenshot captures
increased Niri's allocation by about 22 MiB in this test.

The isolated `tray_gpu_probe` renders two fixture tray items with those same
dimensions through the actual `TrayItem::render_image` and GPUI image path.
It excludes speech UI, screenshot capture, and tray D-Bus services. Over 300
measured frames, recreating images grew process GPU allocation from 108384 KiB
to 272224 KiB, a 160 MiB increase. Reusing the same images stayed at 108384 KiB.
The process exits after each case, releasing its allocations.

The tray image creation was already present in HEAD. Continuous speech
animation accelerates an existing leak by redrawing the entire desktop frame.
Even without animation, the frame's one-second refresh can accumulate images.

## Limits and correction

This proves a desktop GPU leak and matches the driver allocation failure.
There is no per-process GPU allocation snapshot from the exact crash moment,
so the investigation cannot prove that this leak alone exhausted the GPU then.
The captured crash was not deliberately reproduced because another GPU
exhaustion could end the user's live session.

`TrayImageCache` now retains each tray icon's image while its pixmaps are
unchanged. Each frame window updates its own cache with the tray snapshot and
removes replaced, hidden, or disappeared images from its atlas. Rendering
uses the cached image. Screenshot support is unchanged.

The rebuilt shell was activated through the existing autostart service without
restarting Niri. In the same live 30-second speech-bubble test, Dojo's GPU
allocation rose from 565296 KiB to 565300 KiB during warmup, then stayed flat.
The previous 384 MiB accumulation was absent. No new driver allocation or
renderer errors appeared. Both tiled windows retained their original sizes
after restoring one column's width during shell activation.

The 300-frame cache regression checks reported zero GPU allocation growth
for unchanged icons and for repeated replacement, hiding, and removal. The
old recreation path still grew by 160 MiB. All 39 shell unit tests passed;
one existing launch test was ignored. The nested fullscreen check passed all
eight edge comparisons and confirmed that the desktop frame returned.
Live frame and layout evidence is in `/tmp/dojo-gpu-fix-verification/`;
fullscreen evidence is in `/tmp/dojo-agent-nested.MqmqGX/`.

## Reproduction

The check opens four small, unfocused windows in sequence and closes them.
It uses the active systemd session's Wayland environment and stops before
launching a case if global GPU allocation exceeds 4 GiB.

```sh
devenv shell -- cargo build --example tray_gpu_probe
devenv shell -- python3 scripts/check-tray-gpu-memory.py
```

The check passes when recreation grows by more than 64 MiB, reuse and the
production cache grow by less than 16 MiB, and icon churn grows by less than
32 MiB. The cache cases exercise the production `TrayImageCache` and the
real GPUI atlas. The live speech test also verifies `FrameView` uses that cache.

The kernel error originates in buffer-list validation, so it does not by itself
establish system RAM exhaustion or disk exhaustion. See the
[Linux AMD command-submission source](https://code.googlesource.com/linux/torvalds/linux/+/3f008280327ba5ad132965abab0c7846283cef0c/drivers/gpu/drm/amd/amdgpu/amdgpu_cs.c).
