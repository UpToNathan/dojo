# Remote input over OpenSSH

Type: task
Status: resolved
Blocked by: 03

Generalize the gate in the Niri fork so a session under the root `sshd` from a non-local address is accepted in addition to Tailscale's helper, and so the sender's name comes from the dedicated input key's forced command. A process of the user outside such a session, or in one from this machine, is still refused. Amend ADR-0008; add process-table tests as for the Tailscale case. Needs the new Niri on both machines and a login.

## Resolution

`vendor/niri/src/remote_input.rs`: `sshd_origin` and `session_origin` (Tailscale first, then OpenSSH), with the remote address read from the session child's socket through `/proc/net/tcp*` and the name taken from the caller's `--as`. Process-table tests cover a real session, a user process posing as `sshd`, a session from this machine, another user's session, and no attributable connection; a second test reads sample `/proc/net/tcp` and `tcp6` text. ADR-0008 is amended. A connection arriving through a loopback proxy is refused by design. Not done: a live check against a real `sshd` session (none exists on this machine), and the new Niri is not installed anywhere yet, so it takes effect after install and login.
