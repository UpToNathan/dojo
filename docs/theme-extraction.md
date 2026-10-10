# Wallpaper palette extraction

## Scope

The Rust extraction module ports Aether's algorithm from revision
[`29d07b1a8f474c77bf02aece948373f40abed105`](https://github.com/omacom/aether/tree/29d07b1a8f474c77bf02aece948373f40abed105/internal/extraction).
This is a headless palette generator, not an Aether dependency or a complete theme
manager. It never applies desktop settings. Generated ANSI palettes are separate
from Dojo's current five semantic UI roles.

All **23** modes in upstream `validateMode` are supported: normal, monochromatic,
analogous, pastel, material, colorful, muted, bright, complementary, triadic,
split-complementary, tetradic, fire, ocean, forest, earthtone, neon, sunset,
vaporwave, midnight, aurora, high-contrast, and duotone. Each supports light and dark.

The port includes:

- Aspect-preserving, 400-pixel Catmull–Rom sampling and alpha rejection.
- Graded chroma boosting, then 48-bucket median-cut in upstream's OKLab transform.
- Normal-mode monochrome/chromatic classification, chromatic support constraints,
  tint detection, monochrome lightness headroom, and global greedy ANSI assignment.
- The exact mode constants, HSL material generator, derived bright/comment slots,
  gamut clipping, and upstream contrast normalization.

Wallpaper editing, the adjustment sliders, caching, multi-image blending, theme
storage, darkman integration, app exports, preview UI, and applying generated colors
are not part of this extraction-mode port.

## Interface and ownership

`extract(path, light, mode)` reads a local JPEG/PNG/WebP image and returns a palette.
`extract_pixels(pixels, light, mode)` uses already-sampled straight-alpha RGB pixels
and runs the same quantization and generation stages. `Palette::hex_colors()` exposes
16 `#rrggbb` strings. Invalid modes, insufficient pixels/colors, and decoding errors
are returned to the caller rather than silently selecting another mode or image.

The CLI prints JSON, keeping generation separate from activation. Selecting a theme
must eventually use the shared palette to derive Dojo roles and app-specific outputs,
not replace application configs with ANSI slots or overwrite the user-owned config.

## Verification and fidelity

Checked-in fixtures are generated from the **unmodified upstream Go algorithm**:
four synthetic dominant-color sets (chromatic, grayscale, blue-tinted, warm) and
Go's dominant colors from the repository's `astro.jpg`. Each exercises 23 modes ×
2 appearances: **230 exact RGB palette comparisons**. Fixture generation disables
upstream caching and uses minimal image-decoding glue instead of Aether's desktop
application. The harness never launches Aether or modifies desktop settings.

Additional fixtures check:

- Exact alpha-aware resampling on a generated lossless PNG containing transparent,
  translucent, and opaque color gradients.
- Exact quantized bucket colors and counts from identical Go-sampled `astro.jpg`
  pixels, followed by exact matching for all 46 generated palettes.

Go sorts equal-coverage buckets unstably; Rust uses a deterministic stable sort.
The quantization test compares the bucket/count multiset rather than accidental tie
order, then verifies final palettes. Sampled JPEG input can still differ: Go's JPEG
decoder/scaler uses YCbCr-to-16-bit conversion while Rust decodes through an 8-bit
RGB image. Consequently, byte-identical end-to-end JPEG palettes are **not** claimed.
Small pixel changes can change selection/ranking, especially near thresholds.

The port preserves upstream's numerical transforms, including its OKLab matrix
coefficients and contrast steps. It does not silently replace them with a color
library's standard conversion or stronger guarantees. Contrast targets are not a
proof that every output/role combination meets accessibility requirements: app-role
derivation still needs its own validation.

Coverage weights come from the **chroma-boosted** quantization pool, not an independent
count of original-image coverage. The upstream sampling stride calculation is also
preserved: it is not a strict 50,000-pixel cap (for example, 400×200 samples use stride
1 and yield 80,000 samples). JPEG/PNG/WebP support uses Rust decoders; input is bounded
to 16,384 pixels per side, 40 million pixels, and a 256 MiB decoder allocation limit.

## Reproducing fixtures

```sh
python3 scripts/regenerate-aether-fixtures.py
devenv shell -- cargo test --lib
```

Regeneration requires Python, Go ≥1.25, network access to the pinned GitHub source,
and `golang.org/x/image` v0.41.0. It creates a temporary Go module, downloads the
explicit color/extraction source files, runs `tests/reference/aether/generate.go`,
and writes only `tests/fixtures/`. It never downloads or executes an upstream
installation/apply script. The generated astronaut samples derive from this repo's
existing wallpaper; no new wallpaper provenance or redistribution rights are inferred.

## Upstream licensing

Aether's pinned [README](https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/README.md)
declares MIT and credits Bjarne Overli (the inspected tree has no standalone LICENSE).
The copyright attribution and MIT terms are retained in `licenses/aether-MIT.txt`.
Sampling arithmetic follows
[`golang.org/x/image/draw` v0.41.0](https://github.com/golang/image/tree/v0.41.0/draw),
whose BSD notice is retained in `licenses/go-image-BSD.txt`.
