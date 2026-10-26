# Browser Control

Dojo reuses anomalyco/browser-control `0.8.4` at
[`b4e4238742fa73daabfb664f1108113e9577849d`](https://github.com/anomalyco/browser-control/tree/b4e4238742fa73daabfb664f1108113e9577849d).
The literal source is in `vendor/browser-control/upstream`. The local patch and
manifest record the native transport and cancellation changes. CUA's former
`desktop_browser_*` adapters are removed. Desktop input and capture remain
separate tools.

## Installation

`nix/browser-control.nix` pins source, pnpm 11.20.0 and dependencies. Devenv and
Home Manager consume the same package and immutable native broker. Niri is
installed separately. Its recorded integration hashes are checked by the explicit
provenance audit; unrelated Niri edits do not block browser package evaluation. Home Manager
links the extension at `~/.local/share/dojo/browser-control/extension`, installs
the shared browser skill and merges supported agents' MCP configuration while
preserving other settings and keeping private backups. OpenCode's configuration
is maintained by `rift-home-manager.nix`; Pi uses its own MCP extension.

The Helium launcher loads the extension from that stable path. A running browser
must exit normally and relaunch for changed launcher flags. Niri must start with
`DOJO_BROWSER_CONTROL_BROKER_EXE` set to the installed immutable broker. The
native channel is unavailable on an older running compositor.

## Ownership and task lifetime

Each MCP process owns its sessions. Call `session_resume` before execution and
`execute_cancel` at completion, stop or human handoff. Pi performs this lifecycle
automatically and exposes tools as `browser_control_*`. Cancellation retires an
exact generation outside the execution queue, preserves adopted and created tabs,
and prevents old work from adopting a later generation.

To attach an existing page, click the extension toolbar button on that page and
adopt it deliberately. Human browser input retires browser command generations.
Dojo's talk-key release yields human input ownership; other workflows require
an explicit operator command, bound to **Super+Alt+Y**:

```sh
~/.local/share/dojo/niri/current/niri msg yield-human-input
```

The `niri` on `PATH` is stock and has no such command; the compositor's own binary does.

This does not move the human cursor or keyboard focus. Held keys and active grabs
must first finish. Agents must not run this command to override takeover.

Niri owns the final extension socket writes. It orders command admission with
human input on its input actor and refuses queued work after takeover. A kernel
write admits a command; it does not prove the page effect has finished. Commands
already admitted, and page timers they installed, may continue after cancellation.
Pi retains uncertain outcomes and blocks automatic continuation. A new screenshot,
reconnect or retry cannot prove the earlier operation did not happen.

The gate covers browser clients conservatively. It does not claim an exact
mapping between a browser tab and a native Niri window. No focus activation,
primary-pointer fallback, raw CDP bypass or automatic effect retry is permitted.

## When it will not resume

- **Native lease refused**: the person used the keyboard or mouse since the last lease. Press Super+Alt+Y,
  then call `session_resume` again. The agent never does this itself.
- **A restarted MCP client**: each MCP process names itself `browser-control-<pid>-<start>-<uuid>`. The relay
  retires the lease of any owner whose process is gone, within ten seconds (or at once when another client
  resumes or cancels that session), so a restarted client takes the session over instead of finding it
  held. An owner set by `DOJO_BROWSER_OWNER` names no process and is never reaped.

## Checks

Run from devenv:

```sh
node --test tools/speech/pi-browser.test.mjs tools/speech/pi-desktop.test.mjs
python3 scripts/test-install-browser-control-config.py
cargo test --manifest-path crates/dojo-browser-control/Cargo.toml
cargo test --bin dojo-agent
```

Live browser checks use an isolated profile in a nested compositor on the inactive
`dojo-tests` workspace. They never use the authenticated profile or active desktop.
See the [migration spec](../.scratch/browser-control/spec.md) and
[transport contract](../.scratch/browser-control/transport-contract.md).
