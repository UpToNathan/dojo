# Mesh

Make every Dojo node able to see, reach and use its peers, while a single node with no peers stays a complete Dojo. Terms are in [GLOSSARY.md](../../GLOSSARY.md#mesh); identity and permission follow [ADR-0005](../../docs/adr/0005-tailscale-identity-is-node-identity-and-permission.md).

## Requirements

- **Mesh of one.** Every mesh feature works, or reports plainly that there are no peers, on a node with no peers and without Tailscale installed.
- **No one's machines in the source.** No tailnet, tag, hostname or user name appears in tracked source outside test fixtures, which use invented names. A check enforces it. The operator's own setup may live under `examples/`.
- **Membership.** The first membership method is Tailscale tag membership: candidates are devices the local Tailscale client reports with the mesh's membership tag. A node-local allow or deny list overrides the result for that node. Reading membership needs no Tailscale API credentials.
- **Permission.** Dojo does not check permissions itself. Whatever the tailnet policy lets a node do on a peer, the node may do; whatever it refuses fails with the network's refusal, shown as such.
- **Pure core.** Peers, membership decisions, capabilities and offline handling live in the library crate without Tailscale, SSH or process code, and are tested as plain data. Tailscale, SSH and the API are adapters at the edge.
- **Shell layer first.** `status`, `exec` and `cp` (as `dojo-mesh` subcommands) work from the terminal and the launcher before the agent uses them. Calls carry a JSON envelope on stdin, provenance (originating node and session), fail fast on an offline peer, and exit 3 when a human is needed, following [Hona/mesh](https://github.com/Hona/mesh).
- **Capabilities.** A node describes what it offers, in machine-readable form. Consumers ask for a capability, never for a named peer.
- **Agent.** The agent reaches peers only through the shell layer. Access is permissive, with an audit log, provenance on every remote call and a pause switch. Desktop work on a peer is a peer task for that peer's agent.
- **Control plane.** Writing to the network (tags, auth keys, the policy file) is operator-only, reads its credentials from a secret reference such as a 1Password item, and validates a policy before proposing it.

## Out of scope for now

- Membership methods other than Tailscale tags.
- A Dojo-side permission layer.
- Tightening the operator's current root SSH policy.
