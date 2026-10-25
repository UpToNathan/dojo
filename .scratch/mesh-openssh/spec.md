# Mesh over OpenSSH

A second way to form a mesh that needs no Tailscale: a signed roster of members, reached with the system OpenSSH over any route. Decision and reasons are in [ADR-0009](../../docs/adr/0009-a-signed-roster-is-a-second-membership-method-over-openssh.md). Terms are in [GLOSSARY.md](../../GLOSSARY.md#mesh). The first mesh work is in [.scratch/mesh](../mesh/spec.md); Tailscale stays a supported method.

## Requirements

- **Method-neutral core.** Roster, identity, channel and route are seams in the library crate. The Tailscale method and the OpenSSH method are adapters. The core, its config and its documentation name no network. A mesh of one works with neither.
- **Signed roster.** Entries are accepted only when signed by an operator signing key the node already holds. Unsigned, tampered, expired and revoked entries are ignored and reported by `doctor`. The roster directory may be a git repository on any remote; the mesh does not require one, GitHub, or `gh`.
- **Enrollment is deliberate.** A new device is trusted only after the operator compares its fingerprint, and the device confirms the signing key's fingerprint. Signing needs the YubiKey (touch and PIN) or a 1Password-held signing key. Two signing keys are enrolled at setup; `doctor` warns when there is one.
- **Node keys in 1Password.** Connections use the node key through 1Password's SSH agent. The generated configuration names the key by its public half and never touches `~/.ssh`. A locked or missing agent gives a plain "unlock 1Password on this machine" message and exit 3, not a hang.
- **Authorization.** A marked, removable block in the user's `authorized_keys` lists the members' node keys (full user shell) and each member's dedicated input key (restricted to `dojo-mesh inject --peer <name>`). `leave` removes the block. The mesh never edits `sshd_config`.
- **Root.** No root over SSH. Root on a peer is `sudo` with a YubiKey touch via `pam_u2f`, set up by the repository's system-config scripts.
- **Routes.** A peer's route is an address and port or a `ProxyCommand`, from the roster or a node-local override. Offline or unreachable peers fail fast, as today.
- **Remote input.** The Niri gate accepts a session under the root `sshd` from a non-local address in addition to Tailscale's, and takes the sender's name from the dedicated key's forced command. [ADR-0008](../../docs/adr/0008-remote-input-takes-the-real-seat-only-inside-a-peers-ssh-session.md) is amended.
- **Tests without the real tools.** Roster verification, expiry, revocation, configuration generation and the gate run against fixture keys and a fake `ssh`, with no YubiKey, 1Password or network.

## Out of scope for now

- An SSH certificate authority.
- A Cloudflare route (the route seam allows it later).
- Forced commands for ordinary members.
- Changing a node's `sshd_config` from the mesh. Hardening Jupiter's `sshd` is a separate host fix.
