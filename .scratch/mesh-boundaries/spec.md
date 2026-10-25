# Mesh boundaries

Let the operator join the edges of their nodes' screens, so the pointer and keyboard can cross from one node to a peer, and a window dragged across the edge is handed off to the peer. Terms are in [GLOSSARY.md](../../GLOSSARY.md#mesh); this builds on the [mesh](../mesh/spec.md) and on [ADR-0007](../../docs/adr/0007-a-node-accepts-input-only-from-peers-it-names.md).

## Requirements

### Boundaries

- A node declares its boundaries in its own mesh configuration: an edge of one of its outputs, the peer it leads to, and the peer's output and edge. Nothing names a machine in tracked source.
- A boundary only works when the peer declares the matching boundary back and accepts input from this node. `dojo-mesh doctor` reports a boundary that is one-sided, points at an output the peer does not have, or is not accepted.
- With no boundaries, or when the peer is offline, an edge is an ordinary screen edge. Crossing never waits on the network: a peer that cannot be reached at the moment of crossing leaves the pointer where it is.

### Remote input

- Pushing the pointer past a boundary hands the pointer and keyboard to the peer, entering at the matching point of the peer's edge. Pushing back across the peer's matching edge returns them. The local cursor is hidden while away and keystrokes do not reach local windows.
- On the peer, the input is the human's own input, not an agent's: it focuses, types and clicks as if the operator sat there.
- A node accepts input only from peers it names (`accept-input`), and checks who is sending from Tailscale's identity of the connection, never from what the sender says about itself.
- A configurable key always returns input to the local node, and losing the connection returns it at once. Input never stays captured by a peer that stopped answering.
- The lock screen ends remote input on both sides, and a locked peer refuses it.

### Handoff

- Releasing a window past a boundary while moving it hands it off: the peer opens what the window was showing, on the peer's output near where the pointer crossed, and the local window closes. Remote input moves to the peer with it.
- What moves is decided per application by a handoff adapter. The first adapters are the browser (the tab, later the window's tabs), the terminal (its directory, and the foreground program when it is one with its own adapter), nvim (file, line and column), yazi (directory) and the Dojo agent's chat.
- A running process never moves. A terminal's shell starts fresh on the peer; what ran in the old one ends with the window. The status line says so whenever something did not carry over.
- A window with no adapter is relaunched on the peer from its desktop entry, and the status line says it was relaunched, not moved.
- A file the peer does not have refuses the handoff, leaves the window where it was, and offers to copy the file across first. Copying is the operator's choice, made each time; afterwards the two copies are independent. A terminal whose directory the peer lacks opens in the nearest directory that does exist.
- Every handoff is in the mesh audit log, like any other call to a peer.

## Out of scope for now

- Moving running processes, or live windows (portable sessions such as wprs).
- The clipboard while input is on a peer.
- Showing peers' screens in the Displays panel.
