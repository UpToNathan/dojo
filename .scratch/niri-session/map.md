# Forked Niri desktop session

## Notes

The installed fork is selected for the next UWSM login. See [the spec](spec.md).

## Decisions-so-far

- [Installation](issues/01-install-session.md): retain a content-addressed release binary and Nix runtime roots; override UWSM with a user service drop-in. The production and installed binaries pass nested MCP/GPUI checks, and production fullscreen checks pass. Service inspection confirms the next start selects the fork.

## Fog

The current desktop compositor is still upstream. DRM startup and T3's inherited socket need verification after the user saves work and logs out and back in.
