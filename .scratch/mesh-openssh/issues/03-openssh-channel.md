# OpenSSH channel

Type: task
Status: needs-triage
Blocked by: 01, 02

Implement the channel and identity seams over the system `ssh`: the generated configuration and pinned host keys from the roster, `BatchMode`, `IdentitiesOnly` with the 1Password agent and the node's public key, timeouts and keepalives, the marked `authorized_keys` block (member node keys, dedicated input keys with a forced command), and `exec`, `cp`, `status`, send and `task` over it with provenance. A locked or missing agent exits 3 with the step to take. Tests use a fake `ssh` and fixture keys.
