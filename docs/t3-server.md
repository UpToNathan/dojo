# T3 Code server

`t3-home-manager.nix` exposes the installed T3 Code Electron bundle's CLI as
`t3` and starts its backend from the graphical session. The wrapper is pinned
to `/home/nathan/.local/opt/t3code`; it runs Electron in Node mode with the
bundled `apps/server/dist/bin.mjs` entrypoint, matching T3's own CLI shim.

The autostart entry first turns off T3 Desktop's local environment, then runs
`t3 start --no-browser`. The setting migration runs only while the app is
closed, keeps a timestamped backup, and atomically replaces the settings file
while preserving its metadata. If T3 is running or the setting is still
enabled, autostart refuses to start a second backend. `t3-prepare-client` runs
the same guarded migration manually; close T3 before using it. The desktop
settings UI also supports this at **Settings → Connections → Local
environment**.

The standalone server and desktop client share T3's default data directory at
`~/.t3`, so the server sees the existing providers, projects and threads. T3
Desktop must keep its local environment disabled while this standalone server
owns that data. To add it as a client connection, create a pairing link with
`t3 pair` and use **Add Environment** in the desktop app. Keep pairing links
private.

The T3 CLI's `start` command also checks
`~/.t3/userdata/server-runtime.json` and refuses to start when its recorded
process is alive. `serve` does not make that same preflight, so autostart uses
`start`.

Run `t3 --help` or `t3 <command> --help` to inspect the installed bundle's CLI.
The wrapper accepts and forwards all arguments, so normal CLI operations use
the same existing installation and data directory.
