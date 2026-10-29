# Notify on expiring mesh keys

Status: ready-for-agent
Type: task
Blocked by: 01

Replace the `eprintln!` in `src/mesh_renew.rs` with a notice, keeping the 24 hour dedupe per entry.
