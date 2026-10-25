---
status: accepted
---

# A signed roster is a second membership method, reached over OpenSSH

[ADR-0005](0005-tailscale-identity-is-node-identity-and-permission.md) and [ADR-0006](0006-peers-are-reached-with-tailscale-ssh.md) made Tailscale the mesh's identity, permission and transport, and both named a keys-and-host-keys method as the natural second one for meshes without Tailscale. This adds it, so a mesh works on any network where nodes can reach each other, and so the mesh no longer depends on Tailscale's SSH server, which runs as root and is the subject of an unconfirmed outside report (October 2026) of root-reaching flaws.

Three seams in the mesh core, each with the Tailscale method as one implementation and the new method as another: the **roster** (who the members are and how to reach them), **identity** (who a connection is), and the **channel** (running a command or copying a file on a peer). A **route** is how one node reaches a peer: an address and port, or a command that opens the connection (a `ProxyCommand`), so any network works, including a tunnel such as Cloudflare's, and nothing in the core assumes Tailscale.

## How the second method works

- **Roster.** One entry per device: its name, its node key, its host key, and an expiry. The roster also lists the operator signing keys and any revocations. It lives in a directory that can be a git repository on any remote. Every entry is signed by an operator signing key; nodes check the signature against the signing keys they hold locally and ignore anything unsigned, so whoever can write to the remote cannot add a member.
- **Operator signing keys.** A node trusts any number of signing keys, of any custody, and none is required: a hardware `ed25519-sk` key on a YubiKey (resident, PIN-protected, touched to sign), a key held in 1Password and used through its SSH agent (each signature needs the operator's approval), or any other key an SSH agent or key file can sign with. A key is identified by its public half; the custody is only how `ssh-keygen -Y sign` reaches the private half. Which one signs by default is a per-node preference order, so YubiKey first with 1Password as the alternate, the reverse, or one alone are all the same mesh. Enrollment, key rotation, renewal and revocation are signed; nothing else needs a signing key.
- **Trust on first sight is not allowed.** A new device shows its key fingerprint, the operator compares it by eye and approves it on a machine with the signing key, and the device confirms the signing key's fingerprint the same way.
- **Losing the signing key is recoverable.** Each node can be told locally, after the operator compares a fingerprint, to trust a new signing key. With a single signing key and no hardware key the mesh is never locked out for good.
- **Node keys.** One ed25519 key per node. Custody is that node's setting: 1Password's SSH agent by default, so each use needs the operator's approval as 1Password is set to ask, or any other SSH agent or key file. Receiving needs nothing: a node only checks `authorized_keys`.
- **Authorization.** A marked block in the user's own `authorized_keys` lists the members' node keys. A member gets that user's full shell, as in Hona/mesh. A second, dedicated key per node is restricted to `dojo-mesh inject --as <name>` and carries remote input. The mesh never edits `sshd_config`.
- **Connections.** The system `ssh` runs with a generated configuration kept under Dojo's state directory: strict host keys pinned from the roster, `BatchMode`, `IdentitiesOnly` with the 1Password agent, a short connect timeout and keepalives. The user's own `~/.ssh` is never read or changed. Provenance (`DOJO_FROM`, `DOJO_SESSION`, `DOJO_ORIGIN`) travels in the command as before.
- **Root.** There is no root over SSH. Root on a peer is the user's login plus `sudo`, which asks for the user's password. A node that has a YubiKey can add `pam_u2f` so each `sudo` also needs a touch; this is optional and per node.
- **A remote cannot take members out of view.** `roster sync` pulls fast-forward only and checks the result; an update that makes files stop verifying (unsigned, tampered, unreadable) is undone and reported, so the roster the node last believed stays in place. A revocation or an expiry is an ordinary change and is accepted.
- **Entries expire** (90 days by default) until re-signed, which bounds how long a revoked or stolen device stays trusted if a node is offline.
- **Remote input** ([ADR-0008](0008-remote-input-takes-the-real-seat-only-inside-a-peers-ssh-session.md)) accepts a session under the root `sshd` from a non-local address as well as one under Tailscale's helper. The sender's name comes from the dedicated key's forced command.

## Considered options

- **Hona/mesh as built.** A GitHub private repository as registry, member keys on disk, full shell. Taken for its shape; not its GitHub and `gh` dependency, and the keys live in 1Password instead.
- **A YubiKey touch for every connection.** Strongest, but it cannot run unattended and would block every task the agent hands over. Not used for ordinary connections.
- **An SSH certificate authority on the YubiKey.** Short-lived certificates would replace the key list and give revocation by expiry. Deferred: a signed roster with expiry gets most of it with less machinery. It stays open if distributing keys becomes a burden.
- **Forced commands for every member key.** Rejected by the operator for now: members get a full shell.
- **Node keys on disk.** Rejected by the operator: any process of the user could use them, the agent included.
- **Requiring one kind of signing key.** Rejected: custody is a setting. The mesh works the same with or without a YubiKey, and with any mix.
- **Keeping Tailscale as the only method.** The status quo. It stays as an optional method.

## Consequences

- Anything the agent starts on a peer waits for 1Password's approval on the initiating machine. That is stricter than the permissive agent policy of ADR-0005 and ADR-0006; the audit log and pause switch remain.
- A node whose 1Password is locked, or not signed in, cannot start connections, though it still receives them. A reboot locks it again.
- Revocation is a signed roster change that nodes must pull, not one switch in a tailnet policy. Expiry bounds the delay.
- A signing key held in 1Password is gated by a software approval, which is weaker than a hardware touch: if 1Password is set to remember approvals for a period, any process of the user can sign during that period. Set the signing key to ask every time. `doctor` shows how each signing key is held, in order, and warns when the only ones are the software kind.
- Each node runs OpenSSH and the operator owns its configuration. A node's `sshd` must accept public keys only and listen only where intended; this is a setup step, not something the mesh changes.
- ADR-0005 and ADR-0006 stay in force for Tailscale meshes. ADR-0008's gate is amended to cover both transports.
- Accepted once the method was built and tested with real `ssh-keygen` keys, a real git remote and a fake `ssh`. What has not been exercised is a live 1Password agent, a YubiKey, and a real `sshd` session reaching Niri's gate; the tickets in `.scratch/mesh-openssh` list each.
