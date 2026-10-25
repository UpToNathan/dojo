# Enrollment and key custody

Type: task
Status: resolved
Blocked by: 02, 03

`dojo-mesh init`, `join`, `approve`, `renew`, `rm` and `leave`: register signing keys of any custody (a YubiKey created resident with a PIN, a 1Password key, any agent or file key) with a preference order, sign through `ssh-keygen -Y sign` whichever holds the key, show and compare fingerprints in both directions, sign entries, and warn in `doctor` when an entry is near expiry or the signing key is the software kind with remembered approvals. More than one signing key is optional. Add `trust --new-signer` so a node, told locally and after a fingerprint compare, accepts a replacement signing key. Generate the node key and the dedicated input key in 1Password. Document the Jupiter setup (sign in to 1Password, turn on the SSH agent, choose the approval duration).

## Resolution

Done in `enroll.rs` and `enroll_cli.rs`: `init` (1Password through `op`, or `--custody existing`), `join`, `approve` (fingerprint required; exit 3 shows it), `renew`, `rm` (removes the entry, revokes node, input and host keys), `leave`, `trust list|add|remove` (the replacement signing key is `trust add` with a compared fingerprint), `signing-keys` tried in order through `ssh-keygen -Y sign`, and a `doctor` warning when every trusted signer is held in software. Tests use real `ssh-keygen` keys, a real git repository and a fake `op`. Not done: the live check against 1Password and a YubiKey (neither is available to test here), and keeping the last verified roster against a remote that corrupts entries; both are noted for ticket 7. The Jupiter steps (sign in to 1Password, turn on the SSH agent, choose the approval duration) are in docs/mesh.md and still need the person at the machine.
