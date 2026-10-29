# Quick terminal on the Rex client

Status: ready-for-agent
Blocked by: 02

`[quick-terminal] client = "rex"` makes `src/quick_terminal.rs` start `dojo-terminal --panel` with the placement it gives kitty today, and end it on hide. Install the binary with the other Dojo release binaries, link libghostty-vt statically, and add `libghostty-vt` to devenv. Kitty stays the default.
