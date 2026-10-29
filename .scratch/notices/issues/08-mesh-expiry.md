# Notify on expiring mesh keys

Status: resolved
Type: task
Blocked by: 01

Replace the `eprintln!` in `src/mesh_renew.rs` with a notice, keeping the 24 hour dedupe per entry.

## Result

`src/mesh_renew.rs` posts a Normal `mesh-expiry` notice ("Mesh key expiring") through `dojo::notify::post`, with the 24 hour dedupe unchanged and the `eprintln!` kept as a log line. The body is the message from `dojo-mesh renew --due`; that message used to be only a count, so `crates/dojo-mesh/src/enroll_cli.rs` now names each due entry and its days left (`describe_due`, e.g. "beta expires in 9 days"), and `tests/roster_e2e.rs` was adjusted to match.

Verified: `cargo test --bin dojo mesh_renew` (notice kind, summary, body, urgency), `cargo test --bin dojo mesh`, and `cargo test` in `crates/dojo-mesh` (all pass, including the new `describe_due` unit test).
