# Add all modes with constrained semantic feedback

Status: resolved

The user requested status colors fitted into semantic ranges, with mode-specific
hue and chroma rather than fixed status colors. Implement all 23 existing mode
names in the refined recipe and show both wallpapers in dark and light mode.

Added independent mode profiles and a constrained feedback fitter. It verifies
hue bounds after gamut mapping and 8-bit RGB rounding. Status foregrounds and
fills follow semantic ranges; actual surface contrast determines lightness.
Normal preserves the image's main hue, harmony modes use hue relationships, and
mood modes combine anchored hue families with the wallpaper hue. The new API
analyzes the wallpaper once for all 23 modes.

Four focused tests pass, including all modes over 24 source hues and both
appearances, visible semantic chroma, image-driven accents, grayscale neutrality,
mode-dependent feedback, and standard OKLab primary-color round trips. The
study renderer checks all 92 example palettes and every mode/appearance control.
Live derivation remains disabled pending evaluation.
