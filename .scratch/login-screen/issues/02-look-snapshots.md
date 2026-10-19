# Look snapshots

Status: ready-for-agent

Pure format and boundary code for the look snapshot in the spec: what is copied, size caps, validation of `palette.json` and the wallpaper, atomic writes under `/var/lib/dojo-dm/users/<login>/`, and a reader for the greeter. The reader of a home directory runs after dropping to the user and refuses symlinks. Reuse `session::host` and `ConfigStore` rather than re-parsing the palette.

## Acceptance

- A snapshot round-trips; a missing, oversized, symlinked or malformed file leaves the previous snapshot in place.
- The greeter-side reader never touches a home directory.
- Tests use temporary directories; no root needed.

## Comments
