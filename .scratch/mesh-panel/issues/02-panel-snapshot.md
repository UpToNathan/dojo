# Panel snapshot in dojo-mesh

Status: open

Blocked by: 01

`dojo_mesh::panel::read()` returns what the panel draws and nothing it has to compute: this node, a card per member (name, online, days to expiry, route text, state), health findings from doctor with a count, whether the key agent is ready, the trusted signing keys (fingerprint, hardware or software), and the pending requests found in the inbox and Downloads (name, code, file, routes). Pure where it can be, with the file reads at the edge.

## Acceptance

- Works under both methods; the roster-only parts are empty under Tailscale.
- A request file that is oversized, malformed or a link is skipped, not listed.
- Tests with temporary directories.
