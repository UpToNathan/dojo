# Enrollment and key custody

Type: task
Status: needs-triage
Blocked by: 02, 03

`dojo-mesh init`, `join`, `approve`, `renew`, `rm` and `leave`: create a signing key on a YubiKey (resident, PIN) or accept a 1Password key, show and compare fingerprints in both directions, sign entries, enroll a second signing key, and warn in `doctor` when there is only one or an entry is near expiry. Generate the node key and the dedicated input key in 1Password. Document the Jupiter setup (sign in to 1Password, turn on the SSH agent, choose the approval duration).
