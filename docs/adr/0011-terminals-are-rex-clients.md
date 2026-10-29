---
status: proposed
---

# Terminals are Rex clients

A kitty window owns its shell: closing it ends whatever ran there, and [ADR-0010](0010-a-sent-terminal-is-a-session-connected-back-to-its-source.md) could only move a terminal by starting a new session connected back to its source. Moving a running process was left out of scope, waiting on a multiplexer.

Dojo's terminals now run in a session of the personal Rex server, and the terminal you see is a client attached to it. Dojo writes that client itself (`dojo-terminal`), on Rune for drawing and libghostty-vt for terminal state, because Rex's stream is libghostty's own snapshot format followed by VT bytes, and Rex offers no Linux client. The quick terminal starts it as a separate process on a layer-shell surface, placed the way kitty is placed today, so the shell keeps drawing only the frame around it.

## Consequences

- Hiding or closing a terminal ends a client, never a program. The quick terminal costs nothing while hidden, since its client process exits.
- The client depends on Rex's protocol (documented by the server, API revision 7) and on libghostty's snapshot format, which upstream marks as a work in progress. The libghostty build has to match what the Rex server writes; a Rex update can require a libghostty update.
- Rex's package server is private, so every machine needs the package token (from 1Password, `scripts/install-rex-token.sh`).
- A second UI stack (Rune, wgpu, winit) joins GPUI, confined to the terminal client.
- Rex gives each terminal a single size owner; the most recently attached client takes it.
- Sending a terminal to a peer (ADR-0010) can later attach the peer to the same Rex session over Tailscale instead of opening a new shell. Not part of this decision.
