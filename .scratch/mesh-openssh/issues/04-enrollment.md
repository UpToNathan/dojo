# Enrollment and key custody

Type: task
Status: needs-triage
Blocked by: 02, 03

`dojo-mesh init`, `join`, `approve`, `renew`, `rm` and `leave`: use a 1Password-held signing key by default or, optionally, create one on a YubiKey (resident, PIN), show and compare fingerprints in both directions, sign entries, and warn in `doctor` when an entry is near expiry or the signing key is the software kind with remembered approvals. A second signing key is optional. Add `trust --new-signer` so a node, told locally and after a fingerprint compare, accepts a replacement signing key. Generate the node key and the dedicated input key in 1Password. Document the Jupiter setup (sign in to 1Password, turn on the SSH agent, choose the approval duration).
