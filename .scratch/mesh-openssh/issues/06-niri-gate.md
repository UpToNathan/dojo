# Remote input over OpenSSH

Type: task
Status: needs-triage
Blocked by: 03

Generalize the gate in the Niri fork so a session under the root `sshd` from a non-local address is accepted in addition to Tailscale's helper, and so the sender's name comes from the dedicated input key's forced command. A process of the user outside such a session, or in one from this machine, is still refused. Amend ADR-0008; add process-table tests as for the Tailscale case. Needs the new Niri on both machines and a login.
