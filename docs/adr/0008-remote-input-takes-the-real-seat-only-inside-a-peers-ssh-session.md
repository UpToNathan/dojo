---
status: accepted
---

# Remote input takes the real seat, and only for a process inside a peer's Tailscale SSH session

[ADR 0007](0007-a-node-accepts-input-only-from-peers-it-names.md) said accepted input is injected as the human's seat. An agent seat could not do that: it is a second cursor that never reaches Niri's key binds, and it stops to ask whenever the person at the machine touches something. So remote input now enters Niri as the machine's own keyboard and pointer (`Request::RemoteInput`): binds fire, one real cursor crosses every monitor, and typing goes where it would from the hardware.

That is also exactly what the Niri fork shuts everyone else out of (virtual keyboards are disabled so that no process of the user's, the AI agent included, can type as the human). The gate for `RemoteInput` therefore cannot be a flag or a name the caller supplies, because every process of the user can supply those. Niri checks the kernel's process tree instead:

- The caller must run inside a Tailscale SSH session that the **root** `tailscaled` started: an ancestor of the caller is a process of uid 0 named `tailscaled` whose direct child is the session's `tailscaled be-child ssh` helper. Only the root daemon can start such a child, so a user process cannot fake the shape.
- The identity comes from that helper's command line, which the daemon wrote (`--remote-user=tag:venus --remote-ip=100.x`), not from anything the caller says.
- A session from an address that belongs to this machine is refused. An agent running here can `tailscale ssh` to its own node, but that is not a peer.
- The screen must be unlocked. Locking ends every session and keeps it ended; a locked machine cannot be typed into from elsewhere.
- Everything a connection holds is released when it closes.
- The person at the machine can end it from their own keyboard with Ctrl+Alt+Shift+Esc. Hardware only counts: events a peer sends never reach that check. A stopped connection stays stopped.

`dojo-mesh inject` keeps its own checks (`accept-input` against `tailscale whois`, no agent origin, not locked). Those are policy about which node may send; the Niri gate is what stops a local process from skipping `dojo-mesh` altogether.

## Considered options

- **Feed the real seat from any IPC client.** Rejected: any process of the user could then type as the human.
- **Resolve binds on the receiver and keep the seat (option B of `.scratch/mesh-remote-binds`).** Rejected by the user: it leaves a second cursor and no repeat, drag or wheel binds.
- **Check the caller's executable.** Rejected as the anchor: the user owns the install directory, so a replaced binary passes.
- **A root-owned helper binary.** Stronger against same-user replacement, but it needs a root install on every node. The process-tree gate needs nothing beyond Tailscale SSH, which the mesh already requires.

## Consequences

- **What this does not stop:** another node that the tailnet's SSH policy lets in. That node can already run any command as this user, so remote input adds no authority it lacked; the tailnet policy remains the permission system (ADR 0005). It also does not defend against a compromised `tailscaled`, which can already do anything as root.
- On the sending node the AI agent still has no way to start remote input (it is operator-only in `dojo-mesh remote`), but that is the tool surface, not a kernel boundary; an agent that can run `tailscale ssh peer` can reach the peer's `RemoteInput` as that node. Closing that is the same open item as keeping the agent off root ssh (ADR 0006).
- A Niri without `RemoteInput` still works: `dojo-mesh inject` says so once and falls back to the seat.
- Both nodes need the new Niri, which takes effect at the next login.
