# Agent input

The compositor fork provides real Wayland seats for agent controllers. Each seat
has an independent keyboard target and a visible software cursor. Agent clicks
select that seat's keyboard target without activating the human's Niri layout.

## Application compatibility

Clients that bind an agent's Wayland pointer and keyboard receive native seat
input. For clients that bind only the primary seat, including T3's Electron,
Niri sends agent events through that client's existing resources. The human's
compositor pointer position and layout keyboard focus remain independent.

Human pointer interaction inside an application or typing in its focused window
takes ownership before input delivery. Niri cancels conflicting agent movement,
releases held keys and buttons, restores human protocol focus and modifiers,
and keeps the agent cursor visible. Ownership covers the client connection,
including its other windows. Pointer ownership ends when the human leaves;
keyboard ownership ends on pointer interaction after held keys are released.
Held agent input on the shared primary resources prevents another agent from
taking over that pipeline. Native seats retain independent held input.

`handled` confirms that input was sent to a bound resource. It does not prove
that the application performed the intended action. Verify the UI result.
Clicks, keys and scrolling fail if no usable resource exists. Interactive window
move/resize, client pointer grabs, drag and drop, and X11 remain unsupported.

## Ownership protocol

`Request::InputCoordination` returns the current revision and
`human_owned_windows`. Niri's event stream emits `InputCoordinationChanged` and
includes the current snapshot when a subscriber connects. Window membership is
expanded from client connection ownership.

Wrap raw commands in `Coordinated { revision, command }` to reject actions based
on stale ownership. The SDK does this automatically, including arrival waits.
Reset and destruction remain unguarded cleanup operations. Human takeover or a
conflicting held interaction returns `AgentInputPaused` with a reason and fresh
snapshot. The SDK returns `InputPaused`; it updates its revision without
retrying the action.

MCP `desktop_context` includes `coordination`. A paused `desktop_action` returns
`handled: false`, `paused: true`, the reason and snapshot, while retaining its
seat and session. The agent must inspect the situation and choose its next action.
MCP supplies this information through tool responses; a runtime needing immediate
notifications can subscribe to Niri's event stream. No observer worker runs while
the adapter is idle. The updated SDK requires the updated compositor fork.

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
executable and explicitly forwards `NIRI_SOCKET`, `XDG_RUNTIME_DIR` and
`WAYLAND_DISPLAY`. The executable path matches
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
whole-table MCP override shape as the installed T3 bundle, discovers the six
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

Add a stdio server to your AI client's MCP configuration. Use the stable launcher
linked by `scripts/install-niri-fork.sh`:

```json
{
  "mcpServers": {
    "dojo-desktop": {
      "command": "/home/nathan/.local/bin/dojo-agent",
      "args": ["--name", "assistant"]
    }
  }
}
```

The launcher sources the current Dojo Niri installation's `environment.sh` and
executes its validated `DOJO_SEMANTIC_AGENT_EXE` pin. An invalid configured pin
refuses startup. With no pin configured, it uses the checkout's development
binary; native semantic mutation still requires the compositor's pinned agent.
Run `scripts/install-voice-config.py` explicitly to create the Codex entry or
migrate this checkout's old `target/debug/dojo-agent` entry. It preserves other
settings and existing tool membership while adding launch, screenshot,
accessibility observation and accessibility action tools. It does not run during installation.

The client must inherit `NIRI_SOCKET` and `XDG_RUNTIME_DIR` from a session
running the experimental compositor. For a nested session, pass `--socket /path/to/its/niri.sock` in
`args`. The installed upstream compositor cannot execute these input tools.
Do not launch through `devenv shell` in an MCP configuration: its startup output
would mix with the protocol stream.

The client discovers three tools:

| Tool | Behavior |
| --- | --- |
| `desktop_context` | Lists windows, outputs, coordination, this session's seat and shared service PID. Starts no service. |
| `desktop_action` | Connects to the shared service on demand, then focuses, moves, clicks, injects keys, scrolls or resets its seat. |
| `desktop_release` | Drops only this session's seat and cursor; safe to repeat. |
| `desktop_feedback` | Shows an optional fading response in Dojo. `complete: true` restores the settled Arch icon. Starts no cursor service. |

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

### Shared service lifecycle

The stdio MCP process is a small adapter. On the first valid input action it
connects to a shared Unix socket service, starting `dojo-agent --service`
directly if necessary. Systemd is not required. A file lock permits only one
service for each canonical Niri socket path. Concurrent adapters connect to that
same process, with separate sessions and Niri seats. The service runs in its own
process group, so terminating an adapter's process group does not terminate the
other clients' service.

Sockets live under `$XDG_RUNTIME_DIR/dojo-desktop/`, a directory with mode 0700.
The socket has mode 0600. Its name uses the first 24 hexadecimal characters of
the SHA-256 hash of the canonical Niri socket path. The adjacent lock file remains
on disk after exit to preserve its inode across concurrent starts. No service
process, controller or cursor remains after all seats are idle. A grace period
of up to 250 milliseconds allows startup and shutdown traffic to complete.

Each session releases its seat after 60 seconds without an input action.
Context and socket status queries do not renew that deadline. Set
`--idle-timeout-seconds SECONDS` in the MCP adapter arguments to choose a timeout
from 1 through 86400 seconds for that session. Idle reset and release calls do
not start a service. Context reports `worker_pid` and `seat_name` as null for an
idle session, even if other clients are using the service. For an active session,
`worker_pid` is the shared service PID, retained under its original field name
for MCP compatibility.

Release, input execution errors, adapter EOF and adapter crashes release only
that session's held input and cursor. Service failure releases all its seats.
A later action reconnects and creates a fresh seat; select a target again before
typing. Actions are never automatically replayed. Startup can retry its handshake
because no input has been sent. Invalid arguments leave the existing seat intact.
Reset releases held input and focus while retaining an active seat. Actions run
serially in the service; human priority remains enforced immediately by Niri.

Reopen T3 Code or reload its provider session after rebuilding `dojo-agent` to
replace an already-running adapter. The compositor and Dojo shell need no restart.
An already-running shared service keeps its executable until all its sessions
release or expire.

### Direct socket clients

A future Pi or voice client can use the same service without MCP. Obtain the
socket path without starting the service:

```sh
dojo-agent --print-service-socket
```

To launch the service explicitly, run `dojo-agent --service`.
Connect within its startup grace period. The MCP adapter handles launching,
readiness and reconnecting automatically. Explicit `--socket` selects a Niri
socket for either command.

Use one Unix connection per agent session. Send UTF-8 JSON objects terminated by
newlines, with at most 65536 bytes per message including the newline. First send:

```json
{"operation":"hello","version":1,"name":"pi","idle_timeout_seconds":60}
```

Names contain 1 through 32 ASCII letters, digits, hyphens or underscores.
The reply is `{"Ok":{"version":1,"service_pid":12345}}`. Each following request
has one response, either `{"Ok":VALUE}` or `{"Err":"description"}`. Serialize
requests on a connection. Input uses the same action object as the MCP tool:

```json
{"operation":"action","action":{"action":"focus","window_id":42}}
{"operation":"status"}
{"operation":"release"}
```

Status returns the current `seat_name`, or null. Release drops the seat and
returns `{"released":true}`. Closing the connection releases its seat as well.
Malformed or oversized messages close the connection.
A disconnected or idle session has no resumable target or held input. Reconnect
with a new handshake and choose a fresh target. Paused actions retain their seat
and return the same ownership snapshot as MCP.

The server uses the [official Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk)
and [stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports).
It supplies desktop tools to an existing AI client. Pi owns Dojo's model calls
and durable tool execution. The Rust service supplies bounded screenshots,
application accessibility observations, input, and verification.

`desktop_screenshot` captures visible output pixels through native Wayland
capture. `DOJO_CAPTURE_BACKEND=grim` selects the explicit migration backend. Existing `{"output":"eDP-1"}` calls capture the whole
output at scale 1. Optional fields select a smaller observation:

- `region`: `{ "x":100, "y":80, "width":400, "height":300 }`, in
  output-relative logical pixels. The rectangle is clipped to the output.
- `window_id`: a window on the selected output's active workspace. Its visible
  tile rectangle is cropped, including any surfaces overlapping it. The updated Niri supplies tiled and floating placements from the same render
  as the PNG. Unsupported transitions and animations return an error.
- `scale`: image pixels per logical pixel, from 0.1 through 1. Default 1.
  A scale of 0.5 reduces both image dimensions.

`region` and `window_id` cannot be combined. Every result includes a PNG,
`logical_x` and `logical_y` for its origin, captured logical dimensions, actual
image dimensions, and per-axis pixel mapping. Map image coordinates with
`desktop_x = logical_x + image_x * logical_x_per_image_pixel` and the same
formula for y before calling `desktop_action`. Full-output origins are zero.
Inspect the screenshot before clicking, then capture again to verify the result.
Capture does not create or renew an input seat, change focus, or replace the
clipboard. Capture has a five-second deadline. Window captures bind pixels and
placements to one compositor render. Output capture verifies compositor peer
identity and output geometry. Rebuild and reopen the MCP adapter to discover
updated schemas. Installing the updated compositor is a separate deployment.

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
at 24 logical pixels high from an embedded high resolution asset. Its color
follows Dojo's primary accent through the Niri `cursor.agent-color` theme export,
including cursors already visible when the theme changes. Its buffer
exists only while the agent seat exists. Native Wayland clients must
bind multiple seats; XWayland support has not been established.

In experimental mode, the virtual-keyboard and input-method globals are hidden
because their current Smithay handlers bypass the controller's input gate.
These restrictions apply to the whole enabled session. Cursor-shape requests
resolve the matching live seat; retired pointers remain inert. Only the human
seat can change the human cursor. Normal mode retains the upstream behavior.

GPUI routes additional-seat keys, direct composition, repeat, modifiers, pointer
events and scrolling independently. Clipboard and primary selection still use
the primary seat. Additional seats do not bind text-input-v3 or pinch gestures.
Widgets within one window still share their document, caret and interaction
state; the demonstration verifies separate windows.

## Launch desktop applications

`desktop_launch` accepts `application="browser"` for a new default-browser
window, or a desktop entry id such as `org.gnome.Nautilus.desktop`. It shares
Dojo's launcher, reads the graphical session environment from systemd, and
returns only after Niri reports a new window belonging to the application.
Existing human-owned windows do not block launching another application.
Run `devenv shell -- python3 scripts/check-desktop-launch.py` to verify a
real browser launch even with graphical variables removed from the MCP process.

`devenv shell -- python3 scripts/check-voice-browser.py` exercises the same
request through voice dispatch and checks a new browser window, completion,
and explicit zero reasoning. Both launch checks leave their browser windows open.

### Voice handoff

The `YieldHumanInput` IPC request clears human ownership without moving the human
cursor or keyboard focus. Dojo issues it when push-to-talk is released. Held keys,
pointer grabs and suppressed buttons reject the request. The next human input
reacquires ownership and pauses conflicting agents. This requires the updated
compositor, which becomes active on the next Niri login.

## Capture-bound input

`desktop_observe` starts the shared service's bounded capture storage without
creating an input seat. Give it `output` and a positioned `window_id` to receive
an actionable PNG plus opaque `capture_id`, digest, process/output/window
identity, dimensions and coordinate mapping. Output/region observations are
read-only. Capture storage expires after 30 seconds and does not renew a seat's
idle deadline.

Use `desktop_capture_action` with that ID and coordinates in the delivered PNG:

```json
{"capture_id":"capture_...","x":120,"y":80,"input":{"kind":"click","button":"left"}}
```

Inputs support `move`, `click`, `double_click`, physical `key` and `chord`,
`key_hold` for 1 through 1000 milliseconds, printable ASCII `text` on a verified
plain US layout, and in-window `drag`. Drag x/y selects the initial press point;
`path` contains 1 through 16 subsequent image-pixel waypoints. `scroll` uses
`horizontal` and `vertical` logical-pixel deltas. Unicode, clipboard insertion,
cross-window drag, and persistent held keys are refused. Rust validates the owning session, identity,
geometry, bounds and ownership revision, then maps the image point to its
output. Admission consumes the reference; the service also retires sibling
observations before input. Take a fresh observation after every attempt. Release,
disconnect, restart, expiry and intervening low-level input invalidate captures.

The updated compositor accepts captured geometry, exact target, and coordination
wrappers around input. It checks rendered window placement, output scale and
transform, compositor generation, and coordination on each command and animation
frame. It checks the mapped pointer
or keyboard target at actual delivery; deferred pointer events retain the guard
through movement. Releases remain available for cleanup. The captured visible
crop may include another window, so an occluding window causes guarded click or
scroll refusal. Older compositors reject the wrapper and capture actions refuse.

A receipt reports delivery as `handled`, ownership pause as `paused`, or an
uncertain operation as `unknown`. Each carries `verified: false` and requires a
fresh observation. The adapter releases its seat after an unknown effect. Pi
commits capture consumption before dispatch, marks desktop mutations unsafe to
replay, preserves images, and requires new observation after interruption.
These checks do not detect control changes inside an otherwise unchanged window.
Screenshots retain the existing visible-content and transcript privacy limits.

See [the Rust authority and acceptance commands](../crates/dojo-computer/README.md)
and [the bounded Rust model parsers](../crates/dojo-computer-models/README.md).
Pi's optional `desktopModelProfile` converts supported textual profiles into
ordinary Pi tool calls before response classification. Rust owns parsing and
coordinate restoration. Tool arguments preserve bounded prediction provenance.
Native Anthropic, OpenAI, and Gemini envelope decoders preserve safety metadata;
native provider transport and subscription endpoint support remain separate gates.


## CUA-derived capabilities

`desktop_accessibility` returns a bounded AT-SPI tree. GTK7 clients can announce
their accessibility root on the exact Wayland surface; Niri exposes that binding
and its generation. The reader proves bus ownership and process generation,
walks only that root, and rechecks the native target before returning. Other
clients retain application scope, which may include sibling windows. Check
`scope` and `window_scoped`. Password and credential fields are filtered before
secret getters. Neither route returns actionable refs or authoritative bounds.
`complete` describes traversal. `semantic_complete` additionally requires exact
window scope, an unfiltered query, and no redacted controls.
Browser automation uses the pinned anomalyco Browser Control package rather than
CUA's browser adapters. The `desktop_browser_*` routes were removed. Native
capture, independent desktop input, AT-SPI and verification remain available.
See [Browser Control](browser-control.md) for packaging, agent configuration,
cancellation and the operator's ownership handoff.

`desktop_verify` checks bounded window existence and rendered bounds predicates.
It also checks native element predicates against available, exactly bound GTK7
trees. Application trees cannot satisfy window element predicates. Embedded web
content stays outside native semantic verification. Incomplete walks cannot
establish absence, and duplicate matching controls remain ambiguous.
Missing or untrusted element evidence returns unknown. `observe_after=true` on
`desktop_capture_action` returns a fresh observation with the effect receipt.
Failure to obtain that image preserves the receipt and never retries input.

Pi persists tool-attempt reservations before transport. `desktopMaxToolAttempts`
defaults to 1000 per conversation. `desktopRequestImages` defaults to eight
recent desktop images in each request. Request projection preserves canonical
entries, signatures, and call/result pairing. It does not erase recorded pixels.
`human_request` stores a bounded clarification request in Pi. The local voice
host supplies its response. Clarification grants no permission and retires old
capture authority. Stop cancels pending requests.

Build `dojo-agent-setup` for a read-only client configuration plan. `--apply`
performs backed-up configuration edits and removal preserves edited user content.
Build `dojo-computer-recording` for bounded redacted traces, ATIF export, terminal
observations, offline processors, and isolated grounding evaluation. Pi's
`exportDesktopTrace()` projects canonical entries through that Rust codec and
omits screenshots by default. It includes Driver, browser-control, perception
and bounded-step calls, preserves originating Pi intent IDs and textual errors,
and records verification separately from delivery. A tool error alone does not
establish refusal or absence of an effect. Export analysis never replays effects.

Run `tools/speech/dojo-voice trace` to export that JSON from the running voice
agent. This read does not start a daemon, submit a task, reset the conversation,
or change input ownership. A stopped service reports that no export is available.

[The adoption record](../.scratch/cua-adoption/result.md) identifies pinned sources,
validation commands, deliberate differences, and remaining deployment and artifact
gates. Vendored source alone does not establish runtime parity.

## Operator desktop policy

Set `DOJO_DESKTOP_POLICY` to an absolute YAML file before launching a desktop MCP client. The file must be a regular file owned by root or your user, at most 64 KiB, and not writable by group or others. Symlinks are refused. Invalid configuration fails closed.

CUA's YAML evaluator supports allow lists, deny lists and argument constraints. For example:

```yaml
allow:
  tools: [desktop_context, desktop_observe, desktop_verify, desktop_decode_model]
  rules:
    - tool: desktop_capture_action
      constraints:
        x: { min: 0, max: 10000 }
        y: { min: 0, max: 10000 }
deny: [desktop_action, desktop_launch, desktop_browser_navigate]
```

This policy narrows the existing rules. It cannot authorize a stale capture, transfer human ownership, grant browser profile consent or resolve an unknown external effect. Deny entries take precedence. Tools omitted from the allow list are denied. Release, browser close and internal session status remain available for cleanup. The shared service socket and handshake include the policy fingerprint; changed policy files require a new connection before dispatch. This is operator configuration, not a model-visible permission grant.
