# Dojo patch: colour emoji ahead of DejaVu Sans in font fallback

`crates/gpui_wgpu/src/cosmic_text_system.rs` builds its `FontSystem` with its own fallback lists
(`ColorEmojiFirstFallback`) instead of `FontSystem::new()`.

## Why

cosmic-text's Unix fallback tries `DejaVu Sans` before `Noto Color Emoji`. DejaVu Sans has
monochrome outline faces for the whole Emoticons block (U+1F600–1F64F), so an emoji such as 😀
that the interface font lacks drew as a text-style outline while 🤣 (not in DejaVu) drew in colour.

Naming the emoji font on the element does not work: `load_family` refuses a face without an `m`
glyph as a primary font (so it can measure text), and emoji fonts have none. The only lever is the
fallback order.

## What changed

- `COMMON_FALLBACK` is cosmic-text's list with `Noto Color Emoji` moved ahead of `DejaVu Sans`;
  `forbidden_fallback` and `script_fallback` still come from `PlatformFallback`.
- `font_system_with_color_emoji_first` repeats what `FontSystem::new()` does (system fonts, the
  same default families, the locale from `LC_ALL`/`LC_MESSAGES`/`LANG`) and passes the new
  fallback to `FontSystem::new_with_locale_and_db_and_fallback`.
- `CosmicTextSystem::new` uses it. `new_without_system_fonts` is unchanged.
- `Cargo.toml` adds `unicode-script` (already in the lockfile through cosmic-text) for the
  `Fallback` trait's signature.
- Test: `colour_emoji_is_tried_before_dejavu_sans_and_nothing_else_is_lost`.

Side effect: a symbol that the interface font lacks and both emoji and DejaVu cover (©, ™, ♥, ▶)
now draws as a colour emoji instead of a DejaVu outline.
