# Roster, identity, channel and route seams

Type: task
Status: needs-triage
Blocked by: none

Split the mesh core into four seams with no behavior change: roster (members and routes), identity (who a connection is), channel (run, copy, status over a peer) and route (an address or a command). The Tailscale method becomes the only adapter behind them: tag membership and `tailscale status` for the roster, `tailscale whois` for identity, `tailscale ssh` for the channel. Existing tests pass unchanged; add a test double for each seam and cover the mesh of one with neither adapter present.
