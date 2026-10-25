# Taking the cursor and keyboard

Type: task
Status: needs-triage
Blocked by: 01, 03

Niri fork: capture all local input on request (hide the cursor, deliver nothing locally, stream events) and inject a peer's input as the human seat. A stream over Tailscale SSH carries it; the receiver checks `accept-input` against `tailscale whois`. A release key, a lost connection and either side locking return input home.
