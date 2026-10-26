# Sending windows to another machine

Let the operator send a window to another node of the mesh with a key: the peer opens what the window was showing, the local window closes, and the operator's cursor and keyboard can go along to keep working there. Terms are in [GLOSSARY.md](../../GLOSSARY.md#mesh); this builds on the [mesh](../mesh/spec.md) and [ADR-0007](../../docs/adr/0007-a-node-accepts-input-only-from-peers-it-names.md).

## Requirements

### The key and the panel

- Dojo registers its own global shortcut, "Send a window to another machine", with no key of its own. The operator chooses the key in the usual binding panel, the first time and from its settings; Dojo only reacts to it. `dojo --send-window` opens the same panel, for scripts and binds in Niri's config.
- The key opens a panel at the bottom centre of the focused output, over everything, with the keyboard. It lists:
  - the windows of this node, the focused one selected, so another can be chosen;
  - the peers that are online and can receive a window, the last one used selected (or the only one);
  - a switch "Take my cursor and keyboard", remembered between uses.
- Enter sends, Escape closes, Tab moves between the lists, Space flips the switch. With nobody online the panel says so and sends nothing.

### Sending

- What is sent is decided per application by a handoff adapter. The first adapters are the browser (the tab's page), kitty (its directory, and the foreground program when it has an adapter), nvim (file, line and column), yazi (directory) and the Dojo agent's chat.
- A window with no adapter is relaunched on the peer from its desktop entry, and the panel says it was relaunched, not moved.
- The peer opens the window in its own session, on its focused output. Once the peer reports that it opened, the local window closes. If the peer fails, the local window stays and the panel says why.
- A running process never moves. A terminal's shell starts fresh on the peer; what ran in the old one ends with the window. The panel says what did not carry over.
- A file the peer does not have stops the send, leaves the window, and offers to copy the file across first. Copying is the operator's choice each time; afterwards the two copies are independent. A terminal whose directory the peer lacks opens in the nearest directory that does exist.
- The receiving side only opens the kinds of thing adapters describe (a page, a directory, a file at a line, a chat, a desktop entry it has itself). It never runs a command it is sent.
- Every send is in the mesh audit log.

### Taking the cursor and keyboard

- With the switch on, sending also hands the pointer and keyboard to the peer: the local cursor is hidden, local windows get no input, and everything typed or pointed goes to the peer as the operator's own input there.
- Super+Alt+F11 and Ctrl+Alt+Shift+Esc always bring input home, and neither reaches the peer; so does losing the connection, the peer refusing, or either side locking. Input never stays captured by a peer that stopped answering.
- A node accepts this input only from peers it names in `accept-input`, and knows who is sending from Tailscale's identity of the connection, never from what the sender says (ADR-0007).

### Terminal remote control

The kitty adapter reads kitty's windows through kitty's remote control, which must not become a way in:

- kitty runs remote control in `password` mode, so neither its socket nor the terminal itself (escape sequences from a program inside kitty, including over SSH) can control it without a password. A request without a known password makes kitty ask the operator.
- The adapter's password allows `ls` and nothing else.
- The password is generated on each machine when its configuration is applied, kept outside the Nix store in a file only the user can read, and included from kitty's configuration. It is never tracked.
- kitty listens on a socket in `$XDG_RUNTIME_DIR`, never an abstract socket.
- Opening a terminal on the peer starts a new kitty there; it needs no remote control on the peer.

## Out of scope for now

- Moving running processes or live windows.
- The clipboard while input is on a peer.
- Screen edges between machines.
