# Enrollment and control plane

Type: task
Status: resolved
Blocked by: 01

Operator-only commands that mint a tagged auth key to enrol a new node and propose policy changes after validating them. Credentials come from a secret reference.

## Answer

`dojo-mesh enroll` mints a one-use pre-approved key for a tag, and `policy show | validate | apply` read, check and replace the policy. `apply` validates with Tailscale first, shows a diff, asks, and writes with the ETag so a concurrent change is refused. Both writes refuse unless a person is at a terminal and the call is not the agent's. Credentials are named by reference (`env:`, `file:`, `op://`), read for one command and passed to curl on stdin. Verified live read-only with the OAuth client from 1Password: sign-in, policy read, validation of the real policy, and the refusal to apply without a terminal. Key creation and policy replacement were exercised only against fakes.
