# Continuous wallpaper handoff

Changing wallpapers must keep a complete image visible throughout the transition.
Dojo owns the selected wallpaper. Retire the original Astro btrbg startup provider
and the picker's hard-coded Astro fallback; keep the user's wallpaper files.

Read and decode a replacement before changing the rendered source. Keep the old
image and GPU resource through the new frame, then release it. Reuse an unchanged
content digest. Discard stale background work when a newer selection event arrives.
Use GPUI's existing loader so animated images retain the existing decode behavior.

The live regression temporarily uses two opaque fixture images, samples rendered
pixels on an empty workspace, and restores the exact original theme activation and
workspace. Calibrate fixture colors from the display to account for output color
management. Any third color during handoff is a failed frame.

Prepare monitor-specific cover images in background work using Lanczos filtering,
at the wallpaper window's viewport multiplied by its display scale. Retain the
original decoded source for new outputs and scale changes, and retain each
monitor's current image until the replacement has finished filtering. Preserve
frame delays and discard work for superseded sources or target sizes.
