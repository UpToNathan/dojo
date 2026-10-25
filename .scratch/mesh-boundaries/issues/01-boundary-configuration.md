# Boundary configuration

Type: task
Status: needs-triage

Add `[[boundary]]` (output, edge, peer, peer-output, peer-edge) and `accept-input` to the mesh configuration, published in the node's description. `dojo-mesh doctor` reports one-sided boundaries, unknown outputs and boundaries the peer does not accept. Pure parsing and matching, tested without a network.
