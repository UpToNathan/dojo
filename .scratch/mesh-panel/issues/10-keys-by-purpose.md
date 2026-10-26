# Keys by purpose, managed from the panel

Status: open

`dojo-mesh keys` lists the keys this node signs with (in order, with a kind) and the keys it trusts, and moves, removes and adds them, writing `signing-keys` and the trusted keys itself. The panel's Keys view uses it: order with up and down, remove, add (1Password key, security key, key file). Kinds, not fingerprints.

## Acceptance

- The configuration is changed in place and nothing else in it is touched; a signing key that does not exist is refused.
- Adding a security key asks for its PIN through a small Dojo dialog (an askpass), never a terminal.
