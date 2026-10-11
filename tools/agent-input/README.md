# Agent input fixture

To exercise the actual `dojo-agent` MCP subprocess, build it and run:

```sh
devenv shell -- cargo build --bin dojo-agent
devenv shell -- tools/agent-input/run-nested.sh --mcp
devenv shell -- tools/agent-input/run-nested.sh --mcp --gpui --recorder "$PWD/target/debug/examples/agent_seats"
devenv shell -- tools/agent-input/run-nested.sh --mcp --lifecycle
devenv shell -- tools/agent-input/run-nested.sh --mcp --gpui --lifecycle --recorder "$PWD/target/debug/examples/agent_seats"
```

The MCP scenario checks initialization, tool discovery, strict arguments, context
queries, persistent seat ownership, clicks/keys/scroll delivery, independent
server seats, failure cleanup, repeated release and held-key release on stdin
EOF. The `--lifecycle` variant also checks worker absence at initialization, idle
expiry without MCP polling, process reaping, held-key and cursor cleanup, restart,
context traffic that does not renew the lease, worker crash and abrupt adapter
exit. It records Wayland or GPUI callbacks in the same evidence directory. See
[client setup](../../docs/agent-input.md#connect-an-ai-client).

This standalone Rust client creates two native Wayland windows, binds every seat,
and logs input as JSON lines with its seat and target window. The Python runner
keeps one agent IPC connection alive, selects the second window, sends pointer
buttons and evdev keys, and asserts that the primary layout focus stays on the
first window. It disconnects with Shift and a pointer button held and requires
release events followed by removal of the agent seat.

Build in the repository's development environment:

```sh
devenv shell -- cargo build --manifest-path tools/agent-input/Cargo.toml
```

Start the forked compositor inside the current Wayland desktop:

```sh
devenv shell -- env DOJO_AGENT_INPUT=1 vendor/niri/target/debug/niri --config tools/agent-input/nested.kdl
```

The nested compositor prints its Wayland display and IPC socket names. In another
terminal, point both variables at that instance and run:

```sh
WAYLAND_DISPLAY=wayland-N NIRI_SOCKET=/run/user/1000/niri.wayland-N.PID.sock \
  python3 tools/agent-input/run.py --log /tmp/dojo-agent-input-events.jsonl
```

Use the actual socket path printed by Niri. The runner exits with an assertion or
IPC error if delivery, focus isolation, or disconnect cleanup fails. The final
JSON names the recorded evidence file. No desktop configuration is installed.
The recorder also runs alone for manual simultaneous human and agent input:

```sh
devenv shell -- env WAYLAND_DISPLAY=wayland-N tools/agent-input/target/debug/dojo-agent-input-fixture
```

The automated scenario also creates a second controller, proves its reset and
removal preserve the first controller, locks the nested session through
`ext-session-lock-v1`, requires held releases and rejected agent keys, and checks
input resumes after unlock. It saves PNG screenshots before cursor creation, with
one agent cursor, with both agent cursors, and after disconnect. A pixel comparison around the cursor
requires an actual rendered change and a return to the baseline after removal. Physical human key delivery, popup grabs, and cursor
other rendering paths require additional scenarios. Successful runs print a JSON result with the event log and screenshot paths.

Run the nested scenario automatically after building the compositor and fixture:

```sh
devenv shell -- tools/agent-input/run-nested.sh
```

The helper starts one nested compositor, reads its actual child environment from
its log, runs the controller, and terminates only that compositor's PID. It saves
logs and the child environment in a fresh `/tmp/dojo-agent-nested.*` directory.
Override `NIRI_BINARY` if the compositor was built in another target directory.

Build and exercise the two-window GPUI example with the same controller:

```sh
devenv shell -- cargo build --example agent_seats
devenv shell -- tools/agent-input/run-nested.sh --gpui --recorder "$PWD/target/debug/examples/agent_seats"
```

GPUI records application key and mouse callbacks. The raw Wayland recorder also
records seat identity, enter/leave events, modifiers, and removal. GPUI mode
requires the expected key and mouse callbacks and checks retained layout focus;
the raw scenario supplies the per-seat disconnect assertions.

The nested config and runner place both fixture windows in the floating layout
at explicit positions. The runner applies idempotent placement actions because
GPUI may supply its final title after the initial window rule match. This makes their geometry available through IPC; this Niri revision
omits workspace-view coordinates for tiled windows. The scenario still asserts
that agent input preserves the human window's layout focus and both windows'
geometry.

The GPUI scenario also requires exact continuous scroll deltas, independent Shift
state while separate controllers type uppercase `A` and lowercase `b`, and a
held `b` callback in the intended window. Reset must stop that repeat. A later
held `T` exercises repeat cleanup when its controller disconnects. The raw client
asserts the original axis amount and each seat's modifier mask.

The GPUI recorder observes both mouse-up and mouse-up-out callbacks so release
events remain visible when keyboard input has suppressed hover highlighting.

Add `--preexisting-seat` to create the controller seat before the recorder connects.
This runs the same scenario against initial registry discovery, alongside the
normal scenario that announces the seat after the application is running.

The raw scenario and both GPUI registry-discovery scenarios passed in nested
Wayland sessions on 2026-10-10. Each final run observed 660 changed cursor pixels
and an exact return to the blank cursor-region baseline after disconnect.

## Cursor movement

Run `devenv shell -- tools/agent-input/run-nested.sh --motion` to verify actual
intermediate Wayland motion events, arrival before a raw click, redirection and
reset cancellation. The evidence directory includes screenshots at the start,
during movement, at arrival and after reset. The normal input and lifecycle
scenarios wait for cursor arrival before inspecting its destination.

## Shared pointer coordination in Electron

Build the compositor, MCP adapter and fixture tools, then run the installed
Electron runtime in an isolated app and profile:

```sh
devenv shell -- cargo build --release --locked --manifest-path vendor/niri/Cargo.toml
devenv shell -- cargo build --bin dojo-agent
devenv shell -- cargo build --locked --manifest-path tools/agent-input/Cargo.toml
devenv shell -- python3 tools/agent-input/run-electron.py --electron /path/to/t3code
```

The runner copies the executable, links its runtime dependencies and replaces
only the copied app resources. It leaves the running T3 application alone. The
protocol trace must show that Electron bound only the primary seat. The page
must receive an agent click, Shift+A, scrolling and release of a held B.

The `human-pointer` helper injects primary-seat motion into the nested session.
The runner checks ownership notifications, blocked and stale actions, held-key
cleanup, a working human click, the retained agent cursor and recovery after the
human leaves. Screenshots require cyan cursor pixels before and after takeover,
and no cyan pixels after release. The MCP adapter executable is removed before
its first action to exercise worker startup during binary replacement.
An additional raw controller holds a mouse button while the human takes over;
the page must receive its release before subsequent human input.
Evidence remains in the printed `/tmp/dojo-electron-coordination.*` directory.
