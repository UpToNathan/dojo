# Compositor shortcuts while controlling a peer

Status: option A chosen and built; see docs/adr/0008-remote-input-takes-the-real-seat-only-inside-a-peers-ssh-session.md

## Problem

While Venus controls Jupiter (`dojo-mesh remote`), the keys reach Jupiter through an agent input seat. Seat keys never reach Niri's key bindings, so Super+Shift+Space and every other `binds` entry in Jupiter's config do nothing there. Since the capture helper inhibits Venus's own shortcuts, those keys now do nothing at all. The person expects the machine they are controlling to react as if they sat at it.

## Options

**A. Inject into the main seat.** The receiver feeds the remote events into Niri as if from the physical keyboard and mouse. Binds, key repeat, Mod+drag, Mod+wheel, gestures, a single real cursor instead of the seat's second one, and no "human owns this application" pauses all follow for free. It also reopens what the fork closed on purpose: virtual keyboards are disabled so that no same-user process, the AI agent included, can type as the human. Niri cannot tell an operator's ssh session from the agent's shell (both are the same user, on the same socket), so this needs an identity gate Niri cannot provide today. A new ADR and a threat model first.

**B. Resolve binds on the receiver and run their actions (recommended).** The receiver keeps track of held modifiers. When a key is pressed with a modifier held, it asks Niri whether the peer's config binds that chord. If it does, Niri runs the bound action and the key is not sent to the seat. If not, the key goes to the seat as now. Niri already runs any action for any same-user client (`Request::Action`), so this grants nothing new: it only looks the action up from the user's own config instead of the caller naming it.

Recommendation: B. It gives what was asked for (the launcher and every other bind on the peer) with no new power, and A stays open as a separate decision.

## Design for B

Niri fork (`niri-ipc`, `ipc/server.rs`, `input/mod.rs`):

- `Request::TriggerBind { evdev: u32, modifiers: Vec<BindModifier> }` answering `Response::Handled` (a bind ran) or a new `Response::NoBind`. `BindModifier` is `Shift | Ctrl | Alt | Super`.
- Keysym from the human keyboard's xkb state for that evdev code (the way `on_keyboard` gets `raw`), `ModifiersState` from the request, then the existing `find_configured_bind` over the config's binds. Resolves `Mod` the way real input does, and honours `allow-when-locked`.
- Refuses when the screen is locked, and for hardcoded/VT binds (those stay physical-only).
- Runs `do_action` as `Request::Action` does. `spawn-on-release`, `cooldown` and `repeat` are not honoured in the first version.

`dojo-mesh inject`:

- Track held modifiers from the evdev codes 42/54 (Shift), 29/97 (Ctrl), 56/100 (Alt), 125/126 (Super). Modifier keys themselves are still forwarded to the seat, so Ctrl+C in a terminal works.
- On a non-modifier key press with at least one modifier held: send `TriggerBind`. On `Handled`, swallow that press and its release. On `NoBind`, or a Niri without the request (unknown variant, said once as for `Jump`), forward to the seat.
- Operator-only, `accept-input` and lock refusals unchanged.

## Not covered

Key repeat on held binds (`Mod+Right` repeating), mouse binds (Mod+wheel, Mod+drag), touchpad gestures, and the second software cursor: all of these need option A. A bind like `quit` or `power-off-monitors` now fires on the peer; the person pressed it, but it is worth saying in the docs.

## Risks

- Both machines need the new Niri and a re-login before this works. An older peer simply keeps today's behaviour.
- Binds are resolved from the peer's layout, so a key means what it means on the peer, matching the physical position pressed.
- The release chords (Super+F11, Ctrl+Alt+Shift+Esc) never reach `inject`, so they cannot fire a bind.

## Tests

- Niri: a bind resolves from evdev and modifiers; no bind answers `NoBind`; locked refuses; `Mod` resolves; hardcoded binds are not triggered.
- dojo-mesh e2e (fake Niri): a modifier plus key triggers and is swallowed with its release; `NoBind` forwards; an old Niri forwards and says so once.
