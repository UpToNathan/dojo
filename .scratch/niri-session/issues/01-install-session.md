# Install the Dojo compositor for the next UWSM login

Type: task
Status: resolved

Implement [the spec](../spec.md). Keep the installed upstream compositor available for rollback and document next-login activation.

## Answer

Built the default-feature release compositor, including systemd, D-Bus and screencasting, and installed binary `fdaa18e74a348416` under `~/.local/share/dojo/niri`. The wrapper enables independent agent input and restores the retained runtime library environment. Nix GC roots preserve the library closures. Home Manager tracks the wrapper and the persistent UWSM override.

The release binary passes the nested MCP/GPUI input check (`/tmp/dojo-agent-nested.uxzZ1z`) and fullscreen regression with eight unchanged edge crops (`/tmp/dojo-agent-nested.fvtmPN`). The installed artifact also passes MCP/GPUI input outside devenv using its saved runtime environment. Current desktop configuration validates and installed libraries resolve. Repeating installation succeeds without replacing the current installation or restarting the compositor.

UWSM's effective next-start command invokes `/home/nathan/.local/bin/dojo-niri-session`. The active compositor remains upstream PID 1337. Actual DRM startup and T3's inherited socket require logout/login and remain unverified until that login. See [the session guide](../../../docs/niri-session.md) for activation and rollback.
