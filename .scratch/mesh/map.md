# Mesh

## Notes

Make Dojo multi-node without tying it to one person's machines. See [the spec](spec.md).

## Decisions-so-far

- Tailscale identity is enough to admit a peer, and the tailnet policy is the permission system ([ADR-0005](../../docs/adr/0005-tailscale-identity-is-node-identity-and-permission.md)).
- Membership is the membership tag plus discovery, with a node-local allow or deny override.
- Root SSH between tagged nodes stays as it is for now (operator's choice, 2026-10-24). The agent therefore inherits it.
- Control-plane writes are operator-only.

## Fog

- Transport: Tailscale SSH, plain SSH, or a Dojo listener that reads the caller's grants through Tailscale's `whois` (application capabilities in the policy would let the policy file express Dojo-specific permissions).
- How a node publishes its capabilities: on demand over the transport, or as something peers can read without a call.
- Where node-local mesh configuration lives and its format.
- How the agent's no-root rule is enforced while the policy still grants root SSH.
- A second membership method for people without Tailscale.
