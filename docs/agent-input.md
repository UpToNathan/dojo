# Agent input

The compositor fork provides real Wayland seats for agent controllers. Each seat
has an independent keyboard target and a visible software cursor. Agent clicks
select that seat's keyboard target without activating the human's Niri layout.

## Application compatibility

Clients must bind the additional Wayland seat to receive independent agent input.
Dojo's GPUI fork does this. T3's Electron 44.4.2 / Chromium 152.0.7977.130 binds
only the original seat, so the independent cursor renders but T3 ignores its
clicks and keys. This was reproduced with the same Electron runtime in an
isolated compositor and confirmed in its protocol trace.

The current `handled` response acknowledges compositor dispatch and does not
prove that an application received the event. Verify an actual UI response.
[The compatibility bug](../.scratch/agent-input/issues/04-chromium-seat-compatibility.md)
records the evidence and required follow-up. It remains unresolved.

## Build

Run these commands from the repository root:

```sh
devenv shell -- cargo build --manifest-path vendor/niri/Cargo.toml --no-default-features
devenv shell -- cargo build --manifest-path tools/agent-input/Cargo.toml
devenv shell -- cargo build --example agent_seats
```

Dojo imports `gpui` and `gpui_platform` from `vendor/zed`, and `niri-ipc` from
`vendor/niri`. The source revisions and behavior contract are recorded in
[the spec](../.scratch/agent-input/spec.md). The Keywork subtrees remain reference
sources.

## Run the nested demonstration

```sh
devenv shell -- tools/agent-input/run-nested.sh
devenv shell -- tools/agent-input/run-nested.sh --gpui --recorder "$PWD/target/debug/examples/agent_seats"
```

The runner creates a nested Niri session, records its actual socket and display,
and drives two windows. It saves evidence under `/tmp/dojo-agent-nested.*` and
stops only the processes it started. See [the fixture guide](../tools/agent-input/README.md)
for the assertions and manual controls.

## Control from Dojo

Create the controller on a background thread. Its operations use blocking IPC.
Keep it alive while the agent is working:

```rust,no_run
use dojo::agent_input::{AgentController, InputState};

let mut agent = AgentController::connect("assistant")?;
agent.focus_window(window_id)?;
agent.move_pointer(output_name, x, y)?;
agent.button(272, InputState::Pressed)?;
agent.button(272, InputState::Released)?;
agent.key(30, InputState::Pressed)?;
agent.key(30, InputState::Released)?;
agent.scroll(0.0, 40.0)?;
agent.reset()?;
```

Window IDs and output names come from Niri IPC's `Windows` and `Outputs`
requests. Coordinates are logical pixels relative to the output. Keyboard codes
are Linux evdev codes without the XKB offset. Button 272 is the left mouse
button; key 30 is A on the default layout.

Pointer movement follows compositor frames, with distance-dependent travel time
between 120 and 550 milliseconds. It accelerates and decelerates, and a raw
controller can redirect it while preserving its position and velocity. The
first appearance starts at the human pointer's location without moving that
pointer. `move_pointer()` waits for arrival before returning. Niri also defers
raw button and scroll requests until arrival. Niri's `animations { off; }`
setting makes movement instant.

Raw `Move` replies with `AgentInputMoving` when it starts a trajectory. Send
`"Wait"` to wait for arrival, or another `Move` to redirect it. A move requiring
no animation replies with `Handled`. Reset, locking, output removal and
disconnect cancel movement. Active movement drives frame callbacks; a settled
or removed seat schedules no cursor animation frames.

Dropping the controller closes its socket. Niri releases held input, clears its
focus and removes the seat and cursor. `destroy()` explicitly performs the same
cleanup and consumes the controller. Locking resets agent input; unlocking
requires a fresh target or click.

The compositor CLI also accepts JSON commands on one connection:

```sh
vendor/niri/target/debug/niri msg agent-input
```

Send one `AgentInput` command per line. For example, `{"Create":{"name":"assistant"}}`
creates the seat. Keep stdin open for subsequent commands. Separate one-shot
`raw-request` invocations close their controller immediately.

## Connect an AI client

### T3 Code on this machine

T3 Code's active Codex provider is configured through this project's
[`.codex/config.toml`](../.codex/config.toml). It starts the built `dojo-agent`
executable and explicitly forwards `NIRI_SOCKET`. The executable path matches
this checkout; update it if the repository moves. Codex loads project MCP
configuration for trusted projects, as described in the
[official OpenAI documentation](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).

The Dojo repository is marked trusted in the local Codex user configuration.
The previous user config was preserved before adding that entry. Existing T3
provider sessions need to reload before their tool catalog includes the new
server. Use T3 Code with its Codex provider for this integration; other provider
configurations are separate.

Verify the registration without running a model or creating an input seat:

```sh
codex mcp get dojo-desktop --json
python3 tools/agent-input/check-codex.py
```

The check creates an ephemeral Codex app-server session, applies the same
whole-table MCP override shape as the installed T3 bundle, discovers the three
Dojo tools, and calls `desktop_context`. It stops its own app-server afterward.
It verifies provider discovery and socket forwarding; the nested scenarios
below verify actual input. Upstream Niri can supply context but rejects
independent-seat actions. Input needs the experimental fork; see
[the desktop session switch guide](niri-session.md).

T3 is the initial client. The server and controller do not depend on T3; a future
pi-durable adapter can use the same desktop interface. pi-durable integration
has not been implemented or verified.

### Other MCP clients

Build the desktop MCP server:

```sh
devenv shell -- cargo build --bin dojo-agent
```

Add a stdio server to your AI client's MCP configuration. Use the absolute path
to the built executable:

```json
{
  "mcpServers": {
    "dojo-desktop": {
      "command": "/home/nathan/Dojo/target/debug/dojo-agent",
      "args": ["--name", "assistant"]
    }
  }
}
```

The client must inherit `NIRI_SOCKET` from a session running the experimental
compositor. For a nested session, pass `--socket /path/to/its/niri.sock` in
`args`. The installed upstream compositor cannot execute these input tools.
Do not launch through `devenv shell` in an MCP configuration: its startup output
would mix with the protocol stream.

The client discovers three tools:

| Tool | Behavior |
| --- | --- |
| `desktop_context` | Lists windows, outputs, the current agent seat and worker PID. Starts no worker. |
| `desktop_action` | Starts a worker on demand, then focuses, moves, clicks, injects keys, scrolls or resets its seat. |
| `desktop_release` | Stops the worker and drops its seat and cursor; safe to repeat. |

Example `desktop_action` arguments:

```json
{"action":"focus","window_id":42}
{"action":"move","output":"DP-1","x":400,"y":300}
{"action":"click","button":"left"}
{"action":"key","evdev":30,"state":"tap"}
{"action":"scroll","horizontal":0,"vertical":40}
{"action":"reset"}
```

Key states are `press`, `release` and `tap`. For a chord, press a modifier, tap
the key, then release the modifier. Click buttons are `left`, `right` and
`middle`. Key codes follow the compositor's current keyboard layout. There is
no arbitrary Unicode text insertion.

The stdio MCP process is a small adapter. Its desktop worker starts on the first
valid input action and owns one seat. At rest there is no worker, controller
connection, cursor or per-agent animation loop. Context, reset and release calls
do not start an idle worker. Context reports `worker_pid` and `seat_name` as null
when the worker is absent.

After 60 seconds without an input action, the worker closes its controller and
exits. Context traffic does not renew that deadline. Set
`--idle-timeout-seconds SECONDS` in the MCP server arguments to choose another
timeout (1 through 86400 seconds). Release immediately stops the worker. Use it
when an agent task finishes; the timeout catches abandoned sessions. The adapter
reaps children through exit notifications, with no periodic idle polling.

Calls execute serially. Input execution errors stop the worker and release held
input. Adapter EOF, normal exit or crash also stop the worker and remove its
seat. A later action starts a fresh worker; select a target again before typing.
Failed actions are never automatically replayed. Invalid arguments leave the
existing worker intact. Reset releases held input and focus while retaining an
active seat.

Reopen T3 Code or reload its provider session after rebuilding `dojo-agent` to
replace an already-running adapter. The compositor and Dojo shell need no restart
for worker lifecycle changes.

The server uses the [official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)
and [stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).
It supplies desktop tools to an existing AI client. It does not run a model,
record voice, provide screenshots/accessibility observations, or add the
planned agent UI. Use the client's own conversation and tool controls.

Verify the actual MCP subprocess in a nested desktop:

```sh
devenv shell -- tools/agent-input/run-nested.sh --mcp
devenv shell -- tools/agent-input/run-nested.sh --mcp --gpui --recorder "$PWD/target/debug/examples/agent_seats"
devenv shell -- tools/agent-input/run-nested.sh --mcp --lifecycle
devenv shell -- tools/agent-input/run-nested.sh --mcp --gpui --lifecycle --recorder "$PWD/target/debug/examples/agent_seats"
```

## Experimental scope

Agent input requires `DOJO_AGENT_INPUT=1` in the forked compositor's environment.
The nested runner and installed Dojo session wrapper set it. Building alone does
not replace the installed desktop; use the session installer to select a build.

Agent window move/resize, popup grabs, drag-and-drop and layout activation are
disabled. The cursor is the cyan `nf-fa-mouse_pointer` Nerd Font glyph, rendered
at 24 logical pixels high from an embedded high resolution asset. Its buffer
exists only while the agent seat exists. Native Wayland clients must
bind multiple seats; XWayland support has not been established.

In experimental mode, the virtual-keyboard and input-method globals are hidden
because their current Smithay handlers bypass the controller's input gate.
The cursor-shape global is also hidden; the human cursor uses the normal surface
cursor protocol. These restrictions apply to the whole enabled session. Normal
mode retains the upstream behavior.

GPUI routes additional-seat keys, direct composition, repeat, modifiers, pointer
events and scrolling independently. Clipboard and primary selection still use
the primary seat. Additional seats do not bind text-input-v3 or pinch gestures.
Widgets within one window still share their document, caret and interaction
state; the demonstration verifies separate windows.
