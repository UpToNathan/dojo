# Refined wallpaper theme recipe

The user requested read-only light and dark examples from `~/Pictures/astro.jpg`
and `~/Pictures/osaka-jade.jpg`. Preserve wallpaper character while keeping error
red, warning amber/orange, success green, and information blue. The user approved the mode study and requested restoring Derive with this recipe.

`theme::derive_wallpaper(bytes)` produces the normal recipe.
`theme::derive_wallpaper_modes(bytes)` analyzes the image once and produces all
23 named modes, each with light and dark semantic palettes. A separate module owns standard OKLab/OKLCH color math and image sampling;
it does not change the pinned Aether port or adapt static presets.

Sample an oriented image into a 256-pixel thumbnail. Group chromatic samples by
10-degree hue bins, weighted by chroma squared and pixel coverage. Use the winning
bin's circular hue mean and chroma for bounded accents and restrained surface tint.
For achromatic images, use neutral accents. Surfaces have controlled tonal steps;
light panels are brighter than the canvas and dark panels are lighter than it.

Modes produce unconstrained aesthetic hue families, chroma, tint, and preferred
lightness. Project status proposals into OKLCH hue sectors: error 10–35°,
warning 55–95°, success 125–165°, information 205–260°. Retain each mode's chroma
within a visibility floor and an upper bound. Fit lightness to all permitted
text surfaces and each status fill at 4.5:1, or 7:1 in High Contrast mode.
Gamut mapping reduces chroma; verify the quantized RGB result and move slightly
inward if rounding crosses a hue boundary. Fills follow the same hue constraints.
Syntax uses unconstrained mode colors independently of feedback. Terminal slots
are fitted into their conventional hue ranges as well.

The 23 modes are new implementations of the named aesthetics, not exact Aether
output. Complementary/triadic/analogous modes use their corresponding hue
relationships; mood modes use anchored hue families influenced by the wallpaper.
Pastel, muted, monochromatic, bright, neon, and high-contrast modes control chroma,
preferred lightness, or contrast targets. Feedback colors vary by mode inside
semantic constraints, rather than using one fixed status palette.

Previews use the actual serialized Rust palettes, the two source wallpaper files,
and the existing Victor Mono subset with ss01. They show hover, selection/focus,
actions, labeled feedback, syntax, terminal colors, and measured text contrast. A mode selector exposes all 23 modes; expandable
proposal mappings show the mode palette before and after semantic fitting.

New picker and CLI derivations use this recipe. Presets and older persisted
generations keep their stored colors. Wallpaper-only Apply remains independent.
