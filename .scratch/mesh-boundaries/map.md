# Mesh boundaries

## Notes

Join screen edges across nodes for input, and send windows to a peer with a key. See [the spec](spec.md).

## Decisions-so-far

- The limited set of applications makes adapters per application enough; live migration (wprs and the like) is out.
- A handoff is triggered by a key (a direction across a boundary, or a peer picker), not by dragging. It needs no compositor change, so it can ship before remote input.
- A handed-off window closes locally: it is a move, not a copy.
- A file the peer lacks refuses the handoff, with an offer to copy it across first.
- No dependency on tmux: a shell starts fresh in the same directory.
- Input from a peer is the human's input on the receiving node, and is accepted only from peers that node names ([ADR-0007](../../docs/adr/0007-a-node-accepts-input-only-from-peers-it-names.md)).
- Focus follows the mouse; the touchpad is ignored while typing (`dwt`, 2026-10-24) so a bumped palm does not move focus.
- kitty remote control is password-only, with the adapter limited to `ls` and its password kept out of the Nix store.

## Fog

- The clipboard while input is on a peer: follow the pointer, sync on crossing, or keep per-node clipboards with an explicit send. Deferred by the operator as worth its own design.
- The key that always returns input home, and whether it can be one of the global shortcuts.
- Latency of the input stream over Tailscale SSH: whether a warm session kept while a boundary is configured is enough, or a lighter channel is needed.
- The final keys for sending a window (Mod+Alt+Shift+arrows and Mod+Alt+Shift+M are free today).
- Whether dragging across a boundary should also hand off, once remote input exists.
- The Displays panel showing peers' outputs beside local ones.
