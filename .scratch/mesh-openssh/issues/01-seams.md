# Roster, identity, channel and route seams

Type: task
Status: resolved
Blocked by: none

Split the mesh core into four seams with no behavior change: roster (members and routes), identity (who a connection is), channel (run, copy, status over a peer) and route (an address or a command). The Tailscale method becomes the only adapter behind them: tag membership and `tailscale status` for the roster, `tailscale whois` for identity, `tailscale ssh` for the channel. Existing tests pass unchanged; add a test double for each seam and cover the mesh of one with neither adapter present.

## Answer

Added `crates/dojo-mesh/src/method.rs` (the `Membership` and `Identity` traits, `Route`, `Connection`, and `Method` with `tailscale()` and `none()`) and `channel.rs` (the `Channel` trait and the call types moved out of `ssh.rs`). The Tailscale method in `tailscale.rs` fills all three: tag membership from `tailscale status --json`, identity from `tailscale whois` (moved out of `remote_cli.rs`), and the channel over `tailscale ssh`, which now refuses a command route or a port it cannot carry. `Env` carries the method, `call_peer` runs through its channel with a route, and the roster has `admit_from` and a `Source::Method(name)` for other methods. No behavior changed: every existing test passes unchanged (138 unit tests before, 144 now; all 36 end-to-end tests). Six new tests use a double for each seam and cover a node with no method, a call's route and provenance, offline and unknown peers never reaching the channel, a channel that cannot start, identity from the method, and the Tailscale channel's refusals.
