# Sending windows to another machine

## Notes

Send a window to a peer with a key, optionally taking the cursor and keyboard along. See [the spec](spec.md).

## Decisions-so-far

- No screen edges between machines: a key opens a panel (2026-10-25).
- The key is the operator's choice, made through Dojo's own global shortcut and binding panel; Dojo only detects it.
- The limited set of applications makes adapters per application enough; live migration is out.
- A sent window closes locally: it is a move, not a copy.
- A file the peer lacks stops the send, with an offer to copy it across first.
- No dependency on tmux: a shell starts fresh in the same directory.
- Taking the cursor and keyboard is optional per send; input from a peer is the human's input, accepted only from peers the node names ([ADR-0007](../../docs/adr/0007-a-node-accepts-input-only-from-peers-it-names.md)).
- kitty remote control is password-only, with the adapter limited to `ls` and its password kept out of the Nix store.

## Fog

- The clipboard while input is on a peer, deferred by the operator as worth its own design.
- Latency of the input stream over Tailscale SSH.
