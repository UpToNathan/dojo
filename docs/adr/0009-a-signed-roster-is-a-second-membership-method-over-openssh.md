---
status: proposed
---

# A signed roster is a second membership method, reached over OpenSSH

[ADR-0005](0005-tailscale-identity-is-node-identity-and-permission.md) and [ADR-0006](0006-peers-are-reached-with-tailscale-ssh.md) made Tailscale the mesh's identity, permission and transport, and both named a keys-and-host-keys method as the natural second one for meshes without Tailscale. This adds it, so a mesh works on any network where nodes can reach each other, and so the mesh no longer depends on Tailscale's SSH server, which runs as root and is the subject of an unconfirmed outside report (October 2026) of root-reaching flaws.

Three seams in the mesh core, each with the Tailscale method as one implementation and the new method as another: the **roster** (who the members are and how to reach them), **identity** (who a connection is), and the **channel** (running a command or copying a file on a peer). A **route** is how one node reaches a peer: an address and port, or a command that opens the connection (a `ProxyCommand`), so any network works, including a tunnel such as Cloudflare's, and nothing in the core assumes Tailscale.

## How the second method works

- **Roster.** One entry per device: its name, its node key, its host key, and an expiry. The roster also lists the operator signing keys and any revocations. It lives in a directory that can be a git repository on any remote. Every entry is signed by an operator signing key; nodes check the signature against the signing keys they hold locally and ignore anything unsigned, so whoever can write to the remote cannot add a member.
- **Operator signing key.** A hardware `ed25519-sk` key on a YubiKey, resident and PIN-protected, touched to sign; a key held in 1Password is also accepted. Two are enrolled from the start. Enrollment, key rotation, renewal and revocation are signed; nothing else needs the YubiKey.
- **Trust on first sight is not allowed.** A new device shows its key fingerprint, the operator compares it by eye and approves it on a machine with the signing key, and the device confirms the signing key's fingerprint the same way.
- **Node keys.** One ed25519 key per node, held in 1Password's SSH agent, so each use of it needs the operator's approval as 1Password is set to ask. Receiving needs nothing: a node only checks `authorized_keys`.
- **Authorization.** A marked block in the user's own `authorized_keys` lists the members' node keys. A member gets that user's full shell, as in Hona/mesh. A second, dedicated key per node is restricted to `dojo-mesh inject --peer <name>` and carries remote input. The mesh never edits `sshd_config`.
- **Connections.** The system `ssh` runs with a generated configuration kept under Dojo's state directory: strict host keys pinned from the roster, `BatchMode`, `IdentitiesOnly` with the 1Password agent, a short connect timeout and keepalives. The user's own `~/.ssh` is never read or changed. Provenance (`DOJO_FROM`, `DOJO_SESSION`, `DOJO_ORIGIN`) travels in the command as before.
- **Root.** There is no root over SSH. Root on a peer is the user's login plus `sudo` with `pam_u2f`, which needs a YubiKey touch each time.
- **Entries expire** (90 days by default) until re-signed, which bounds how long a revoked or stolen device stays trusted if a node is offline.
- **Remote input** ([ADR-0008](0008-remote-input-takes-the-real-seat-only-inside-a-peers-ssh-session.md)) accepts a session under the root `sshd` from a non-local address as well as one under Tailscale's helper. The sender's name comes from the dedicated key's forced command.

## Considered options

- **Hona/mesh as built.** A GitHub private repository as registry, member keys on disk, full shell. Taken for its shape; not its GitHub and `gh` dependency, and the keys live in 1Password instead.
- **A YubiKey touch for every connection.** Strongest, but it cannot run unattended and would block every task the agent hands over. Reserved for enrollment and root.
- **An SSH certificate authority on the YubiKey.** Short-lived certificates would replace the key list and give revocation by expiry. Deferred: a signed roster with expiry gets most of it with less machinery. It stays open if distributing keys becomes a burden.
- **Forced commands for every member key.** Rejected by the operator for now: members get a full shell.
- **Node keys on disk.** Rejected by the operator: any process of the user could use them, the agent included.
- **Keeping Tailscale as the only method.** The status quo. It stays as an optional method.

## Consequences

- Anything the agent starts on a peer waits for 1Password's approval on the initiating machine. That is stricter than the permissive agent policy of ADR-0005 and ADR-0006; the audit log and pause switch remain.
- A node whose 1Password is locked, or not signed in, cannot start connections, though it still receives them. A reboot locks it again.
- Revocation is a signed roster change that nodes must pull, not one switch in a tailnet policy. Expiry bounds the delay.
- Losing every YubiKey locks the operator out of enrollment and root, so a second one is part of setup.
- Each node runs OpenSSH and the operator owns its configuration. A node's `sshd` must accept public keys only and listen only where intended; this is a setup step, not something the mesh changes.
- ADR-0005 and ADR-0006 stay in force for Tailscale meshes. ADR-0008's gate is amended to cover both transports.
