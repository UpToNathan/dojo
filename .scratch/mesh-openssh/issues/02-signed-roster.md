# Signed roster

Type: task
Status: needs-triage
Blocked by: 01

The roster format (devices, signing keys, revocations, expiry), its directory layout and its verification with `ssh-keygen -Y verify` against the node's local allowed signers. Unsigned, tampered, expired, revoked and unknown-signer entries are ignored and shown by `doctor` with the reason. Sync from an optional git remote. Tests use generated ed25519 keys, including a signer that is not trusted and a roster whose remote was changed.
