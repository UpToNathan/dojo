---
status: accepted
---

# A sent terminal is a session connected back to its source

[Sending windows](../../.scratch/mesh-boundaries/spec.md) first opened a kitty window's directory on the peer, with a fresh shell there: a terminal moved by being started again where the directory might not even exist, and anything it was running ended with the window. That made a sent terminal a different place from the one you were working in.

A terminal is now sent as a window on the peer that is **connected back to the machine it came from**, in the same directory, over the mesh's own channel (`dojo-mesh shell`, with a terminal requested). The files, the history and the programs stay where they were; the peer's window is a second view of that machine. A program the window was running that Dojo knows how to open (nvim on a file at a line, yazi in a directory) is started again in the new session, on the source. The window here closes once the peer has opened its session, unless something Dojo cannot carry over is running in it, which would end with the window.

The handoff can name the machine the session lives on (`host`). A window that is itself such a session (a kitty running `dojo-mesh shell HOST --directory DIR`) is sent as that session, so sending it **back** to HOST opens an ordinary terminal there, and sending it to a third machine connects that one to HOST. It reopens in the directory the session started in: where the shell has moved to since is known only to the machine it runs on.

T3 Code follows the same idea: its thread is not moved either, and the peer's T3 Code opens it through T3 Connect from a `t3code://` link.

## Consequences

- The peer must be able to reach the sender, which a mesh already means: both are members, and each holds the other's host key. Who the sender is comes from the mesh method's identity of the connection, never from the handoff, so a handoff cannot name some other node to connect to. Under the roster method a node key names nobody, so the sender's own statement is checked against the signed roster, as `inject --as` is; a member holds a full shell on the peer already, so a false claim gains nothing but a window connected to another member.
- The peer never runs a command it is sent. It starts its own terminal on `dojo-mesh shell SENDER --directory PATH`, where the path and the program are checked and the program is one of a fixed few.
- Files the peer lacks no longer stop a terminal send or need copying, because the terminal reads them where they are.
- Moving a running process is still out of scope. A multiplexer that holds one (tmux, abduco) can be attached to from the new session; recognising it is future work.
- A peer with an older `dojo-mesh` refuses the new handoff, and says so.
