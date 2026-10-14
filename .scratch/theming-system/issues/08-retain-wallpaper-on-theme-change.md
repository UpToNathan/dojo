# Keep the rendered wallpaper through theme changes

Status: resolved

Theme generations copy the selected wallpaper to new paths. The wallpaper controller must compare image content so unchanged copies do not clear the image cache and expose the fallback provider. A genuinely different wallpaper still updates normally.

Move the personal wallpaper to `~/Pictures/astro.jpg`, update the desktop startup and picker fallback paths, and retain extraction test input separately at `tests/fixtures/astro.jpg`.

The controller now retains its rendered path and image cache when snapshot bytes have the same SHA-256 digest. Pending events are drained before reading the selected wallpaper. The regression verifies identical content at different generation paths keeps the original rendered source, while different content updates it. Extraction tests pass using the relocated test fixture. Startup config was validated with the installed Niri fork and reloaded with a backup. The release is installed and running.

## Comments

The later flash while switching to different image content was reproduced and
fixed in `.scratch/wallpaper-handoff/issues/01-retain-ready-image-through-switch.md`.
Content identity prevented identical-copy reloads but did not cover the cold
decode gap for a replacement image. Dojo now prepares it before swapping and
the legacy Astro btrbg startup provider has been retired.
