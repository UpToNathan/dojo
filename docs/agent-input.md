# Agent input

The compositor fork provides real Wayland seats for agent controllers. Each seat
has an independent keyboard target and a visible software cursor. Agent clicks
select that seat's keyboard target without activating the human's Niri layout.

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

## Experimental scope

Agent input requires `DOJO_AGENT_INPUT=1` in the forked compositor's environment.
The nested runner sets it. The normal installed desktop is not replaced by these
builds.

Agent window move/resize, popup grabs, drag-and-drop and layout activation are
disabled. The cursor is a compositor-owned arrow. Native Wayland clients must
bind multiple seats; XWayland support has not been established.

In experimental mode, the virtual-keyboard and input-method globals are hidden
because their current Smithay handlers bypass the controller's input gate.
The cursor-shape global is also hidden; the human cursor uses the normal surface
cursor protocol. These restrictions apply to the whole nested session. Normal
mode retains the upstream behavior.

GPUI routes additional-seat keys, direct composition, repeat, modifiers, pointer
events and scrolling independently. Clipboard and primary selection still use
the primary seat. Additional seats do not bind text-input-v3 or pinch gestures.
Widgets within one window still share their document, caret and interaction
state; the demonstration verifies separate windows.
