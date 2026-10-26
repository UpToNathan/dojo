# Signed join request

Status: resolved

A join request was checked by the node key's fingerprint alone, so the host key or input key in an unsigned request file could be swapped in transit while the node key still matched. A request is now **signed by the node's own key** (`ssh-keygen -Y sign`, namespace `dojo-mesh-join`, through the agent or a private key file beside the public one): the signature covers every field in a fixed order, `approve` verifies it before showing anything, and the person still compares only the node key's fingerprint.

An earlier draft of this ticket added a short hash-based "request code". It was dropped: a signature by a key the operator is already comparing binds the same fields with the strongest primitive the machine already trusts (OpenSSH, Ed25519), needs no new dependency and no new human step.

## Acceptance

- The same request verifies on both machines; changing any field, signing with another key or for another namespace fails. (unit tests, real `ssh-keygen`)
- `approve` refuses edited, foreign-signed and unsigned (version 1) requests, with the way forward. (end-to-end)
- `join` signs through the agent when only the public half is on disk, and onboarding does it with 1Password. (end-to-end with a real agent)

## Comments

The panel's "request code" in the spec is the node key's fingerprint, as in the terminal.
