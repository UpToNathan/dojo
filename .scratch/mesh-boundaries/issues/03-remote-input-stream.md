# Remote input stream

Type: task
Status: needs-triage
Blocked by: 01, 02

Carry pointer motion, buttons, scroll and keys from the sending node to the peer over a Tailscale SSH session kept warm while a boundary is configured (ADR-0006: no listening daemon). The receiving side identifies the sender through Tailscale (`tailscale whois` of the connection), refuses senders it does not accept and refuses while locked, and injects as the human seat. Losing the session, the release key and the lock screen return input home at once.
