# Signed roster

Type: task
Status: resolved
Blocked by: 01

The roster format (devices, signing keys, revocations, expiry), its directory layout and its verification with `ssh-keygen -Y verify` against the node's local allowed signers. Unsigned, tampered, expired, revoked and unknown-signer entries are ignored and shown by `doctor` with the reason. Sync from an optional git remote. Tests use generated ed25519 keys, including a signer that is not trusted and a roster whose remote was changed.

## Answer

Added `signed.rs` (the pure core: entry and revocation types, name and key validation, `evaluate` with the order of checks and a `Verifier` trait) and `openssh.rs` (the adapter: `SshKeygen` verifier using `ssh-keygen -Y verify`, the directory loader that ignores symlinks and oversize files, a TCP reachability probe, `SignedMembership`, and `git pull --ff-only` sync). New config keys `method`, `roster-dir`, `signers-file` and `node-name`; a `roster check | sync` command; `Device.routes`, `Roster.rejected` and `Admission::Signed`; `doctor` and `status` understand a roster source (no tag advice, ignored entries listed). 12 new unit tests with a fake verifier and 7 end-to-end tests with real `ssh-keygen` keys, a real bare git remote and the real binary: unsigned, tampered, untrusted-signer, expired, future, revoked, superseded and malformed entries, a remote that adds a member or rewrites one, a rewritten history, a missing signers file, deny, and configuration errors. Key fields are checked to be exactly one key of a known type so nothing can carry options into a later `authorized_keys` line.

Known limit, to settle with ticket 4: a remote with write access can remove members from view by corrupting their entries (the node then ignores them), though it can never add or change one. Keeping the last verified roster would make that a delay instead of an outage.
