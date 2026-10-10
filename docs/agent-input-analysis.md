# Agent input in forked Niri: source analysis

## Scope and conclusion

The requested feature is an agent seat with its own visible pointer and keyboard focus while the human continues using the desktop. The compositor is in [`vendor/niri`](../vendor/niri), with references in [`vendor/keywork`](../vendor/keywork) and [`vendor/keywork-compositor`](../vendor/keywork-compositor). Dojo's GPUI application is separate from the compositor.

This analysis uses Niri `d0cd58ec`, Keywork `7137f5d9`, and Keywork compositor `6f26973d`, as imported into this repository. It is static source analysis. No compositor code changed, and no build or runtime checks ran.

A separate logical Wayland seat is a feasible direction, but creating a seat and injecting events does not establish independent cursor rendering, keyboard focus, or window-management behavior. Keywork demonstrates transient seats and seat-targeted pointer dispatch. Its transient keyboards lack focus initialization, and its transient cursors lack display integration. The pinned GPUI client also replaces its current seat when another is announced.

## Niri input and focus

Niri stores one `Seat<State>`, one `KeyboardFocus`, and shared cursor and popup state on its main state ([`niri.rs:343`](../vendor/niri/src/niri.rs#L343)). Startup creates that seat and adds its keyboard and pointer ([`niri.rs:2518`](../vendor/niri/src/niri.rs#L2518)). The hardware backend assigns libinput to the backend seat ([`tty.rs:446`](../vendor/niri/src/backend/tty.rs#L446)). An agent seat does not require another libinput instance, logind seat, or GPU assignment.

Virtual-pointer resources retain the requested `WlSeat` ([`virtual_pointer.rs:339`](../vendor/niri/src/protocols/virtual_pointer.rs#L339)). Their handlers nevertheless send events through the ordinary `process_input_event` path ([`handlers/mod.rs:662`](../vendor/niri/src/handlers/mod.rs#L662)). Relative motion, absolute motion, buttons, and axes then select the primary seat ([`input/mod.rs:2438`](../vendor/niri/src/input/mod.rs#L2438), [`2688`](../vendor/niri/src/input/mod.rs#L2688), [`2784`](../vendor/niri/src/input/mod.rs#L2784), [`3109`](../vendor/niri/src/input/mod.rs#L3109)). Output mapping already exists in [`backend_ext.rs`](../vendor/niri/src/input/backend_ext.rs). Seat routing is the missing dimension.

Human keyboard focus is recomputed during every refresh from shared layout, layers, and compositor UI ([`niri.rs:1241`](../vendor/niri/src/niri.rs#L1241)). Agent focus needs its own maintenance policy. Using normal window activation to select an agent target would also alter the human's focused window and may scroll the layout.

Several surrounding behaviors assume one seat:

- XDG move, resize, and popup-grab handlers ignore the request's seat. Popup handling installs grabs on the primary seat and uses singleton bookkeeping ([`xdg_shell.rs:71`](../vendor/niri/src/handlers/xdg_shell.rs#L71), [`184`](../vendor/niri/src/handlers/xdg_shell.rs#L184), [`326`](../vendor/niri/src/handlers/xdg_shell.rs#L326)). The ordinary [`ClickGrab`](../vendor/niri/src/input/click_grab.rs) operates through its supplied pointer handle and is reusable.
- Pointer-constraint callbacks ignore their pointer argument and assume one active constraint ([`handlers/mod.rs:151`](../vendor/niri/src/handlers/mod.rs#L151)).
- Activation validates serials against a seat, then activates the shared layout ([`handlers/mod.rs:778`](../vendor/niri/src/handlers/mod.rs#L778)).
- Clipboard and primary-selection focus callbacks already use the supplied seat. Cursor-image and LED callbacks ignore seat identity ([`handlers/mod.rs:100`](../vendor/niri/src/handlers/mod.rs#L100)).

Niri pins Smithay to `19c16d3e34172fcc8fb3d293bcae563b0a19ea70` ([`Cargo.toml:31`](../vendor/niri/Cargo.toml#L31)). Its virtual-keyboard manager honors the supplied seat. However, [virtual-keyboard key dispatch](https://github.com/Smithay/smithay/blob/19c16d3e34172fcc8fb3d293bcae563b0a19ea70/src/wayland/virtual_keyboard/virtual_keyboard_handle.rs#L103) sends keys directly to that seat's focused keyboard resources. It bypasses Niri's physical input processing and normal keyboard-grab dispatch. Agent keybinding, popup, inhibition, and lock behavior need explicit handling.

The pinned Smithay [Wayland module list](https://github.com/Smithay/smithay/blob/19c16d3e34172fcc8fb3d293bcae563b0a19ea70/src/wayland/mod.rs) has no transient-seat helper. Supporting `ext-transient-seat-v1` would require a Niri-owned protocol handler or dependency work. The controller transport remains a design choice.

## Cursor rendering and lifecycle

[`render_pointer`](../vendor/niri/src/niri.rs#L3847) reads the primary pointer position and the shared `CursorManager`. The main rendering path includes it under the human pointer's visibility state ([`niri.rs:4373`](../vendor/niri/src/niri.rs#L4373)). A second cursor needs separate image and visibility state, independent of hide-when-typing and tablet cursor replacement.

Painting another image is only part of that change. Cursor surfaces also participate in:

- Commit handling and hotspot updates ([`handlers/compositor.rs:406`](../vendor/niri/src/handlers/compositor.rs#L406)).
- Output enter/leave, scaling, and surface cleanup ([`niri.rs:3988`](../vendor/niri/src/niri.rs#L3988)).
- Primary scanout tracking, DMA-BUF feedback, frame callbacks, and presentation feedback ([`niri.rs:4912`](../vendor/niri/src/niri.rs#L4912), [`5161`](../vendor/niri/src/niri.rs#L5161), [`5257`](../vendor/niri/src/niri.rs#L5257), [`5344`](../vendor/niri/src/niri.rs#L5344)).
- Animated-cursor scheduling and capture paths ([`niri.rs:4768`](../vendor/niri/src/niri.rs#L4768), [`5687`](../vendor/niri/src/niri.rs#L5687)).

Those paths currently use singleton cursor state. The TTY backend allows cursor-plane scanout ([`tty.rs:1937`](../vendor/niri/src/backend/tty.rs#L1937)). A second cursor may require software composition. Its performance impact needs measurement rather than an estimate based on the number of cursors.

## What Keywork actually supplies

Both Keywork trees create temporary seats with independent pointer state. [`TransientSeat.create`](../vendor/keywork/src/compositor/wayland/transient_seat.zig#L144) initializes a full seat, advertises its global, and tracks resources and references. Virtual-pointer devices resolve the requested seat, and the server routes motion using that seat's position ([`virtual_pointer.zig:155`](../vendor/keywork/src/compositor/wayland/virtual_pointer.zig#L155), [`server.zig:6294`](../vendor/keywork/src/compositor/server.zig#L6294)). The nondefault-seat branch hit-tests and delivers pointer motion without moving the human pointer ([`server.zig:6153`](../vendor/keywork/src/compositor/server.zig#L6153)).

The earlier description overstated its keyboard support. A new seat starts with `parent_focused = false` and `focus = null` ([`seat.zig:306`](../vendor/keywork/src/compositor/wayland/seat.zig#L306)). Transient-seat creation never establishes either. The compositor's keyboard-focus assignments target the default seat, and [`keyWithGrab`](../vendor/keywork/src/compositor/wayland/seat.zig#L1041) rejects delivery without parent focus. Targeting a transient seat with a virtual keyboard therefore does not make its keystrokes reach applications.

Visible cursors are also incomplete. Only the default seat gets a default cursor and repaint listener ([`server.zig:1327`](../vendor/keywork/src/compositor/server.zig#L1327), [`1725`](../vendor/keywork/src/compositor/server.zig#L1725)). Desktop painting and submission select the default seat, despite an existing seat-parameterized render helper ([`server.zig:8968`](../vendor/keywork/src/compositor/server.zig#L8968), [`9095`](../vendor/keywork/src/compositor/server.zig#L9095)).

The lifetime and lock behavior need review before adaptation:

- Cursor-shape devices retain a raw seat binding without retaining the transient seat ([`cursor_shape.zig:290`](../vendor/keywork/src/compositor/wayland/cursor_shape.zig#L290)). Cursor surfaces are excluded from resource accounting even though seat deinitialization asserts they are gone ([`seat.zig:1601`](../vendor/keywork/src/compositor/wayland/seat.zig#L1601), [`345`](../vendor/keywork/src/compositor/wayland/seat.zig#L345)). These are source-level lifetime concerns, not runtime-confirmed failures.
- Lock transitions reset default-seat pointer focus and grabs. Existing transient grabs can retain previous focus. Virtual-keyboard inhibition is broader ([`server.zig:5095`](../vendor/keywork/src/compositor/server.zig#L5095)).
- Transient clicks call shared window-management logic and can alter human focus ([`server.zig:6502`](../vendor/keywork/src/compositor/server.zig#L6502)).
- Security-context filtering hides privileged globals from registered sandbox clients. Ordinary clients remain allowed. This is not controller-specific authentication ([`security_context.zig:138`](../vendor/keywork/src/compositor/wayland/security_context.zig#L138)).

The relevant transient-seat and virtual-input modules in the standalone reference are byte-identical to the monorepo copies. The monorepo has newer cursor fallback and drag handling, but neither tree completes transient focus and display integration. Useful patterns to adapt are device-to-seat routing, coordinate mapping, release of pressed inputs, and delayed seat destruction.

## Client compatibility and desktop integration

Dojo's pinned GPUI Wayland client keeps one seat. On a new `wl_seat` announcement, it releases the existing pointer, keyboard, and seat, then binds the new seat ([pinned `client.rs:1606`](https://github.com/zed-industries/zed/blob/9ab0715969e9854f2efe61a9d782e7698e5fe6d2/crates/gpui_linux/src/linux/wayland/client.rs#L1606)). Capability handling also maintains singleton devices, text-input objects, and cursor objects. Advertising another seat can disrupt human input in Dojo.

Hiding a seat from a target application prevents that application from binding the seat's input resources. Supporting both seats requires real client support. For GPUI, that includes input devices, cursor handling, text input, activation, and clipboard state. Merely ignoring the second seat preserves human input but does not make the app controllable through it.

Separate compositor focus does not provide separate application state. A client can still have one caret, shared dialogs, or its own notion of the active window. Ordinary application support must be demonstrated individually. X11 support also remains unproven because Niri launches an external [`xwayland-satellite`](../vendor/niri/src/utils/xwayland/satellite.rs#L235).

The current Home Manager configuration selects packaged Niri ([`home-manager.nix:471`](../home-manager.nix#L471)). The subtree is not automatically built or installed. Development needs a nested forked compositor and its own build environment before desktop integration.

## Recommended implementation sequence

1. Build and run the fork in a nested development session with a purpose-built client that binds both seats. Establish the baseline input and rendering behavior there.
2. Add one opt-in logical agent seat with controller lifetime tracking. Route pointer input to its handle, maintain pointer focus independently, and render its cursor with complete surface lifecycle handling. Disconnect must release pressed inputs.
3. Add explicit agent keyboard targets and focus maintenance. Agent input must not activate the human layout. Resolve popup grabs by request seat and enforce lock and inhibition behavior on every agent input path.
4. Add or verify client support, starting with GPUI. Exercise human and agent input concurrently, then characterize ordinary app and X11 compatibility.
5. Integrate the validated fork into the desktop configuration. Shared window management and capture behavior require deliberate policy rather than accidental inheritance from primary-seat paths.

The first useful milestone is two independently rendered pointers delivering input to separate targets in a nested session, followed by independent keyboard focus. Full hardware multiseat is unnecessary for that milestone. Universal application compatibility is not established by this analysis.
