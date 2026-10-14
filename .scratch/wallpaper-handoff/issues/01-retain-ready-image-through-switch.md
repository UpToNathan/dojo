# Retain the wallpaper until its replacement can render

Status: resolved

The live regression captured 23 fallback frames out of 187 while alternating two
fixture wallpapers. The old controller cleared its image cache and switched paths
before the replacement was decoded. GPUI then painted no image during loading,
exposing the original Astro wallpaper owned by btrbg below it.

Prepare replacements through GPUI's image loader before publishing them to the
wallpaper windows. Release the old GPU image after the replacement frame renders.
Retire btrbg's original Astro startup entry with a scoped, validated configuration
install and backup. Preserve the selected theme, wallpaper files, and workspace.

Acceptance: the same live regression passes with no third-color frames, including
while btrbg is still running, and only Dojo wallpaper surfaces remain afterward.

## Comments

The deployed handoff passed with zero unexpected frames out of 122 while the
Astro provider still ran, then zero out of 125 after removing it. Evidence lives
in `/tmp/dojo-wallpaper-before-calibrated`, `/tmp/dojo-wallpaper-after-warm`, and
`/tmp/dojo-wallpaper-without-fallback`. The original activation and workspace
were restored after each probe. The scoped Niri configuration was validated and
installed with a timestamped backup, the old provider was stopped, and Dojo is
the only wallpaper owner on the currently active output.
