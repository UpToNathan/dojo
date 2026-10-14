# Implement and preview a semantic wallpaper recipe

Status: resolved

Implemented `src/theme/refined.rs`, independent of the existing Aether port.
Generated dark and light palettes from both user wallpapers through
`examples/refined-theme-study.rs`. `scripts/render-refined-theme-study.py` renders
those saved colors into an interactive comparison without inventing palette data.

Checks cover semantic hue retention and 4.5:1 text/pair contrast for a sweep of
wallpaper hues in both appearances, standard OKLab primary-color round trips,
and image-driven accents with neutral output for grayscale images. All three
focused tests pass. The rendered study also verifies its displayed text pairs,
appearance controls, and layout at desktop and phone widths.
The desktop remains preset-only. The generated study and its color data live in
`.scratch/refined-theme/`.
