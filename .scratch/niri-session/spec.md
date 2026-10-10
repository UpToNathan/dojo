# Forked Niri desktop session

The next UWSM Niri login uses the pinned Dojo compositor fork with independent agent input enabled. Keep existing UWSM lifecycle, session notification, D-Bus and screencast support. Build the default features in release mode through devenv. Do not restart the active compositor during installation.

Install the binary in a content-addressed user data directory, retain its Nix runtime library closures, and atomically update a current symlink. Preserve the previous installation and replaced user configuration files. A user service drop-in overrides UWSM's generated compositor command after its `50_custom.conf`. Home Manager retains the wrapper and drop-in sources.

Verify the current Niri config parses, runtime dependencies resolve, the release binary passes nested input and fullscreen checks, and the user service's effective next-start command invokes the fork. Actual DRM-session startup requires a new login.
