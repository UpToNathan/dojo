# Design candidate A: one pure reducer

## Problem

`dojo-lock` must hold `ext-session-lock-v1` across several outputs. It races two PAM conversations in killable worker processes, shows one shared password on every output, and runs a power timeline that keeps the locked machine at near-zero cost. Its first frame must also be the shell's desktop frame, pixel for pixel. The rules are tangled. A display turning off must disarm the sensor. A faillock notice must stop the password worker. A win by either method must kill the other worker and start the reverse transition. Spread over GPUI views, timers and process callbacks, those rules turn into split ownership. Constraints from grounding:

- GPUI parks its frame loop when nothing is dirty. One stray `notify` per second breaks the power rule.
- Niri disconnects a client that commits before `ack_configure`. GPUI spins on a dead connection, so the process must exit.
- Only one lock surface gets keyboard focus, the one under the pointer.
- `pam_unix` failures block for 1.5 to 3 s. `pam_fprintd` sends `PAM_TEXT_INFO` and never prompts. The sensor is free again right after a kill.
- The shell's geometry (`src/geometry.rs`), surround painting (the canvas in `src/frame.rs` around line 1667), `ThemeColors` (`src/config.rs`), fonts (`src/power.rs`) and clock format (`src/clock.rs`) are all bin-private today.

## Usage (caller's view)

The edge in `src/bin/dojo-lock/app.rs` is the only caller. It stamps the time on each input, feeds it to the model, carries out the returned commands and redraws from the model's state.

```rust
use dojo::lock::{Effect, Event, EventKind, Key, LockModel, Now, Settings};

pub fn run(options: Options) -> ExitCode {
    let assets = Assets::load();                         // ThemeColors, blurred wallpaper, fonts; parsed here
    application().run(move |cx| {
        let settings = Settings { reduced_motion: cx.reduce_motion(), user: options.user.clone() };
        let (model, effects) = LockModel::new(settings, Now::read(), outputs(cx));
        let locker = cx.new(|_| Locker::new(model, assets));
        cx.on_session_lock_event({
            let locker = locker.clone();
            move |event, cx| locker.update(cx, |l, cx| l.dispatch(EventKind::from(event), cx))
        });
        locker.update(cx, |l, cx| { l.apply(effects, cx); l.sync(cx) });
    });
    EXIT.load()
}

impl Locker {
    /// The only way anything changes. Every input source ends here.
    pub fn dispatch(&mut self, kind: EventKind, cx: &mut Context<Self>) {
        let effects = self.model.update(Event { now: Now::read(), kind });
        self.apply(effects, cx);
        self.sync(cx);
    }
}

// view.rs: composed text (dead keys included) arrives through EntityInputHandler.
fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
    for c in text.chars() {
        self.locker.update(cx, |l, cx| l.dispatch(EventKind::Key(self.output, Key::Char(c)), cx));
    }
}
```

A reducer test is plain data in and data out. It needs no GPUI, Wayland or PAM:

```rust
#[test]
fn wrong_password_shakes_and_starts_a_fresh_worker() {
    let mut lock = locked_with_one_output();                        // helper: new + Locked + prompt
    type_text(&mut lock, "hunter2");
    let fx = lock.update(at(2_000, EventKind::Key(OUT, Key::Enter)));
    assert!(matches!(fx[..], [Effect::Answer { to: 1, .. }]));
    let fx = lock.update(at(4_500, worker(PW1, Report::Finished(PamOutcome::Rejected))));
    assert!(matches!(fx[..], [Effect::Spawn(WorkerId { method: Method::Password, generation: 3 })]));
    assert_eq!(lock.scene(OUT, now(4_600)).unwrap().pill().tone, Tone::Wrong);
    assert_eq!(lock.scene(OUT, now(5_000)).unwrap().pill().dots, 0);
}
```

## Shape

### Public surface of `dojo::lock`

```rust
// src/lock/mod.rs: re-exports only. model, scene, secret and pam are private modules.
pub struct LockModel { /* private: see model.rs */ }

impl LockModel {
    /// Lock requested, nothing confirmed. Returns [CloseShellSurfaces, SampleChips].
    pub fn new(settings: Settings, now: Now, outputs: Vec<Output>) -> (Self, Vec<Effect>) { todo!() }
    /// The single transition function. Total: every (state, event) pair is defined, most as no-ops.
    pub fn update(&mut self, event: Event) -> Vec<Effect> { todo!() }
    /// What one output shows at `now`. Pure. `None` for an unknown output.
    pub fn scene(&self, output: OutputId, now: Now) -> Option<Scene> { todo!() }
    /// Earliest moment the model needs a `Timer` event. `None` means sleep until input.
    pub fn next_deadline(&self) -> Option<Instant> { todo!() }
    /// The outputs that should have a lock window.
    pub fn outputs(&self) -> impl Iterator<Item = OutputId> + '_ { todo!() }
}

#[derive(Clone, Copy)] pub struct Now { pub mono: Instant, pub unix: i64 }   // wall time is for the minute and labels
#[derive(Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Debug)] pub struct OutputId(pub u64);
pub struct Output { pub id: OutputId, pub size: Size<Pixels> }               // logical px; GPUI owns scale
pub struct Settings { pub reduced_motion: bool, pub user: String }

pub struct Event { pub now: Now, pub kind: EventKind }
pub enum EventKind {
    Locked,                          // SessionLockEvent::Locked
    Finished,                        // SessionLockEvent::Finished: refused, or ended by the compositor
    Outputs(Vec<Output>),            // the full current set; diffed inside, so repeats are harmless
    Pointer(OutputId),               // pointer entered or moved on that output's lock surface
    Key(OutputId, Key),
    Worker(WorkerId, Report),        // already parsed from the worker socket
    Timer,                           // next_deadline passed; a spurious one is a no-op
    Resumed,                         // logind PrepareForSleep(false)
    Chips(HostChips),                // battery and network, sampled on request
}
pub enum Key { Char(char), Backspace, Enter, Escape, ClearLine }             // manual Debug prints Char(<redacted>)

#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Method { Fingerprint, Password }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub struct WorkerId { pub method: Method, pub generation: u32 }
impl Method { pub fn service(self) -> &'static str { todo!() } }             // "dojo-lock-fingerprint" | "dojo-lock-password"

#[derive(Debug)] pub enum Effect {
    Spawn(WorkerId),                                     // start a PAM worker process (this is "arm the sensor")
    Kill(WorkerId),                                      // SIGKILL its process group; no-op if already gone
    Answer { to: u32, secret: Password },                // password worker of that generation only; the type allows nothing else
    DisplayOff,                                          // Niri Action::PowerOffMonitors; Niri wakes on input
    SampleChips,
    CloseShellSurfaces,                                  // control::send(ControlCommand::Locking), best effort
    Unlock,                                              // cx.unlock_session()
    Exit(ExitReason),
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum ExitReason { Unlocked, Refused, Ended }

// src/lock/pam.rs: PAM vocabulary, parsed into domain terms. No libpam here.
pub enum Report { Prompt { echo: bool }, Notice(Notice), Finished(PamOutcome), Crashed }
pub enum Notice { Hint { text: String, error: bool }, PasswordPaused { remaining: Duration } }
impl Notice { pub fn parse(style: MessageStyle, text: &str) -> Self { todo!() } }   // recognises faillock's "(N minutes left to unlock)"
pub enum PamOutcome { Success, Rejected, Unavailable, MaxTries, Other(i32) }
impl PamOutcome { pub fn from_code(code: i32) -> Self { todo!() } }                // 0, 7, 9, 11
```

`Report`, `Notice` and `PamOutcome` are on the public surface because the bin's worker client builds them. They are domain types. No PAM integer or message style reaches `LockModel`.

### State (src/lock/model.rs)

Each glossary term owns one place in the state. Invalid combinations do not compile (per type-system-discipline).

```rust
pub struct LockModel {
    settings: Settings,
    outputs: BTreeMap<OutputId, Size<Pixels>>,
    active: Option<OutputId>,                 // Active output: the last output with pointer or key input
    stage: Stage,
    transition: dojo::design::MenuReveal,     // Lock transition progress: 0 = desktop frame, 1 = lock screen; reversible
    motion: Option<Motion>,                   // the one short animation in flight, if any
    chips: HostChips,
    next_generation: u32,
}

enum Stage {
    /// Lock requested and the first frame shown, compositor not yet confirmed. Typing is buffered.
    /// Workers start only on Locked, so a success can never come before the session is really locked.
    Requested { typed: Typed },
    Locked { at: Instant, password: PasswordFlow, fingerprint: Fingerprint, power: Power },
    /// Reverse transition; both workers are already dead. At its end: [Unlock, Exit(Unlocked)].
    Unlocking { by: Method },
    Exiting,
}

/// Unlock method 1. One enum, so a "checking" UI with no worker cannot exist.
enum PasswordFlow {
    Starting  { worker: u32, typed: Typed },  // spawned; faillock preauth runs before any prompt
    Prompting { worker: u32, typed: Typed },  // blocked in conv(); an Enter answers immediately
    Checking  { worker: u32 },                // answer sent; 1.5 to 3 s on failure; keys ignored
    Paused    { until: Instant },             // Password pause: no worker, so no failure reaches the tally
    Broken,                                   // worker died before prompting; the next Enter respawns it (no timer loop)
}
enum Typed { Nothing, Text(Password), Submitted(Password) }   // Submitted: Enter came before the prompt

/// Unlock method 2.
enum Fingerprint {
    Starting(u32),                            // no PAM message yet
    Listening { worker: u32, hint: String },  // pam_fprintd's first TEXT_INFO, shown as the hint
    Exhausted,                                // MaxTries or an unknown failure: re-arm on the next activity
    Disarmed,                                 // display off: the sensor is released
    Unavailable,                              // Unavailable before any message: no reader or no print. Never retried this lock
}

/// Power timeline. Activity is a key, a pointer move or Resumed.
enum Power {
    Lit { last_activity: Instant },           // active: full brightness
    Dim { since: Instant },                   // settled: one static dim frame, then nothing
    Off,                                      // display off: no clock deadline, sensor disarmed
}

struct Motion { kind: MotionKind, since: Instant }
enum MotionKind { DialShrink, Shake, RingFlash, RingFill }

const TRANSITION: Duration = Duration::from_millis(450);
const SHAKE: Duration = Duration::from_millis(400);
const RING: Duration = Duration::from_millis(300);
const SETTLE: Duration = Duration::from_secs(10);       // Lit to Dim; not in the spec, see open questions
const DISPLAY_OFF: Duration = Duration::from_secs(30);  // Dim to Off (spec recommendation)
```

The Dial is not stored anywhere. `scene()` derives it from `Fingerprint`, `Motion` and whether anything is typed. Wrong is not stored either. It is a `Shake` motion that the scene shows in the wrong tone (colour only under reduced motion). After it, the flow is already `Starting` with a fresh worker and an empty buffer.

### The reducer

```rust
pub fn update(&mut self, Event { now, kind }: Event) -> Vec<Effect> {
    let mut fx = Vec::new();
    match kind {
        Finished => { fx.extend(self.kill_all()); fx.push(Exit(if self.was_locked() { Ended } else { Refused })); self.stage = Exiting }
        Outputs(set) => self.outputs = set.into_iter().map(|o| (o.id, o.size)).collect(),   // idempotent
        Locked => if let Requested { typed } = take(&mut self.stage) {
            // TODO: the stage becomes Locked { password: Starting { pw, typed }, fingerprint: Starting(fp), power: Lit(now) }
            fx.extend([Spawn(password(pw)), Spawn(fingerprint(fp)), SampleChips]);
        },
        Pointer(out) => { self.active = Some(out); self.activity(now, &mut fx) }
        Key(out, key) => { self.active = Some(out); self.activity(now, &mut fx); self.key(key, now, &mut fx) }
        Worker(id, report) => self.worker(id, report, now, &mut fx),   // a stale generation falls through to nothing
        Timer => self.advance(now, &mut fx),
        Resumed => self.activity(now, &mut fx),
        Chips(c) => self.chips = c,
    }
    fx
}
```

Password rules, all in `fn key` and `fn worker`:

- `Char` and `Backspace` edit `Typed::Text`. The first character starts `DialShrink`. A full buffer drops the character.
- `Enter` on `Starting` stores `Submitted`. `Enter` on `Prompting` returns `Answer` and moves to `Checking`. `Escape` and `ClearLine` drop the buffer, which zeroes it.
- `Prompt` on `Starting` with `Submitted(s)` returns `Answer` and moves to `Checking`. Otherwise it moves to `Prompting`.
- `Finished(Success)` moves to `Unlocking { by: Password }`. It returns `Kill(fingerprint)` and reverses `transition`.
- `Finished(Rejected)` in `Checking` starts `Shake` and moves to `Starting` with a new generation. It returns `Spawn`.
- `Notice(PasswordPaused { remaining })` returns `Kill` and moves to `Paused { until: now + remaining }`. When the pause ends, `advance` moves to `Starting` and returns `Spawn`. pam_faillock is the only authority on pauses. The model keeps no count of its own.
- `Crashed` or `Other` before any prompt moves to `Broken`.

Fingerprint rules:

- The first `Notice::Hint` moves to `Listening`. Later hints replace the text and start `RingFlash`.
- `Success` starts `RingFill`, moves to `Unlocking { by: Fingerprint }` and returns `Kill(password)`.
- `Unavailable` from `Starting` moves to `Unavailable`. The hint stays the password hint and no error is shown.
- `Unavailable` from `Listening` is a timeout. The worker is respawned if power is not `Off`, otherwise the state becomes `Disarmed`.
- `MaxTries`, `Rejected`, `Other` and `Crashed` move to `Exhausted`.
- A `Prompt` from the fingerprint worker is a misconfigured stack. It returns `Kill` and moves to `Unavailable`. The password is never sent there, and `Answer` can only address a password generation.
- Fingerprint keeps working during a password pause, as ADR 0002 implies. That is an owner decision.

Power rules, in `fn activity` and `fn advance`:

- Activity in `Dim` or `Off` moves to `Lit`. If the sensor is `Disarmed` or `Exhausted`, activity returns `Spawn(fingerprint)`.
- When `last_activity + SETTLE` passes, power moves to `Dim`.
- When `since + DISPLAY_OFF` passes, power moves to `Off` and returns `[DisplayOff, Kill(fp)]`. The sensor becomes `Disarmed`.
- Niri itself turns monitors back on for any input. `vendor/niri/src/input/mod.rs:144-158` calls `activate_monitors` and does not return early, so the waking key reaches the lock surface and is typed (read from code, not yet run).

`next_deadline` is the minimum of the following, and nothing else:

- The end of the transition, when it is moving.
- `motion.since + duration`.
- The power deadline (`SETTLE` or `DISPLAY_OFF`).
- `Paused.until`.
- The next wall-clock minute from `now.unix`, but only while power is not `Off`.

With the display off, the only wakeup left is the end of a pause. The minute `Timer` also returns `SampleChips`.

### Projection: windows, frames and the timer are derived, not commanded

The model does not return a `Redraw` effect. After every `update`, `Locker::sync` projects state:

```rust
fn sync(&mut self, cx: &mut Context<Self>) {
    // TODO: open a WindowKind::SessionLock window for each id in model.outputs() that lacks one; close the rest.
    // TODO: for each window, s = model.scene(id, Now::read()). If s != window.last, store it and notify that window.
    // TODO: if model.next_deadline() changed, replace the single timer task. When it fires: dispatch(Timer).
}
```

`Scene: PartialEq` holds no `Instant`, only what is painted: the minute-resolution label, dot count, tones, geometry. So the zero-frames rule is structural. A timer, chips sample or pointer move that changes nothing visible commits nothing. A per-minute tick changes only the label, so it is exactly one frame. Each keystroke redraws every output's pill, so the dots match everywhere. Inactive outputs show `Role::Passive`, which carries no dots, so their scene does not change and they get no frame. While `scene.animating`, the view calls `window.request_animation_frame()`. Because animating is true only inside a `Motion` or a moving transition, animation cannot loop.

An effect that could also be expressed as a projection would give two ways to redraw, and every new visual field would need a matching `Redraw` emission. Projection keeps one rule: if the paint changed, there is a frame (per laziness-protocol: consolidate decisions).

```rust
// src/lock/scene.rs
#[derive(Clone, PartialEq, Debug)]
pub struct Scene { pub frame: FrameShape, pub clock: ClockText, pub dim: f32, pub role: Role, pub animating: bool }
pub struct FrameShape { pub opening: Bounds<Pixels>, pub radius: Pixels, pub surround_opacity: f32 }
pub struct ClockText { pub label: String, pub centre: Point<Pixels>, pub size: Pixels }
pub enum Role { Passive, Active { date: String, dial: Dial, pill: Option<Pill>, hint: String, chips: Chips } }
pub struct Dial { pub diameter: Pixels, pub ring: f32, pub tone: Tone }
pub struct Pill { pub dots: usize, pub tone: Tone, pub offset_x: Pixels, pub dashed: bool, pub countdown_minutes: Option<u32> }
pub enum Tone { Normal, Wrong, Muted }

/// progress 0 returns exactly geometry::desktop_opening(viewport) and OPENING_RADIUS,
/// with the clock at geometry::top_bar_clock_bounds' centre in the 11 px top-bar label.
fn transition_shape(viewport: Size<Pixels>, progress: f32, reduced_motion: bool) -> (FrameShape, ClockText) { todo!() }
```

The pause countdown is shown in whole minutes, rounded up, so it rides the minute tick. A seconds countdown would mean a frame every second for up to ten minutes.

### Password secrecy

The mechanism is a fixed-capacity zeroizing buffer in the locker, and a `SOCK_SEQPACKET` socket on fd 3 to a re-executed worker.

```rust
// src/lock/secret.rs
/// No Clone, Display or Serialize. Debug prints "Password(<redacted>)".
/// Fixed capacity: a growing String would realloc and leave old copies in freed memory.
pub struct Password { bytes: Box<[u8; Password::CAPACITY]>, len: usize, chars: usize }
impl Password {
    pub const CAPACITY: usize = 512;
    pub fn new() -> Self { todo!() }                    // also mlock()s the box so it cannot reach swap
    pub fn push(&mut self, c: char) -> bool { todo!() } // false when full
    pub fn pop(&mut self) { todo!() }
    pub fn char_count(&self) -> usize { todo!() }       // the dots
    /// The only read of the secret. One caller, worker::send_answer, so a leak is one grep away.
    pub fn expose(&self) -> &[u8] { todo!() }
}
impl Drop for Password { fn drop(&mut self) { zeroize::Zeroize::zeroize(&mut *self.bytes) } }
```

Spawning a worker (`src/bin/dojo-lock/worker.rs`):

- Make a `socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC)`.
- `Command::new("/proc/self/exe").args(["--pam-worker", "password"])`, with `env_clear()`, then only `LC_MESSAGES=C.UTF-8` and `PATH`.
- In `pre_exec`: `dup2(child_end, 3)`, `setsid()`, `prctl(PR_SET_PDEATHSIG, SIGKILL)` and `prctl(PR_SET_DUMPABLE, 0)`.

argv carries only the service name. The environment is rebuilt from nothing. `Answer` is written as one datagram, `b'A' ++ secret.expose()`, and the `Password` drops (zeroes) right after. The worker `recv`s into a zeroizing stack buffer. It copies into `malloc` memory with a trailing NUL for PAM's `resp`, which PAM owns and frees. It zeroes its own buffer. Both processes set `PR_SET_DUMPABLE 0` and `RLIMIT_CORE 0`, so the secret is in neither a core dump nor a same-user ptrace. Nothing in either process logs a `Key`, an `Effect::Answer` or a datagram. Their Debug impls are redacted, so even a careless `{:?}` is safe. `LC_MESSAGES=C` keeps faillock's text parseable. In exchange, PAM hints come in English.

Messages from the worker are tagged datagrams on the same socket: `P` (prompt, echo byte), `I` / `E` (text), `D` (i32 code). Both ends live in `worker.rs`, so the format is defined once. A reader thread per worker parses into `Report` and sends `(WorkerId, Report)` over an `async_channel`. A GPUI task drains it into `dispatch`. EOF without `D` becomes `Crashed`. `Kill` is `killpg(pgid, SIGKILL)` plus a reap on the reader thread. It is a no-op for an unknown id, so it is idempotent (per make-operations-idempotent). Real stacks have no long-lived children, and PDEATHSIG also covers a crashed locker.

### Shell geometry shared through the lib

These bin-private files move into the lib unchanged, and `src/main.rs` swaps `mod x;` for `use dojo::x;`. That is the same trick it already uses for `control`, `design` and `speech`. So every `crate::config::…` and `crate::geometry::…` path in the 11 shell files keeps compiling:

- `src/config.rs` (ThemeColors, ConfigStore). The locker resolves colours exactly as the shell does: config.toml first, then the `current/dojo.json` overlay.
- `src/geometry.rs`.
- `src/power.rs`, for the font family and features and for `read_status` and `battery_glyph` (chips).

What moves out of `src/frame.rs` into the lib:

- `geometry.rs` gains `pub const OPENING_RADIUS: f32 = 28.` (from frame.rs `CORNER_RADIUS`). It gains `pub fn desktop_opening(viewport) -> Bounds<Pixels>`, the inset arithmetic now inlined in the canvas closure. It gains `pub fn top_bar_clock_bounds(viewport, controls_width)` (from frame.rs `clock_bounds`).
- `design.rs` gains `pub fn paint_desktop_frame(window, bounds, opening, radius, surround: Rgba, border: Rgba)`. This is the canvas body at frame.rs:1670-1716: surround fill, `paint_frame_shadow`, border stroke. `append_rounded_rectangle{,_reverse}` move with it as private helpers. The shell's canvas becomes one call to it, plus its menu extension.
- A new `src/wall_clock.rs` gets `top_bar_label(unix)`, which is `format_local` with `"%a, %b %d  %H:%M"`. It also gets `dial_time` and `date_line`. The shell's `clock.rs` keeps its Global and calls `top_bar_label`.

So the shell and the locker run the same function for the same pixels. Drift would need an edit to a function both of them call.

## Module map

Lib (`src/`), pure, with no Wayland and no PAM:

- `lib.rs` adds `pub mod config; pub mod geometry; pub mod lock; pub mod power; pub mod wall_clock;`.
- `lock/mod.rs` holds only re-exports. Internals are private modules, so an outside import of `lock::model::Stage` fails to build.
- `lock/model.rs` is the state, `update`, `next_deadline` and the timing constants, with most of the tests.
- `lock/scene.rs` is `Scene`, `scene()` and `transition_shape`.
- `lock/secret.rs` is `Password`.
- `lock/pam.rs` is `Report`, `Notice::parse` and `PamOutcome::from_code`.
- `geometry.rs`, `config.rs` and `power.rs` are moved from the bin. `design.rs` gains `paint_desktop_frame`. `wall_clock.rs` is new.

Bin (`src/bin/dojo-lock.rs` plus `src/bin/dojo-lock/`, with `#[path]` modules as in `dojo-agent.rs`):

- `dojo-lock.rs` parses args into `Lock`, `Worker(service)` or `Listen`, and hardens the process. `--pam-confdir DIR` is accepted only under `cfg(debug_assertions)`.
- `app.rs` is `Locker` (`dispatch`, `apply`, `sync`) and the input sources: session lock events, display observer, logind resume thread, worker channel and timer.
- `view.rs` is `LockView`. It paints a `Scene` with `paint_desktop_frame`, the dial, pill and chips. It holds the input handler (text to `Key::Char`), the key bindings and mouse move to `Pointer`. It maps `Tone` to `ThemeColors`.
- `worker.rs` holds the parent side (spawn, kill, reader thread, wire format) and the worker side (`run(service)`: an `extern "C"` libpam block with `pam_start`, or `pam_start_confdir` in debug builds, plus the conv callback and fd 3 framing).
- `host.rs` holds `PowerOffMonitors` over `niri_ipc`, chips sampling from sysfs, `control::send(Locking)` and `Assets::load`.
- `listen.rs` is issue 04. It is a resident, non-GPUI logind loop that turns `Lock` and `PrepareForSleep(true)` into `systemctl --user start dojo-lock.service`. It holds a delay inhibitor until `LockedHint`. Every trigger (menu, keybind, stasis, suspend) goes through `loginctl lock-session`, so there is one way to lock.

Existing files that change:

- `Cargo.toml` adds `zeroize = "=1.9.1"`. That version is already in Cargo.lock, pulled in by another dependency. The bin is auto-discovered like `dojo-agent`.
- `src/main.rs` changes as described above.
- `src/frame.rs` calls the lib for the surround and clock bounds.
- `src/clock.rs` calls `wall_clock`.
- `src/control.rs` and `src/shell.rs` get `ControlCommand::Locking`, which closes the launcher, tray menus and pickers.
- `src/system_menu.rs` gets a Lock row.
- `home-manager.nix` gets `systemd.user.services.dojo-lock` with `Restart=on-failure`, `RestartSec=0`, `RestartPreventExitStatus=3` and `Environment=VK_DRIVER_FILES=…radeon_icd…`, which pins the driver in config rather than code. It also gets the listener unit, the stasis action and the keybind.
- `system-config/pam.d/dojo-lock-{password,fingerprint}`, taken from `spikes/pam/recommended/`.
- `tools/agent-input/run-nested.sh` gains `--lock` and a new `run-lock.py`.

## Crash and exit

`Finished` before `Locked` means another locker is alive. That is `Refused` and exits with status 3, which `RestartPreventExitStatus` will not restart. `Finished` after `Locked` exits 0 as `Ended`. After `Unlock`, the edge calls `cx.unlock_session()` and quits with 0. A lost Wayland connection ends the process inside the patched GPUI (issue 01) with a non-zero code. So does a panic. systemd restarts the locker, the compositor keeps the session locked in the meantime, and the new instance takes over the dead lock. Workers die with the locker through PDEATHSIG. A restart is a fresh `LockModel::new`, so there is nothing to reconcile. The buffer is lost on purpose.

## Testing seams

- **Reducer** (`cargo test --lib`). Scripted event sequences are asserted against literal effects and scenes. They cover the following: an Enter before the prompt is answered on the prompt; a rejection shakes and respawns; a faillock notice pauses with no worker until the pause ends; a fingerprint win kills the password worker and the reverse is true; an immediate Unavailable is never retried; a timeout is re-armed; display off disarms and activity re-arms; a stale generation is ignored; `Finished` before and after `Locked` exits differently; a Timer that changes nothing leaves every `scene` equal; the display-off deadline set has no minute deadline.
- **Scene and geometry.** `transition_shape(1920x1080, 0.)` returns opening `(64,24)-(1908,1068)`, radius 28, and the clock at the centre of `top_bar_clock_bounds`. The same `desktop_opening` is asserted from the shell side. Passive outputs carry no dots.
- **Boundaries.** `Notice::parse` is checked against the strings the spike captured: "Place your right index finger on the fingerprint reader", "Verification timed out", and faillock's lock message. `PamOutcome::from_code` is tested too. `format!("{:?}", Password)` and `Key::Char` are checked to be redacted.
- **Fake PAM** (`tests/lock_worker.rs`). The test runs `env!("CARGO_BIN_EXE_dojo-lock") --pam-worker password --pam-confdir tests/fixtures/pam`. The fixture password stack is `pam_exec.so expose_authtok check_fake_pw.sh`, ported from `spikes/pam/bin/helpers`. The fingerprint stand-ins are `sleep_ok.sh` and `pam_deny`. It asserts success and rejection, and that a kill leaves no process in the group. While the worker waits in the prompt, the test reads `/proc/<pid>/cmdline` and `/proc/<pid>/environ` and asserts that the password bytes appear in neither. A second test checks that every `Method::service()` has a file in `system-config/pam.d/`, so the two lists cannot drift.
- **Nested Niri.** `run-nested.sh --lock` checks the following: `locked` on every output, kill and restart, agent keys rejected, and that `ChangeVt` escapes. Real-finger and real-password checks stay human-run under `spikes/pam/`.

## Synthesis decision

Filled in by arena.

## Tradeoffs accepted

- We accept a per-character `dispatch` and a full scene diff per event, in exchange for one entry point and structural frame gating. Both are microseconds against a human typing.
- We accept that GPUI's transient keystroke and IME strings are not zeroized. In exchange, the vendored GPUI does not need a secret-aware input path. Our own buffers are covered.
- We accept that workers start about 100 ms late, on `Locked`, so the model has no "authenticated before locked" state.
- We accept one idle password worker blocked in the prompt for the whole lock, so faillock's verdict arrives before the user presses Enter.
- We accept `gpui::Bounds`, `Size` and `Pixels` in the lib's scene. They are plain data the shell's paint code already uses, and a private rect type would only add conversions.
- We accept English PAM messages, so the faillock pause can be parsed rather than counted by a second authority.

## Alternatives considered

- **One object per concern** (an `AuthRace`, a `PowerTimer`, per-output view state). Each surface is smaller, but rules like "display off disarms the sensor" and "a win kills the loser and starts the reverse" span two objects. The edge would coordinate them, so the policy leaks into the GPUI layer and is untestable there. Its interface is wider and shallower.
- **State in GPUI entities** (one per output). This needs password copies in sync across outputs, and every rule needs a GPUI test context. It fails the shared-state test (per separate-before-serializing-shared-state).
- **A model that returns `Redraw` effects.** It duplicates the scene's knowledge in the reducer and makes stray frames easy to add. It was rejected for projection.
- **Password over stdin pipe and JSON, as in the spike.** serde would copy the secret into Strings, and it needs a line protocol for both directions. A seqpacket socket on fd 3 gives message boundaries with no codec.

## Red-flag screen

- **Shallow module.** No. Five methods hide every phase, race, power and transition rule.
- **Information leakage.** None. PAM codes and texts are parsed only in `pam.rs`, and the wire format lives only in `worker.rs`.
- **Temporal decomposition.** None. Modules are grouped by knowledge (policy, appearance, secret, PAM vocabulary), and edge files by host system.
- **Pass-through.** None. `dispatch` adds the time and the effect interpretation.
- **Split ownership.** None. The model owns intent and the edge owns OS handles keyed by ids the model issues. Generations make a stale handle harmless.
- **Two ways.** None. Redraw only happens through `sync`, unlock only through `Effect::Unlock`, and a trigger only through `loginctl`.
- **Importable internals.** No. They are private modules, and `expose` has one caller.
- **Hand-synced list.** No. The service names against `pam.d` are checked by a test, and geometry constants have one home.

## Open questions and risks

- Frame 0 is pixel-identical for the surround, shadow, border and clock. Should it also redraw the top-bar controls, tray icons and launcher button? The locker cannot see the shell's live tray. The recommendation is to drop them, and to mask the parity screenshot with the lib's own control rects.
- Does "interrupting unlock mid-way (a keypress) reverses cleanly" mean that a key during the lock-in transition snaps to typing? Here a successful unlock is committed and a key cannot re-lock.
- `SETTLE` = 10 s from Lit to Dim is not in the spec. Is 10 s right?
- Should fingerprint stay live during a password pause? This design keeps it live.
- Does `cx.unlock_session()` followed by an immediate quit flush `unlock_and_destroy`, or does the edge need a roundtrip first?
- Is `PowerOffMonitors` over IPC honoured while locked, and does the waking key reach the surface? This was read from Niri code, not run.

## Next implementation step

Write `src/lock/{mod,model,secret,pam}.rs` with these types and `todo!()` bodies. Then write the reducer test table for the password-only slice of issue 02 and make it pass.
