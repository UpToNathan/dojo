# T3 backend and Rift setup

Use T3's existing provider runtimes and subscription authentication through a
standalone local backend. Home Manager manages its CLI and freedesktop
autostart entry. The backend uses the existing T3 data directory and runs for
the graphical session, independently of the client window.

Prevent concurrent desktop-owned and standalone backends from opening that
data directory. Preserve the current running conversation during installation.
Document any client pairing or restart needed to complete the handoff.

Install a pinned Rift CLI through devenv and Home Manager. Configure the
matching OpenCode V2 plugin to use that executable. Project registration is
explicit because Btrfs initialization replaces a directory with a subvolume.

Dojo's voice input, visual responses and T3 RPC client are subsequent work.
This setup supplies the local runtime and workspace tools they will use.
