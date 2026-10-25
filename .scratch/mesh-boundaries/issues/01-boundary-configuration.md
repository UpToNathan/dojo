# Boundary configuration

Type: task
Status: resolved

Add `[[boundary]]` (output, edge, peer, peer-output, peer-edge) and `accept-input` to the mesh configuration, published in the node's description. `dojo-mesh doctor` reports one-sided boundaries, unknown outputs and boundaries the peer does not accept. Pure parsing and matching, tested without a network.

## Answer

`[[boundary]]` (output, edge, peer, peer-output, peer-edge) and `accept-input` are parsed and validated at the boundary (edges, empty names, one boundary per edge). `dojo-mesh describe` now publishes this node's outputs (from `niri msg --json outputs`, absent without a compositor), boundaries and `accept-input`. `dojo-mesh doctor` asks the peer of each boundary and reports a missing or mismatched boundary back (naming exactly what the peer must declare), unknown outputs, a peer outside the mesh, and input not accepted at either end; an offline or unreadable peer is a warning. Pure checks have unit tests; the flows run end to end against the fake network. Documented in [docs/mesh.md](../../../docs/mesh.md).
