---
status: proposed
---

# A node accepts input only from peers it names, and treats it as the human's

Remote input (the operator's pointer and keyboard taken along when a window is sent to a peer) is more powerful than running a command on a peer: it can type into any window, including terminals with root shells and the browser with signed-in sessions. Membership in the mesh is therefore not enough to send it. A node accepts input only from peers listed in its own `accept-input`, and learns which peer is sending from Tailscale's identity of the connection (`tailscale whois`), never from the sender's own claim such as `DOJO_FROM`. Accepted input is injected as the human's seat, because it is the operator at another keyboard, not an agent.

## Considered options

- **Any mesh member may send input.** Rejected: one compromised peer could type into every node.
- **Inject as an agent seat.** Rejected: agent seats are built to yield to the human and stay out of the human's focus, which is the opposite of what someone at the other keyboard needs.

## Consequences

- The sender's identity comes from the network, so the check holds even though the provenance variables of ADR-0006 are advisory.
- A locked node refuses input, so crossing into a locked peer does nothing until it is unlocked there.
- The agent gets no tool to send remote input; its way to act on a peer remains a peer task.
