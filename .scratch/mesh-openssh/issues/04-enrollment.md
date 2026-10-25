# Enrollment and key custody

Type: task
Status: needs-triage
Blocked by: 02, 03

`dojo-mesh init`, `join`, `approve`, `renew`, `rm` and `leave`: register signing keys of any custody (a YubiKey created resident with a PIN, a 1Password key, any agent or file key) with a preference order, sign through `ssh-keygen -Y sign` whichever holds the key, show and compare fingerprints in both directions, sign entries, and warn in `doctor` when an entry is near expiry or the signing key is the software kind with remembered approvals. More than one signing key is optional. Add `trust --new-signer` so a node, told locally and after a fingerprint compare, accepts a replacement signing key. Generate the node key and the dedicated input key in 1Password. Document the Jupiter setup (sign in to 1Password, turn on the SSH agent, choose the approval duration).
