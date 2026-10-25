---
status: accepted
---

# Tailscale identity is a node's identity and permission

A node admits a peer when Tailscale reports that the device carries the mesh's membership tag, minus or plus the node's own membership overrides, and trusts Tailscale's word for which device is connecting. Dojo adds no authentication of its own, and what a node may do on a peer is exactly what the tailnet's policy grants it: the policy file is the mesh's permission system. This keeps one source of truth the operator already edits and audits, rather than a second set of keys and rules that can drift from it.


[ADR-0009](0009-a-signed-roster-is-a-second-membership-method-over-openssh.md) adds the signed-roster method for meshes without Tailscale. This decision still governs a Tailscale mesh.

## Considered options

- **Pinned SSH host keys per peer, alone or required alongside the tag.** Not now: it duplicates what Tailscale already proves, and every new node would need a key exchange. It stays available as a later membership method for meshes without Tailscale; nothing in the mesh core may assume Tailscale.
- **A Dojo-side permission layer (roles per peer).** Rejected for now: two permission systems disagree, and the stricter one is never the one people look at.

## Consequences

- Permission is per node, not per process. An agent on a node can reach whatever that node can, including root SSH where the policy grants it. As of 2026-10-24 the operator's policy still grants root SSH between all tagged nodes, by choice; tightening it is a policy edit, not a Dojo change.
- Nothing in tracked source names a tailnet, tag or machine. The membership tag and overrides are node-local configuration.
- Changing the policy file is an operator action. The agent may read it and propose changes, never write it.
