# Rex protocol and libghostty in dojo-rex

Status: ready-for-agent

Build `crates/dojo-rex` per [the spec](../spec.md): control connection, data connection with handshake and resume, stream demux of `rex1` messages, and a libghostty-vt terminal (snapshot restore, VT write, screen rows with styled cells, key encoding). Unit tests for framing, handshake bytes, APC parsing and byte counting; one ignored live test against the running server.
