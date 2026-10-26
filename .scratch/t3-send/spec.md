# Send a T3 Code window to another machine

Sending a T3 Code window to a peer opens the chosen thread in the peer's own T3 Code. The thread is not moved: T3 Connect keeps threads in step between the machines, and the peer's T3 opens it from a deep link. Terms are in [GLOSSARY.md](../../GLOSSARY.md#mesh); this extends [sending windows](../mesh-boundaries/spec.md) and needs no change to T3 Code.

## What T3 Code already gives

- `t3code://threads/<environmentId>/<threadId>` is a deep link: the running desktop app navigates to that thread (`promptLinkAppPath` in T3's `packages/shared`, handled by `DesktopWebLinkCoordinator`). The app registers itself as the `t3code://` handler.
- `t3 thread list --project <id|path> --json` lists one project's threads; the CLI cannot list projects, so the adapter reads the same data from `statev2.sqlite`.
- `~/.t3/userdata/environment-id` and `t3 thread`/`t3 app` need the T3 CLI wrapper Home Manager installs.
- Nothing tells an outside process which thread a window is showing: the window title is fixed and the backend only records a read watermark.

## Requirements

- A T3 Code window has its own handoff adapter. Sending it shows the machine's recent threads (unsettled first, newest by `updatedAt`), preselects the first, and the status line names the thread that will open. The operator can pick another before sending.
- The handoff carries the environment id and thread id only. The peer opens `t3code://threads/…` through its own handler, with `ELECTRON_RUN_AS_NODE` and other variables a T3 child process inherits cleared. It never runs a command it is sent, and it refuses while its screen is locked, like every handoff.
- The peer refuses, with the reason, when it has no T3 Code, or has not got that environment (the thread is not there to open).
- A T3 window is never closed by a send: the backend belongs to the desktop app and the window holds other threads. It is sent with the source kept, and the panel says so.
- A node that has T3 Code publishes a `t3-code` capability holding its environment id, through the existing `-file` attribute convention (`environment-id-file`), so `dojo-mesh find t3-code` lists the peers that can receive one. No machine name appears in tracked source.
- When a call waits for a person (1Password approving the node key, a locked agent), the send panel must not cover the prompt: see `.scratch/mesh-boundaries/issues/05-panel-hides-the-key-approval.md`.

## Out of scope

- Reading which thread a window shows (would need a patch to T3 Code or a debugging port).
- Moving a thread, its running agent or its files: T3 Connect keeps the thread in step; the agent keeps running where it runs.
- Patching or forking T3 Code.
