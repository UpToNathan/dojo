# Design candidate B: `dojo-lock` as small modules, one body of knowledge each

One deep module (`UnlockRace`) owns everything about PAM processes. Six shallow pure modules each own one rule set (password buffer, attempt pause, power timeline, dial view, transition geometry, theme load). A thin `LockApp` at the GPUI/Wayland edge wires them through typed channels. There is no central reducer and no `Msg` enum.

## Problem

`dojo-lock` takes `ext-session-lock-v1` (ADR 0001), races a fingerprint and a password PAM conversation (ADR 0002), and must cost nothing while locked. Constraints from grounding that shape the design:

- Both PAM conversations block and cannot be cancelled in-process, so they must be separate processes. `pam_unix` failures take 1.5 to 3 s inside `pam_authenticate`. `pam_exec` children `setsid()` and escape a process-group kill.
- `src/frame.rs`, `src/config.rs` (`ThemeColors`) and `src/geometry.rs` are bin-private (`mod frame;` in `main.rs`). The locker is a second bin, so the first lock frame can only match the shell if drawing and constants move into the lib.
- GPUI parks its frame loop when nothing is dirty. `speech::AnimationSchedule {Idle, Animate, WakeAt}` and `design::MenuReveal` (reversible, eased) already encode the repo's idiom for event-only animation.
- The spec says `palette.json`, but the shell's `ThemeColors` parses `dojo.json`. `palette.json` is the only file holding the `error` colour the wrong state needs.

## Usage (caller's view)

```sh
# dojo-lock.service: ExecStart=dojo-lock  Restart=on-failure  KillMode=control-group
# exit 0 = unlocked, or refused because another locker is alive.  exit 75 = lost Wayland connection (restart).
systemctl --user start dojo-lock.service        # shell menu, keybind, loginctl handler, sleep inhibitor all do this
```

```rust
// src/bin/dojo-lock.rs
fn main() -> ExitCode {
    let origin = lifecycle::claim_marker(&runtime_dir())?;                 // Fresh | Takeover { since }
    let theme = LockTheme::load(&Paths::user_default()?.root.join("current")); // read once, never reloaded
    let (race, events) = UnlockRace::start(RaceConfig::production()?)?;    // spawns both workers
    let exit = app::run(theme, origin, race, events);                      // GPUI event loop
    lifecycle::exit_code(exit)
}

// LockApp: every handler is "ask the module that owns the rule, then notify".
fn on_key(&mut self, key: Key, now: Instant, cx: &mut Context<Self>) {
    self.power.touch(now);                                  // PowerClock
    match key {
        Key::Text(t) if self.attempts.gate(now).is_open() => self.entry.push(&t),
        Key::Backspace => self.entry.pop(),
        Key::Enter => if let Some(s) = self.entry.take() { self.race.submit(s); self.checking = true; },
        _ => {}
    }
    self.after_change(now, cx);                             // apply power rules, rearm one timer, cx.notify()
}
fn on_unlock_event(&mut self, ev: UnlockEvent, now: Instant, cx: &mut Context<Self>) {
    match ev {
        UnlockEvent::Hint { method: Method::Fingerprint, text } => { self.fp = FpLight::Reading; self.hint = Some(text) }
        UnlockEvent::PasswordRejected => { self.attempts.record_wrong(now); self.wrong_since = Some(now); self.checking = false }
        UnlockEvent::Unlocked(_) => self.begin_unlock(now),  // reveal.set_open(false); unlock_session() when it finishes
        UnlockEvent::FingerprintUnavailable | UnlockEvent::FingerprintGaveUp => self.fp = FpLight::Off,
        _ => {}
    }
    self.after_change(now, cx);
}
// Per-window render: pure model in, paint out.
let vs = model.view_state(self.output, now);          // ViewState
let dial = view::dial(&vs, now);                      // Dial (what the renderer draws)
let layout = transition::layout(viewport, progress, style); // Layout (shell frame -> dial)
```

## Shape

Data first. Each type is owned by exactly one module and every phase is derived, never stored twice (per model-the-domain: a structure per body of knowledge).

### Glossary terms to types

| Term | Type | Module |
|---|---|---|
| Unlock method | `enum Method { Fingerprint, Password }` with `const fn service(self) -> &'static str` as the only place service names appear | `unlock` |
| Dial | `struct Dial` (pure output of `view::dial`) | `view` |
| Active output | `OutputRole::{Active, Dim}` from `model.active == output` | `view` |
| Password pause | `Attempts` + `Gate::{Open, Paused { until }}` | `pause` |
| Lock transition | `Layout` + `reveal: design::MenuReveal` (reused, already reversible) | `transition` |
| Session lock | `ExitReason`, `Origin` | `lifecycle` |

### `unlock`: the deep module

```rust
pub enum SensorArming { Armed, Disarmed }
pub enum UnlockEvent {                       // domain events only: no pam rc, no JSON, no fds
    Hint { method: Method, text: String },   // PAM TEXT_INFO/ERROR_MSG verbatim
    FingerprintUnavailable,                  // result with no message first: no reader or no print. Never retried.
    FingerprintGaveUp,                       // timeout or max-tries. Restarts only on the next set_sensor(Armed)
    PasswordChecking,                        // Secret reached the worker; the 1.5-3 s PAM delay is running
    PasswordRejected,
    Unlocked(Method),                        // invariant: every worker and descendant is already dead, sensor released
    Broken(String),                          // helper missing or password worker crash-looping
}
pub struct RaceConfig { pub helper: PathBuf, pub services: [&'static str; 2],
                        #[cfg(feature = "fake-pam-stack")] pub confdir: Option<PathBuf> }
impl RaceConfig { pub fn production() -> anyhow::Result<Self> }   // helper = sibling of current_exe(), checked at the boundary

pub struct UnlockRace { commands: Sender<Command>, coordinator: Option<JoinHandle<()>> }
impl UnlockRace {
    pub fn start(cfg: RaceConfig) -> anyhow::Result<(Self, async_channel::Receiver<UnlockEvent>)>;
    pub fn submit(&self, secret: Secret);              // buffered if the prompt has not arrived yet
    pub fn set_sensor(&self, arming: SensorArming);    // Disarmed kills the fingerprint worker; Armed after GaveUp restarts it
}
impl Drop for UnlockRace { /* Command::Shutdown, kill_tree both workers, join with 1 s bound */ }
```

The coordinator thread owns all mutable race state (workers, `awaiting_prompt`, `queued: Option<Secret>`, `fp_state`). Nothing else touches it (per separate-before-serializing-shared-state: no lock, one owner, commands in, events out).

```rust
fn coordinate(cfg, cmds: Receiver<Command>, wake: OwnedFd, out: Sender<UnlockEvent>) {
    let mut pw = Worker::spawn(&cfg, Method::Password); let mut fp = None; let mut queued = None;
    // loop: rustix::event::poll([wake, pw.stdout, fp.stdout]) blocks, so an idle race costs zero wakeups
    // ready worker -> wire::read_events():
    //   Msg(text)      -> out.send(Hint { method, text })
    //   Prompt         -> w.awaiting = true; if let Some(s) = queued.take() { w.reply(&s); out.send(PasswordChecking) }
    //   Result(Success)-> let loser = other worker; reaper::kill_tree(loser); reaper::reap(winner);
    //                     out.send(Unlocked(method)); return          // first success wins, exactly once
    //   Result(AuthErr), Password -> out.send(PasswordRejected); pw = Worker::spawn(..)  // fresh PAM handle per attempt
    //   Result(AuthinfoUnavail), Fingerprint, saw_no_message -> FingerprintUnavailable (sticky, no restart loop)
    //   Result(MaxTries | AuthinfoUnavail after a message | AuthErr), Fingerprint -> FingerprintGaveUp
    //   EOF without result: password -> respawn up to 3 in 10 s then Broken; fingerprint -> FingerprintUnavailable
    // cmds: Submit(s) -> if pw.awaiting { pw.reply(&s); out.send(PasswordChecking) } else { queued = Some(s) }   // s dropped = zeroized
    //       Sensor(Armed) -> spawn fp unless Unavailable/Running;  Sensor(Disarmed) -> kill_tree(fp)
    //       Shutdown -> kill both; return
}
// private submodules, unreachable from outside: wire (framing), reaper (proc-tree kill), worker (child + helper entry), pam (dlopen)
mod reaper {
    pub(super) fn descendants(table: &ProcTable, root: Pid) -> BTreeSet<Pid>;  // pure, tested on fake tables
    pub(super) fn kill_tree(root: Pid);   // snapshot descendants BEFORE signalling, SIGTERM, then SIGKILL, 500 ms each, verify via /proc
}
pub fn run_helper() -> !;                 // the only public symbol of the worker side; called by src/bin/dojo-lock-pam.rs
```

**Worker process: a separate helper binary `dojo-lock-pam`**, not a re-exec of `dojo-lock`. The helper links no GPUI or wgpu, so two of them add little RSS for the whole lock. A refactor of `dojo-lock`'s `main` cannot make the workers initialise Vulkan. The helper body lives in the lib as `unlock::worker`, so the wire types stay private to `unlock`; the bin is `fn main() { dojo::lock::unlock::run_helper() }`. libpam is loaded with `dlopen("libpam.so.0")` through `libc` (4 symbols), so no other Dojo bin gains a link-time libpam dependency. Helper startup: `prctl(PR_SET_PDEATHSIG, SIGKILL)` (a crashed locker cannot leave a worker holding the sensor), `prctl(PR_SET_DUMPABLE, 0)`, `setsid()`, dup stdin/stdout to CLOEXEC fds then point 0/1 at `/dev/null` (PAM children inherit nothing), `pam_start_confdir`/`pam_start` for the current uid's user from `getpwuid`, never from argv. argv is the service name only, checked against `Method::service()`.

**Password path (no argv, env, log, Debug):**

```rust
pub struct Secret(Zeroizing<Vec<u8>>);        // private field; no Clone, Display, Serialize. Debug prints "Secret(<redacted>)"
impl Secret { pub(in crate::lock) fn as_bytes(&self) -> &[u8] }   // only unlock::wire can read it
pub struct PasswordEntry { buf: Zeroizing<Vec<u8>> }              // capacity fixed at MAX_SECRET_BYTES (256): push never reallocates,
impl PasswordEntry {                                              // so no stale unzeroed copy is left behind
    pub fn push(&mut self, text: &str);       // silently ignored when it would exceed capacity
    pub fn pop(&mut self);                    // removes one char, zeroes its bytes
    pub fn dots(&self) -> u16;                // the only thing the view ever learns
    pub fn take(&mut self) -> Option<Secret>; // moves the buffer out; None when empty
    pub fn clear(&mut self);
}
```

Route: key events, `PasswordEntry`, `take()`, `Secret`, command channel (moves the heap pointer, not the bytes), coordinator, `wire::write_secret` as a `[u16 len][bytes]` frame on the worker's stdin pipe (not JSON, so serde never makes an unzeroized copy), helper reads into `Zeroizing<Vec<u8>>`, `strdup` into the PAM response which libpam scrubs when it frees it (assumption to verify against `_pam_drop_reply`). The coordinator drops the `Secret` right after the write. Worker to parent traffic is JSON lines with no secrets. `Command::env_clear()` plus `PATH`, `LANG`, `XDG_RUNTIME_DIR`; stderr to `/dev/null`.

### `pause`, `power_policy`: lookup tables over a small state

```rust
pub struct Attempts { wrong: u32, paused_until: Option<Instant> }
pub enum Gate { Open, Paused { until: Instant } }
pub const fn pause_for(consecutive_wrong: u32) -> Option<Duration>;  // <3 none, 3: 30 s, 4: 60 s, 5: 120 s, 6+: 300 s (proposal)
impl Attempts { pub fn record_wrong(&mut self, now: Instant); pub fn gate(&self, now: Instant) -> Gate; }

pub enum PowerPhase { Active, Settled, DisplayOff }
pub enum FrameBudget { AnimateOnEvents, StaticOnly, Dark }
pub struct PowerRules { pub frames: FrameBudget, pub minute_tick: bool, pub sensor: SensorArming,
                        pub dim: f32, pub next: Option<(Duration, PowerPhase)> }
pub const fn rules(p: PowerPhase) -> PowerRules {            // exhaustive match; the whole policy is this table
    match p {
        Active     => PowerRules { frames: AnimateOnEvents, minute_tick: true,  sensor: Armed,    dim: 1.0,  next: Some((SETTLE_AFTER /*5 s*/, Settled)) },
        Settled    => PowerRules { frames: StaticOnly,      minute_tick: true,  sensor: Armed,    dim: 0.45, next: Some((DISPLAY_OFF_AFTER /*30 s*/, DisplayOff)) },
        DisplayOff => PowerRules { frames: Dark,            minute_tick: false, sensor: Disarmed, dim: 0.0,  next: None },
    }
}
pub struct PowerClock { phase: PowerPhase, since: Instant }
impl PowerClock {
    pub fn touch(&mut self, now: Instant) -> Option<PowerPhase>;     // any key/pointer: back to Active, returns Some if it changed
    pub fn advance(&mut self, now: Instant) -> Option<PowerPhase>;   // due transition per rules(phase).next
    pub fn deadline(&self) -> Option<Instant>;
}
```

`after_change` applies a returned `PowerPhase` with three lines: `race.set_sensor(rules(p).sensor)`, a niri IPC `PowerOffMonitors`/`PowerOnMonitors` (idempotent, allowed while locked), `windows.notify()`. Lock transition and the settle timeline are independent: `touch` also runs when the transition ends, so Settled is measured from the last activity.

### `view`: Dial is a pure function of a small ViewState

```rust
pub struct ViewInputs { dots: u16, checking: bool, fp: FpLight, wrong_since: Option<Instant>,
                        paused_until: Option<Instant>, unlocking: bool }
pub enum FpLight { Off, Waiting, Reading }
pub enum Phase { Resting, FingerprintReading, Typing { dots: u16, checking: bool }, Wrong { since: Instant },
                 PasswordPause { until: Instant }, Unlocking }
pub fn phase(i: &ViewInputs, now: Instant) -> Phase;  // priority: Unlocking > Pause > Wrong(<WRONG_MS) > Typing > Reading > Resting

pub struct ViewState { pub phase: Phase, pub hint: Option<String>, pub role: OutputRole, pub motion: Motion,
                       pub power: PowerPhase, pub clock: ClockFace }
pub struct Dial { pub ring: Ring /*fill 0..1, flash 0..1, tone*/, pub scale: f32, pub pill: Option<Pill /*dots, shake_x, tone*/>,
                  pub hint: String, pub time: String, pub date: String, pub dim: f32 }
pub fn dial(v: &ViewState, now: Instant) -> Dial;          // Dim role yields only a dim clock; Reduced motion zeroes shake and flash, keeps colour
pub fn schedule(v: &ViewState, now: Instant) -> Schedule;  // reuses speech's idiom: Idle | Animate | WakeAt(Instant)
impl Schedule { pub fn earliest(self, other: Self) -> Self }
```

`Phase` is derived each frame from `ViewInputs`, so there is no stored `phase` to drift. The spec's states are all variants, plus `checking` inside `Typing` for the measured 1.5 to 3 s PAM delay (inputs ignored, dots stay).

### `transition`: pure geometry from shell frame to dial

```rust
pub struct Layout { pub opening: Bounds<Pixels>, pub radius: Pixels, pub clock: Bounds<Pixels>, pub clock_px: f32,
                    pub shell_label_alpha: f32, pub dial_alpha: f32, pub wallpaper_alpha: f32, pub surround_alpha: f32 }
pub enum Style { Morph, Crossfade /* reduced motion */ }
pub fn shell_layout(viewport: Size<Pixels>, controls_width: f32) -> Layout;   // geometry::* + CORNER_RADIUS + clock_bounds
pub fn dial_layout(viewport: Size<Pixels>) -> Layout;                         // centred disc, big time, no surround
pub fn interpolate(a: &Layout, b: &Layout, t: f32) -> Layout;                 // per-field lerp, surround fades in the last 40%
pub fn layout(viewport: Size<Pixels>, t: f32, style: Style) -> Layout;        // t from design::MenuReveal::sample
```

Uses only gpui plain-data geometry (`Bounds`, `Pixels`), no `Window` or `Context`. `MenuReveal::set_open(false)` already reverses from the visible shape, so interrupted unlock is existing, tested behaviour.

### Shared shell drawing (item 4): what moves into the lib

- `geometry.rs` gains `CORNER_RADIUS = 28`, `TOP_BAR_CLOCK_PX = 11`, `clock_bounds()` and `opening_bounds(viewport)`, replacing the inline arithmetic at `frame.rs:1665`. It becomes `pub mod geometry`.
- `design.rs` gains `paint_desktop_surround(window, bounds, opening, radius, &ThemeColors)` (the canvas body at `frame.rs:1665-1728`, with `append_rounded_rectangle{,_reverse}`), `shell_clock_element(label, bounds, px, colors)` (the `shell-clock` div at `frame.rs:1733`), and `FONT_FAMILY`/`font_features()` from `power.rs`. `FrameView::render` calls these, so shell and locker draw with one code path: pixel identity by construction, not by copied numbers.
- `ThemeColors` and `Color` move from `config.rs` to lib `colors.rs`; `config.rs` keeps `ConfigStore`.
- `clock.rs` keeps the 1 s `Clock` global (shell only). Its pure `format_local` moves to lib `clock_text.rs` with `ClockFace`, `seconds_until_next_minute`.

### Per-output windows and the single buffer (item 3)

`LockApp` opens one `WindowKind::SessionLock` window per `cx.displays()` entry, each a `LockView { output: DisplayId, model: Entity<LockModel> }`. There is exactly one `LockModel` entity, holding the one `PasswordEntry`, so every screen shows the same dots by construction. `model.active: DisplayId` is set by pointer motion on a window, and also by any key event (keys arrive only on the compositor-focused surface, so the key is the truth). `view_state(output)` sets `role = Active` only for that output. `cx.notify()` runs only when `active` actually changes. Hotplug calls `sync_outputs` on GPUI's display-change hook from issue 01.

### Power rules (item 5), traced

- Zero frames when idle: render calls `request_animation_frame` only when `schedule == Animate`. There is no interval, no `cx.observe_global<Clock>` and no 1 s timer. Every `cx.notify()` comes from an event handler or the one deadline timer.
- One deadline timer (`wake: Option<(Instant, Task)>`, same pattern as `speech_wake` in `frame.rs`). Its value is `earliest(power.deadline, wrong end, pause end, next_minute if rules.minute_tick)`.
- Animation is bounded by event timestamps: transition reveal, wrong shake (about 500 ms from `wrong_since`), fingerprint ring fill (about 400 ms from the Hint or match). After that `schedule` returns `WakeAt` or `Idle`.
- Display-off at 30 s after Settled (`DISPLAY_OFF_AFTER`). `DisplayOff` has `Dark` frames, `minute_tick: false`, `sensor: Disarmed`. Wake runs `touch`, re-arms the sensor and turns monitors on. The waking keypress still reaches `on_key`, which depends on niri forwarding it (verify in issue 06).

### Crash and exit (item 6)

`lifecycle::ExitReason { Unlocked, Refused, ConnectionLost, SessionEndedByCompositor }`; `exit_code`: 0 for the first, second and fourth, 75 for `ConnectionLost`. `SessionLockEvent::Finished` before `Locked` means another locker is alive, so exit 0 and do not restart. A lost connection calls `std::process::exit(75)` explicitly (GPUI spun at 99% CPU in the spike), so `Restart=on-failure` brings up a replacement while the compositor keeps the session locked. `PDEATHSIG` plus `KillMode=control-group` guarantee workers die with the locker. `lifecycle::claim_marker` writes `$XDG_RUNTIME_DIR/dojo/locked-at` (atomic rename) on the first `Locked` and deletes it before `unlock_session()`. A restarted locker that finds it gets `Origin::Takeover { since }`: it starts at `dial_layout` with no morph and keeps the lock-age chip honest. The password buffer is deliberately lost on restart.

### Testing seams (item 7)

- Pure modules take `now: Instant` and run with no GPUI app: `pause_for`, `rules`, `PowerClock`, `phase`, `dial`, `schedule`, `interpolate` (`t=0` equals `shell_layout`, `t=1` equals `dial_layout`, monotone, `layout(t)` reverses), `PasswordEntry` (capacity never reallocates, popped bytes zeroed), `reaper::descendants` on fake `ProcTable`s.
- `UnlockRace` integration test `tests/unlock_race.rs` (`required-features = ["fake-pam-stack"]`, helper from `env!("CARGO_BIN_EXE_dojo-lock-pam")`). It builds a tempdir confdir with `pam_start_confdir` stacks and ports the spike's scenarios A to H (`run_scenarios.py`): `pam_echo` for hints, `pam_exec expose_authtok` against a fake constant for the password, `pam_exec sleep` for a slow match (this also covers `setsid` escapes), `pam_deny`, a crash-the-worker stack. Added assertions: every worker and descendant is gone when `Unlocked` arrives; no secret bytes in `/proc/<helper>/{cmdline,environ}` while the worker waits on the prompt; `format!("{:?}", cmd)` stays redacted; the feature flag is absent from release builds, so a production helper cannot be pointed at a fake stack.
- `LockView` smoke test under `gpui::TestAppContext` (dev-dependency already present). Nested-Niri run: `tools/agent-input/run-nested.sh --lock` dispatches to a new `run-lock.py` (lock reaches `locked` on two outputs, kill-and-takeover, agent keys rejected, `ChangeVt` escapes) plus a pixel diff of the first lock frame against a shell screenshot.

## Module map (item 8)

Lib (`src/lock/`, all `pub mod lock;` in `src/lib.rs`; none import `Window` or `Context`):

| File | Owns |
|---|---|
| `lock/mod.rs` | module list, `Method` |
| `lock/secret.rs` | `Secret`, `PasswordEntry` |
| `lock/unlock/mod.rs` | `UnlockRace`, `UnlockEvent`, `RaceConfig`, `SensorArming`, `run_helper` |
| `lock/unlock/{wire,reaper,worker,pam}.rs` | private: framing, process-tree kill, child and helper body, libpam dlopen |
| `lock/pause.rs` | `Attempts`, `Gate`, `pause_for` |
| `lock/power_policy.rs` | `PowerPhase`, `rules`, `PowerClock` |
| `lock/view.rs` | `ViewInputs`, `Phase`, `ViewState`, `Dial`, `dial`, `schedule` |
| `lock/transition.rs` | `Layout`, `Style`, `layout` |
| `lock/theme.rs` | `LockTheme::load` (reads `dojo.json` via `ThemeColors`, `error` from `palette.json`, optional `lock-wallpaper.png`; defaults on failure) |
| `lock/lifecycle.rs` | `ExitReason`, `Origin`, marker, exit codes |
| `colors.rs`, `clock_text.rs` | moved `ThemeColors`/`Color`; pure clock formatting |

Bin (`src/bin/`, structured like `dojo-agent` with `#[path = "dojo-lock/..."]`):

| File | Owns |
|---|---|
| `dojo-lock.rs` | `main`, wiring order from Usage |
| `dojo-lock/app.rs` | `LockApp`, `LockModel` (state owners plus the handlers above), window set, deadline timer |
| `dojo-lock/view.rs` | `LockView: Render`, key and pointer listeners, paints `Dial` and `Layout` |
| `dojo-lock/paint.rs` | tick ring, pill, chips as canvas code over `Dial` |
| `dojo-lock-pam.rs` | `dojo::lock::unlock::run_helper()` |

Existing files that change: `Cargo.toml` (`zeroize`, feature `fake-pam-stack`, `[[test]]`), `src/lib.rs`, `src/main.rs` (drop `mod geometry;`, add `--lock`), `src/geometry.rs`, `src/design.rs`, `src/frame.rs` (call shared drawing), `src/config.rs`, `src/clock.rs`, `src/power.rs`, `src/control.rs` and `src/system_menu.rs` (Lock trigger, issue 04), `src/theme/export.rs` (`lock-wallpaper.png`), `home-manager.nix` (service, keybind), `system-config/pam.d/dojo-lock-{fingerprint,password}` from the spike's `recommended/`, `scripts/install-dojo-release.sh` (install both new bins), `tools/agent-input/run-nested.sh`, `devenv.nix` if libpam is needed at test time.

## Interface depth and red-flag screen

`UnlockRace` exposes 1 constructor, 2 methods, 1 event enum of 7 variants and a config. Behind that: process spawning, the wire protocol, prompt-versus-submit timing, a fresh PAM handle per attempt, first-success-wins, killing the loser plus `setsid` descendants, sensor release, fingerprint retry policy, crash handling, zeroization on the pipe. The alternative is `Worker::spawn/kill/reply` exposed to the UI, where the caller coordinates five steps per attempt (per minimize-reader-load: broad interface, little hidden). The other modules are shallow by design (a table, a pure function), but each holds a rule that would otherwise be repeated across handlers.

- Shallow module: only `Attempts` and `PowerClock` are small, and each answers a question (`gate`, `deadline`) that otherwise needs two modules' knowledge. Everything the caller needs from `UnlockRace` is one call.
- Information leakage: PAM rc codes, JSON and fds never leave `unlock`; the service-name list exists once (`Method::service`); PAM message text is shown verbatim, never parsed (per boundary-discipline).
- Temporal decomposition: modules are split by rule set (typing, pausing, powering, drawing), not by load/validate/transform. Submodules of `unlock` are protocol, process-tree and PAM-side knowledge.
- Pass-through: `LockApp` handlers are not forwarders; each composes two or three owners with a policy decision. The risk is that `LockModel` grows into the reducer this design avoids (see tradeoffs).
- Split ownership: password buffer in `PasswordEntry` only; worker state in the coordinator only; `Phase` derived (not stored); `active` output in `LockModel` only; sensor arming decided by `rules()` and applied by the race.
- Two ways to do one task: one unlock path (`submit`), one table for power, one painter for the surround.
- Importable internals: `Secret::as_bytes` is `pub(in crate::lock)`; `wire`/`reaper`/`pam` are private modules; the fake-stack field is cfg-gated.
- Hand-synced list: none. `rules()` and `Phase` are exhaustive matches, so a new variant fails the build (per type-system-discipline).

## Synthesis decision

Left to arena.

## Tradeoffs accepted

- We accept a `LockModel` that coordinates six owners in exchange for no global `Msg`/`Effect` reducer. The discipline is that each handler touches at most three owners and calls `after_change` once.
- We accept `ViewInputs` partly mirroring `LockModel` fields in exchange for a derived `Phase` that cannot drift from its inputs.
- We accept a second binary and an install-script change in exchange for light workers and a worker body that cannot initialise the GPU.
- We accept `dlopen` of libpam (about 60 lines of `unsafe`) in exchange for no link-time libpam dependency in the shell and agent bins.
- We accept one blocked coordinator thread per lock in exchange for zero idle wakeups and no async PAM code.
- We accept that the lock-age marker file can be stale after a crash between delete and `unlock_session()`; the cost is one lock with no morph.
- We accept restarting the PAM password worker after every wrong attempt (a few ms) in exchange for a clean handle and faillock preauth state each time.

## Alternatives considered

- One `LockState` plus `Msg` and `Effect` reducer: easy to snapshot-test, but the `Msg` enum becomes the widest interface in the program and pause, power and PAM rules interleave in one match. It loses on depth.
- Worker as a thread or async task calling PAM in-process: PAM blocks and cannot be cancelled, so the loser could not be killed and the sensor not released. ADR 0002 forces processes.
- Worker as `dojo-lock --pam-worker` re-exec: single artifact and no install change, and it is close. It loses because the worker would sit behind the GPUI-initialising `main` and a bin built with the GPU dependency graph.
- Passing the password by env, argv or a temp file: rejected. A pipe write end held only by the coordinator is the only route with no persistent trace.
- Talking to fprintd over D-Bus: rejected by ADR 0002.

## Open questions and risks

- Can a touch be seen before the result? PAM only emits "Place your finger" and a final result, so "ring fills on touch" has no signal. May the locker subscribe read-only to fprintd's `VerifyStatus` D-Bus signal for UI only, never for trust? Until then the ring fills on `Hint` and completes on `Unlocked`.
- Should a keypress during the reverse transition after a successful auth cancel the unlock (issue 07 says reverse cleanly)? That forces a fresh `UnlockRace` and a second authentication. I recommend making unlock irreversible after `Unlocked`.
- Which file is the theme source of truth, `dojo.json` or `palette.json`? The design reads both (colours plus `error`). Is that acceptable or should `dojo-theme` export a `lock.json`?
- First-frame parity covers the surround and clock only. The shell top bar (tray, battery, audio) cannot be reproduced by the locker. Should the shell hand `controls_width` and a snapshot to the locker, or is the top bar allowed to vanish at lock?
- Pause table (30 s at 3 failures, doubling to 300 s) is a proposal. It is shorter than faillock's own 10 minute lock. Should fingerprint stay live during a pause (current default: yes)?
- Is `pam_fprintd` returning `PAM_AUTH_ERR` or `PAM_MAXTRIES` on no-match? Neither was observed. Both map to `FingerprintGaveUp` for now.
- Does libpam scrub `PROMPT_ECHO_OFF` replies on free in the installed 1.7.3? Needs a check before claiming it.
- Display hotplug while locked depends on the issue 01 hook, whose name is not in the spike patch.
- Does niri forward the waking keypress while monitors are off (issue 06)?

## Next implementation step

Build `lock/secret.rs`, `lock/unlock/*` and `dojo-lock-pam` with the ported spike scenarios A to H as `tests/unlock_race.rs` (issue 02), before any GPUI code, since it is the deep module and does not depend on issue 01.
