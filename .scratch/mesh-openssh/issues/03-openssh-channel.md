# OpenSSH channel

Type: task
Status: resolved
Blocked by: 01, 02

Implement the channel and identity seams over the system `ssh`: the generated configuration and pinned host keys from the roster, `BatchMode`, `IdentitiesOnly` with the 1Password agent and the node's public key, timeouts and keepalives, the marked `authorized_keys` block (member node keys, dedicated input keys with a forced command), and `exec`, `cp`, `status`, send and `task` over it with provenance. A locked or missing agent exits 3 with the step to take. Tests use a fake `ssh` and fixture keys.

## Resolution

Done in `sshchannel.rs`, `authorize.rs` and the roster method's identity in `openssh.rs`: generated `ssh` options with `-F /dev/null`, `BatchMode`, `IdentitiesOnly` with the public key file and the agent socket, pinned host keys by alias, timeouts, keepalives, node key and input key roles, an agent preflight that exits 3 with the step to take (`doctor` warns the same), the marked `authorized_keys` block (`roster authorize`, run after `roster sync`), and an identity taken from the input key's forced command and checked against the roster. Tests use fake `ssh` and `ssh-add` and real `ssh-keygen` keys. Not done here: nothing was run against a real `sshd` (none is installed on this machine).
