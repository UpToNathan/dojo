# Disable wallpaper-derived themes in the user interface

Status: resolved

Keep wallpaper derivation implementation in the repository, but disable it as a
user-facing theme mode while its flaws are being reconsidered. Static presets
remain the only palette-selection mode. Keep wallpaper preview and wallpaper-only
Apply independent from theme selection.

Removed Derive from the picker, stopped wallpaper selection from triggering
palette extraction, and disabled the `dojo-theme preview`, `generate`, and
image-based `apply` commands. Existing stored generations remain readable for
selection and rollback. Updated the theme docs and spec.
