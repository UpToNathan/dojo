# Super+T reattaches, Super+Shift+T picks

Status: ready-for-agent
Blocked by: 05

Niri binds Super+T to `dojo-terminal --reattach` and Super+Shift+T to `dojo-terminal --picker` through `home-manager.nix`; reattach focuses an existing window for the session through Niri IPC instead of opening a second.
