---
status: accepted
---

# Peers are reached with Tailscale SSH, with no Dojo daemon between nodes

`dojo-mesh exec`, `cp` and `task` run `tailscale ssh <peer> <command>`, putting the caller's name, session and origin (operator or agent) in the remote command's environment as `DOJO_FROM`, `DOJO_SESSION` and `DOJO_ORIGIN`. Tailscale proves who is calling and the tailnet policy decides whether the connection is allowed (ADR-0005), so a node runs nothing extra to be reachable and a mesh of one runs nothing at all. Every call is appended to a local audit log before it starts and again with its result.


[ADR-0009](0009-a-signed-roster-is-a-second-membership-method-over-openssh.md) adds a channel over the system OpenSSH for the signed-roster method. This decision still governs a Tailscale mesh.

## Considered options

- **A Dojo listener on each node that reads the caller's grants through `tailscale whois`.** This would let the policy file express Dojo-specific permissions ("may hand over tasks but not run commands"). Rejected for now: it is a second long-running service to secure and keep alive on every machine, to gain a distinction nobody needs yet. It is the way to go if per-action permission is ever wanted.
- **Plain `ssh` with pinned host keys.** Rejected for now: it duplicates what Tailscale already proves. It remains the natural second membership method for meshes without Tailscale.

## Consequences

- A node's reach is whatever the policy grants it. The agent runs as the user and its tools take no user name, but nothing stops a process running as that user from calling `tailscale ssh root@…` itself where the policy allows it. The audit log, the origin mark and the pause switch are brakes and a record, not a boundary; the boundary is the policy.
- The origin mark is set by the caller and is advisory: a peer can log it but must not trust it.
- When the policy asks a person to sign in again (Tailscale's check mode) the call stops and exits 3 with the page to open, instead of waiting.
- A peer needs `dojo-mesh` installed to answer `describe`, and a `voice-agent` capability with a `command` to accept a task.
