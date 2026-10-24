# Emoji command

Status: ready-for-agent

Blocked by: 01

A view that searches emoji by name and keyword and copies the chosen one to the clipboard. Data is generated at build time from nixpkgs `unicode-emoji` (names, groups) and `cldr-annotations` (keywords), with no runtime download and no runtime devenv. Fuzzy search with nucleo, frecency-ordered recents, skin tones as a variant on Tab. No automatic paste.

## Comments
