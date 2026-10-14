# Filter wallpapers for each monitor

Status: resolved

## Problem

A 6749×5203 wallpaper looks grainy when directly sampled onto a 1920×1080
output. The shared full-resolution texture has no minification mipmaps.

## Acceptance

- Prepare a Lanczos-filtered cover crop for each window's physical pixel size.
- Include fractional display scale and refresh preparation after size/scale changes.
- Keep the source snapshot intact and preserve animation frame delays.
- Retain the current prepared image until its replacement is ready; discard stale work.
- Check high-frequency filtering, dimensions, frame timing, and live handoff.

## Resolution

Wallpaper views retain the decoded source and prepare their own centered cover
crop with Lanczos3 in background work. The target comes from the actual window
viewport multiplied by its scale factor, rather than shared logical dimensions.
Completed images are cached per view/source/size; stale work cannot replace a
newer source. The previous image remains rendered through preparation and is
retired after the next completed frame. Original snapshots and frame delays
remain intact; small inputs are not enlarged during preparation.

Checks from devenv: 2 wallpaper tests and 15 picker tests passed. The filtering
regression reduces a fine checkerboard to a neutral image, checks target
dimensions and frame timing, and verifies small sources are not enlarged.
Live HDMI captures `/tmp/dojo-wallpaper-grain-before.png` and
`/tmp/dojo-wallpaper-grain-after.png` show the speckling removed. HiDPI evidence
is `/tmp/dojo-wallpaper-grain-after-hidpi.png`. The live handoff regression
captured 234 frames with zero unexpected colors; retained evidence is in
`/tmp/opencode/dojo-verification/monitor-resampling-handoff/`. Exact original
activation and workspace were restored by the probe.

Release `9cf87b7145e6afe1-mmap128k` installed and desktop service restarted.
