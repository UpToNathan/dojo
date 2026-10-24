# Emoji command

Status: resolved

Blocked by: 01

A view that searches emoji by name and keyword and copies the chosen one to the clipboard. Data is generated at build time from nixpkgs `unicode-emoji` (names, groups) and `cldr-annotations` (keywords), with no runtime download and no runtime devenv. Fuzzy search with nucleo, frecency-ordered recents, skin tones as a variant on Tab. No automatic paste.

## Comments

Built: generated Unicode 17 / CLDR 48 table (1,914 emoji, 144 KB, `scripts/generate-emoji-data.py`), ranked search, Recent group, Tab for skin tones. Not verified: colour glyph rendering in the live session (the nested harness has no emoji font).
