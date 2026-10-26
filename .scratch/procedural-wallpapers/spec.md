# Procedural wallpapers

A wallpaper can be drawn instead of chosen. A photograph is a fixed grid of pixels that Dojo has to fit to each monitor, and the more a monitor stretches it, the softer it looks; on four 1080p monitors, two of them rotated, that is most images. A procedural wallpaper has no pixels of its own. It is a recipe, and every monitor draws its part at its own resolution, so it is exactly as sharp as the monitor and seamless across monitors.

See [the glossary](../../GLOSSARY.md) for **Procedural wallpaper**, **Recipe** and **Canvas**.

## Experience

The Wallpaper tab of the theme picker gains a row of styles (Contours, Sunset) under **Choose image…**. Clicking a style draws a new one with a new seed and previews it; the layout switches to **Span all monitors**, which stays a choice until **Apply wallpaper**. A second row picks the colours: **Theme colours** (the default), **Random colours**, or **Paste palette**, which reads any text on the clipboard with at least two `#rrggbb` colours. Choosing colours redraws the same picture in them so palettes can be compared.

Applying keeps the palette, preset, appearance and targets, as for an image. The generated wallpaper does not follow later theme changes: regenerate it.

`dojo-theme procedural preview STYLE OUT.png` draws a style without touching the desktop. `dojo-theme procedural apply STYLE` selects one and saves the layout.

## Decisions

- **A recipe, not pixels.** The recipe (style, seed, three accents over a background) is stored beside the retained `wallpaper` in the theme generation as `wallpaper-recipe.json`, and in `theme.json`. The picture is a continuous function of position over the canvas, so a monitor evaluates it at its own pixels and nothing is resampled.
- **The canvas is the bounding box of the monitors** in Niri's logical coordinates, one unit tall. Spanned, a monitor shows the rectangle it occupies, so a gap skips the picture behind it. Filled, it shows the largest centred part with its own shape.
- **Spanning is the default.** With no saved layout a generated wallpaper spans where an image fills. A saved layout, including an explicit fill, wins.
- **A reference picture is still retained.** The retained `wallpaper` is a 1920 px PNG of the whole canvas. Palette derivation, the lock screen and the login screen read it like any image, so none of them changed. The wallpaper surface never decodes it.
- **Sunset knows the theme's appearance.** A palette from a theme records whether the theme is light. The picture keeps its dark ground either way, but a light theme gets a bright sun and a pale sky, and a dark one a dim moon (its brightness capped whatever the theme's brightest accent is) with a few stars. Facets of the low-poly ridges are lit from the moon or sun and shaded from the palette's own colours.
- **Theme colours are chosen by role.** The accents are a deep shade of the theme's primary, its primary, and its secondary, in that order. An earlier version picked three of the primary, secondary and info colours and ordered them by brightness, which put Tokyo Night's cyan in the slot that most lines, glows and shading use, so everything came out teal.
- **Two styles.** Contours and Sunset. Strata and Ripples, tried and dropped, are gone.
- **No limits on the theme's colours.** The ground is the theme's `background`, the shell's own colour, and the accents are its primary and secondary, exactly as stated. Nothing is darkened or capped, for a dark theme or a light one. An earlier version darkened the ground to a fixed deep tone, which made Tokyo Night Storm (`#24283b`) nearly the same as Tokyo Night (`#1a1b26`). Sunset is a flat low-poly valley, in the theme's three colours and no others. The sky is the background itself, flat, with a warm gradient laid over it that never exceeds 10% (the clouds are held to the same 10%). A flat sun (the secondary itself, no glow) sets in a valley: eight faceted ridges dip toward it, each lighter and hazier than the one in front (far ridges lean on the secondary, nearer ones on the primary, the nearest settles to the ground), with stepped pines along the front three. Colours are blended in gamma space, as an illustrator's colour picker does; blending in linear light is what made earlier versions glow. The facets are fine (about 0.05 to 0.12 of the canvas's height across), so a rotated monitor showing a quarter-width slice of a four-monitor canvas still holds a full scene. A light ground gets the same sky with ridges that grow stronger toward the front.
- **No cache, no decode.** Drawing 1920×1080 takes a fraction of a second on all cores; a cache would add state for little gain.

## Not done

Following the theme as it changes, so that switching presets redraws the wallpaper in the new colours, was considered and declined.
