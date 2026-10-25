# Mesh

## Notes

Make Dojo multi-node without tying it to one person's machines. See [the spec](spec.md).

## Decisions-so-far

- Tailscale identity is enough to admit a peer, and the tailnet policy is the permission system ([ADR-0005](../../docs/adr/0005-tailscale-identity-is-node-identity-and-permission.md)).
- Membership is the membership tag plus discovery, with a node-local allow or deny override.
- Root SSH between tagged nodes stays as it is for now (operator's choice, 2026-10-24). The agent therefore inherits it.
- Control-plane writes are operator-only.

- [Read-only mesh](issues/01-read-only-mesh.md), [shell layer](issues/02-shell-layer.md), [capabilities](issues/03-capabilities.md), [agent peers](issues/04-agent-peers.md) and [control plane](issues/05-control-plane.md) are implemented. Peers are reached with `tailscale ssh` ([ADR-0006](../../docs/adr/0006-peers-are-reached-with-tailscale-ssh.md)); there is no Dojo daemon between nodes.

## Fog

- A Dojo listener that reads the caller's grants through `tailscale whois` would let the policy file express Dojo-specific permissions (ADR-0006 keeps it as the way forward if per-action permission is wanted).
- A shared membership tag does not exist on the operator's tailnet yet; nodes use allow lists until one is created and applied (`dojo-mesh enroll`, `policy apply`).
- A second membership method for people without Tailscale (pinned host keys over plain SSH).
- Enforcing the agent's no-root rule beyond its tool surface, while the policy grants root SSH between nodes.
- Peers' Tailscale client versions are not in `tailscale status`, so `doctor` cannot report drift.
- A peer task is sent to the peer's active chat; it is not isolated into a chat of its own.
