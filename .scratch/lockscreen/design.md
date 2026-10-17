# Lock screen design (synthesis)

Synthesised from [design-a.md](design-a.md) (single reducer) and [design-b.md](design-b.md) (small modules plus a deep PAM module). Where this file is silent, the candidate it names wins. Vocabulary is in [GLOSSARY.md](../../GLOSSARY.md).

## Shape

**Policy is one pure reducer, mechanism is a separate deep module.** The reducer decides what should be running and what the screen shows. The worker module owns processes and nothing else.

| Piece | Source | Owns |
| --- | --- | --- |
| `lock::LockModel` (`update(Event) -> Vec<Effect>`, `scene`, `next_deadline`, `outputs`) | A | Every lock rule: stages, password flow, fingerprint state, power timeline, active output, unlock sequence |
| `lock::Scene` (`PartialEq`, no timestamps) | A | Exactly what is painted. A window redraws only when its scene changed, so zero frames when idle is structural |
| `lock::power` table `rules(PowerPhase)` | B | Settle after 5 s, display off 30 s after settle, minute tick, sensor arming |
| `session::transition` pure `Layout` interpolation | B | Shell frame to dial geometry for every `Occasion`, using `design::MenuReveal` for progress. Lives outside `lock` because the shell uses it too |
| `session::message` pure selection of the greeting or farewell for an `Occasion` | new | Pools and rules in [greetings.md](greetings.md) |
| `session::Occasion` (Lock, Unlock, Login, Logout, Suspend, Hibernate, Restart, Shutdown) | new | Direction (closing or opening), message pool and action for each occasion |
| `lock::secret::Password` fixed 512 byte zeroizing buffer, redacted Debug, one `expose` caller | A | The shared password |
| `lock::pam` vocabulary (`Report`, `Notice::parse`, `PamOutcome::from_code`) | A | PAM codes and text parsed once, run with `LC_MESSAGES=C` |
| `lock::worker` parent side (spawn, kill tree, answer, report channel) | B | Process mechanics, no policy |
| `dojo-lock-pam` helper bin | B | PAM calls via `dlopen("libpam.so.0")`, links no GPUI |
| `dojo-lock` bin: `app` (edge), `view` (paint), `host` | A | GPUI and Wayland edge, input to `Event`, effects out |

## Decisions where the candidates disagreed

- **Worker process.** A separate helper binary `dojo-lock-pam`. The process that handles the password links no GPUI and cannot initialise Vulkan. The wire is one `SOCK_SEQPACKET` socketpair on fd 3 with tagged datagrams (A), so there is no JSON and no serde copy of the secret. argv holds only the service name, the environment is rebuilt from nothing, the PAM user comes from `getpwuid`, and both ends set `PR_SET_DUMPABLE 0`, `RLIMIT_CORE 0` and `PDEATHSIG`.
- **Password pause.** `pam_faillock` is the only authority (A). The model keeps no attempt count. It parses faillock's "minutes left" notice and holds no worker until the pause ends. Fingerprint stays live during a pause (spec default).
- **Power constants.** Settle 5 s after the last activity, display off 30 s after settle (B's table). Spec amendment below.
- **Exit codes.** 0 for unlocked, refused (another locker is alive) and ended. 75 for a lost connection so `Restart=on-failure` restarts it. A `locked-at` marker in `$XDG_RUNTIME_DIR/dojo/` (B) lets a restarted locker skip the morph and keep the lock-age chip honest.
- **Shared drawing.** Move `config.rs`, `geometry.rs` and `power.rs` into the lib whole, and swap `mod x;` for `use dojo::x;` in `main.rs` (A). Add `design::paint_desktop_frame`, `design::shell_clock_element`, `geometry::{desktop_opening, top_bar_clock_bounds, OPENING_RADIUS}` and a pure `wall_clock` for the clock text. The shell calls the same functions, so pixel parity is by construction.
- **Theme.** Load `ThemeColors` exactly as the shell does and read the `error` colour from `palette.json`, with a constant fallback.
- **Triggers.** Every trigger goes through `loginctl lock-session`. Prefer stasis for idle and lock-before-suspend; build a listener only if stasis cannot hold a delay inhibitor.
- **First frame.** The shell closes first (see below), so the locker's first frame is the closed state the shell ended on. It draws the same surround, dial and clock through the shared functions, with the ring, date, hint and chips at zero alpha, then fades them in over 250 ms (`REVEAL`) from `Locked`. It cannot see the shell's tray or controls, so they disappear at lock. The parity test masks those rectangles.

## Spec amendments

- **Ring behaviour.** PAM gives no signal for a touch before the result, and ADR 0002 forbids trusting a fingerprint match the locker checks itself. So the ring is static while the sensor is armed, flashes on each PAM hint (such as a no-match), and completes on success. It does not fill during a read. A read-only subscription to fprintd's `VerifyStatus`, used for the UI only, is deferred.
- **Display-off timeline.** Lit for 5 s after the last activity, dim and static, display off 30 s later.
- **Unlock is irreversible once authenticated.** A key press or touch during the greeting or the opening skips ahead and never cancels or re-locks.
- **Lock sequence.** The first 900 ms of `Locked` carry the farewell the shell left in `lock-handoff`, shown over the resting dial. It is part of the scene, delays nothing, and any key skips it.
- **Unlock sequence.** `Unlocking` has two phases: the greeting (about 950 ms with its fades), then the opening transition (450 ms), then `[Unlock, Exit]`.

## Session transitions across occasions

The shell draws every closing, because its frame is a layer surface above the windows and closing the opening hides the workspace with no fork change. A resident `SessionTransition` animator in the shell plays the closing with the farewell, then runs the occasion's action through an injectable runner (so tests record the action instead of powering the machine off). The locker draws only the unlock opening, starting from the closed state the shell ended on, and the shell draws the login opening. For a lock the locker is started `LOCKER_LEAD` (160 ms) before the closing ends, so the compositor's swap lands on a finished closing; a slower locker only makes the cover hold the closed frame until the `locked-at` marker (or logind's `LockedHint`, set by the locker on the display session) confirms, and a watchdog restarts it once 700 ms after the closing. The lock's farewell is not drawn by the shell: it travels in `lock-handoff` and the locker plays it over the resting dial. The locker therefore needs no lock-side morph, only a closed first frame whose own parts fade in. `session::transition` and `session::message` are shared by the shell and the locker and carry no GPUI context.

Consequences for earlier decisions: the locker's first frame is the closed state, not the shell state, so the takeover marker only matters for lock age. Issue 08 shrinks to the unlock fade-in. The shell gains a control command (`dojo --leave <occasion>`) used by the system menu, the keybind and the triggers.

## Order of work

1. Shared drawing extraction (scaffold; the shell must render unchanged).
2. PAM worker mechanism and helper, with the spike scenarios as tests, in parallel with 1.
3. The pure core: model, scene, transition, power, greeting, secret, pam vocabulary.
4. The `dojo-lock` binary edge on top of issue 01's GPUI window kind.
5. Fingerprint, visuals, triggers, power verification, transition polish.

## Open risks carried forward

- Waking keypress delivery while monitors are off is read from Niri code, not run.
- `cx.unlock_session()` followed by quit must flush `unlock_and_destroy`. Issue 01 flushes explicitly.
- libpam scrubbing of echo-off replies in 1.7.3 is unverified.
- Real no-match messages from `pam_fprintd` have not been seen, so their parsing is unverified.
