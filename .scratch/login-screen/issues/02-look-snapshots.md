# Look snapshots

Status: resolved

Pure format and boundary code for the look snapshot in the spec: what is copied, size caps, validation of `palette.json` and the wallpaper, atomic writes under `/var/lib/dojo-dm/users/<login>/`, and a reader for the greeter. The reader of a home directory runs after dropping to the user and refuses symlinks. Reuse `session::host` and `ConfigStore` rather than re-parsing the palette.

## Acceptance

- A snapshot round-trips; a missing, oversized, symlinked or malformed file leaves the previous snapshot in place.
- The greeter-side reader never touches a home directory.
- Tests use temporary directories; no root needed.

## Comments

The wallpaper is stored decoded. `lock-wallpaper.raw` is `DJWP`, width and height as `u32` LE, then BGRA, at most 512x2048 (header plus 4 MiB); `Wallpaper::parse` checks only the header and the exact length. The collector decodes the user's PNG after the privilege drop with explicit `image::Limits` and under `RLIMIT_AS`, so the greeter, which sees every password, never runs an image decoder. A PNG that fails the limits is a rejected piece and the previous snapshot's wallpaper stays.
