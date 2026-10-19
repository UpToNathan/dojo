//! The shell's closing: an Overlay-layer window per output that draws the frame
//! closing in with the farewell, then runs the occasion's action.
//!
//! `dojo::session::closing` decides what happens and when; this module owns the
//! windows, the one deadline timer, the lock confirmation and the real actions.
//! Frames are requested only while the picture changes, so a parked closing
//! costs nothing.
//!
//! The login opening reuses the same windows and painter: `dojo::session::opening`
//! decides, and [`begin_login`] and [`start_login`] bracket the shell's start-up.
//!
//! So does the unlock opening. When the lock is confirmed the closing's windows are
//! not dismissed but parked: one per output, on the closed state the locker also
//! draws, with no timers and no frames, hidden by the compositor for as long as
//! the lock lasts. `$XDG_RUNTIME_DIR/dojo/overlay-ready` tells the locker they are
//! there. When the locker's `locked-at` marker goes (or logind's `LockedHint` drops)
//! the same windows open back into the desktop frame over the live windows.

use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use dojo::config::{ConfigStore, ThemeColors};
use dojo::session::action::{Action, ActionRunner, Departure};
use dojo::session::closing::{Closing, Effect, Event, EventKind, Phase};
use dojo::session::host::{self, Account};
use dojo::session::{handoff, login, logind};
use dojo::session::opening::{self, Opening};
use dojo::session::paint::{self, Look};
use dojo::session::settings::SessionSettings;
use dojo::session::transition::{self, SHELL_CONTROLS_WIDTH, Style};
use dojo::session::{Occasion, Reason, history, message};
use dojo::wall_clock;
use gpui::{
    App, AppContext, AsyncApp, FocusHandle, Global, IntoElement, Render, Rgba, Task, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div,
    layer_shell, prelude::*, px,
};

#[cfg_attr(feature = "record-session-actions", allow(dead_code))]
const LOCKER_SERVICE: &str = "dojo-lock.service";

#[derive(Default)]
struct SessionOverlay {
    active: Option<Active>,
}
impl Global for SessionOverlay {}

struct Active {
    closing: Closing,
    /// When the closing began, for the timeline lines in the log.
    begun: Instant,
    shell_clock: String,
    dial_clock: String,
    colors: ThemeColors,
    error: Rgba,
    look: Rc<Look>,
    windows: Vec<WindowHandle<ClosingView>>,
    runner: Box<dyn ActionRunner>,
    /// The one deadline timer. Replacing it cancels the previous one.
    timer: Option<Task<()>>,
    _lock_watch: Vec<Task<()>>,
    _wallpaper: Option<Task<()>>,
    /// The first frame has been reported to the closing.
    presented: bool,
}

/// One line of the closing's timeline, in milliseconds since it began. The
/// journal is how a real handoff is read afterwards.
fn note(begun: Instant, event: &str) {
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    eprintln!(
        "dojo-session: event={event} ms={} unix_ms={unix_ms}",
        begun.elapsed().as_millis()
    );
}

fn trace(begun: Instant, effects: &[Effect]) {
    for effect in effects {
        match effect {
            Effect::StartLocker => note(begun, "locker-start"),
            Effect::Run(action) => note(begun, &format!("run {}", action.key())),
            Effect::DismissOverlay => note(begun, "overlay-dismissed"),
            Effect::Done => note(begun, "done"),
        }
    }
}

/// What the shell has to do after a batch of effects.
#[derive(Debug, Default, PartialEq, Eq)]
struct Outcome {
    dismiss: bool,
    done: bool,
}

/// Runs the actions in `effects` in order. A failed action is logged and never
/// stops the rest; the closing's hard cap removes the cover regardless.
fn perform(effects: &[Effect], runner: &mut dyn ActionRunner) -> Outcome {
    let mut outcome = Outcome::default();
    for effect in effects {
        match effect {
            Effect::StartLocker | Effect::Run(_) => {
                if let Some(action) = effect.action()
                    && let Err(error) = runner.run(action)
                {
                    eprintln!("Dojo could not run {}: {error}", action.key());
                }
            }
            Effect::DismissOverlay => outcome.dismiss = true,
            Effect::Done => outcome.done = true,
        }
    }
    outcome
}

/// The real actions: `systemctl --user start` for the locker, and the system
/// menu's logind and Niri calls for everything else.
#[cfg_attr(feature = "record-session-actions", allow(dead_code))]
struct ShellRunner {
    /// Inside the shell nothing may block the UI thread; a one-shot CLI waits.
    detached: bool,
}

impl ActionRunner for ShellRunner {
    fn run(&mut self, action: Action) -> Result<(), String> {
        if action == Action::StartLocker {
            return start_locker(self.detached);
        }
        if self.detached {
            std::thread::spawn(move || {
                if let Err(error) = crate::system_menu::perform(action) {
                    eprintln!("Dojo could not {}: {error:#}", action.key());
                }
            });
            return Ok(());
        }
        crate::system_menu::perform(action).map_err(|error| format!("{error:#}"))
    }
}

#[cfg_attr(feature = "record-session-actions", allow(dead_code))]
fn start_locker(detached: bool) -> Result<(), String> {
    let mut child = Command::new("systemctl")
        .args(["--user", "start", LOCKER_SERVICE])
        .spawn()
        .map_err(|error| format!("starting {LOCKER_SERVICE}: {error}"))?;
    if detached {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }
    let status = child
        .wait()
        .map_err(|error| format!("starting {LOCKER_SERVICE}: {error}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("systemctl --user start {LOCKER_SERVICE} exited with {status}"))
}

/// Release builds always act. A build with `record-session-actions` never does:
/// it appends each action's name to `DOJO_SESSION_ACTION_LOG` instead.
#[cfg(not(feature = "record-session-actions"))]
fn runner(detached: bool) -> Box<dyn ActionRunner> {
    Box::new(ShellRunner { detached })
}

#[cfg(feature = "record-session-actions")]
fn runner(_detached: bool) -> Box<dyn ActionRunner> {
    use dojo::session::action::RecordingRunner;
    Box::new(match std::env::var_os("DOJO_SESSION_ACTION_LOG") {
        Some(path) => RecordingRunner::to_file(path),
        None => RecordingRunner::new(),
    })
}

/// `dojo --leave` with no shell running: the action alone, with no animation.
pub fn leave_without_shell(departure: Departure) -> anyhow::Result<()> {
    run_directly(departure, runner(false).as_mut())
}

fn run_directly(departure: Departure, runner: &mut dyn ActionRunner) -> anyhow::Result<()> {
    runner.run(departure.action()).map_err(anyhow::Error::msg)
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn session_settings() -> SessionSettings {
    ConfigStore::user_default()
        .and_then(|store| store.session_settings())
        .unwrap_or_else(|error| {
            eprintln!("Dojo is using default session settings: {error:#}");
            SessionSettings::default()
        })
}

/// The line for this occasion, never the line shown last time.
fn pick_line(occasion: Occasion, unix: i64, fresh_boot: bool) -> String {
    let reason = (occasion == Occasion::Lock).then_some(Reason::Manual);
    let account = Account::current();
    let previous =
        history::default_directory().and_then(|directory| history::load(&directory, occasion));
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    let context = message::Context {
        hour: host::local_hour(unix),
        fresh_boot,
        reason,
        previous,
        seed: (unix as u64) << 32 | u64::from(nanos),
        ..message::Context::default()
    };
    let name = message::display_name(account.gecos.as_deref(), &account.login);
    message::select(occasion, &context, &name)
}

fn store_line(occasion: Occasion, line: String) {
    std::thread::spawn(move || {
        if let Some(directory) = history::default_directory()
            && let Err(error) = history::store(&directory, occasion, &line)
        {
            eprintln!("Dojo could not store the {} line: {error}", occasion.key());
        }
    });
}

/// What asks for a closing: the shell's own departure, which performs its action,
/// or a farewell for an occasion that something else carries out.
#[derive(Clone, Copy)]
enum Request {
    Leave(Departure),
    Farewell(Occasion),
}

impl Request {
    fn occasion(self) -> Occasion {
        match self {
            Request::Leave(departure) => departure.occasion(),
            Request::Farewell(occasion) => occasion,
        }
    }

    fn begin(
        self,
        settings: &SessionSettings,
        line: Option<String>,
        now: Instant,
    ) -> (Closing, Vec<Effect>) {
        match self {
            Request::Leave(departure) => Closing::depart(departure, settings, line, now),
            Request::Farewell(occasion) => Closing::farewell_only(occasion, settings, line, now),
        }
    }
}

/// Tells the control socket whether a closing is still playing, so a waiting
/// `dojo --farewell` returns as soon as the picture is done.
fn publish(cx: &App) {
    let playing = cx
        .try_global::<SessionOverlay>()
        .and_then(|overlay| overlay.active.as_ref())
        .is_some_and(|active| !active.closing.timeline_over());
    dojo::control::CLOSING_PLAYING.store(playing, std::sync::atomic::Ordering::Release);
}

/// Begins the closing for `departure` and performs its action when it ends. A
/// second request while one runs is ignored, except that a lock asked for during
/// a suspend or hibernate closing starts the locker.
/// When no window can be opened the action runs at once, as it did before the closing existed.
pub fn leave(departure: Departure, cx: &mut App) -> anyhow::Result<()> {
    let result = play(Request::Leave(departure), cx);
    publish(cx);
    result
}

/// Plays the closing and farewell for `occasion` and performs nothing: logind, or
/// whatever else started the occasion, does that. Idempotent: while any closing is
/// on screen, including the one the shell began for the same occasion, it does nothing.
pub fn farewell(occasion: Occasion, cx: &mut App) -> anyhow::Result<()> {
    anyhow::ensure!(
        Departure::parse_farewell(occasion.key()) == Some(occasion),
        "{} has no farewell to play",
        occasion.key()
    );
    let result = play(Request::Farewell(occasion), cx);
    publish(cx);
    result
}

/// A lock was asked for while a closing is on screen. Only a sleeping closing acts on it.
fn ask_for_lock(cx: &mut App) {
    cx.update_global::<SessionOverlay, _>(|overlay, cx| {
        let Some(active) = overlay.active.as_mut() else {
            return;
        };
        let effects = active.closing.lock_requested();
        if effects.is_empty() {
            eprintln!("Dojo ignored lock: a transition is already running");
            return;
        }
        active._lock_watch = watch_lock(active.begun, SystemTime::now(), cx);
        trace(active.begun, &effects);
        perform(&effects, active.runner.as_mut());
    });
}

fn play(request: Request, cx: &mut App) -> anyhow::Result<()> {
    let occasion = request.occasion();
    let login_running = cx
        .try_global::<OpeningOverlay>()
        .is_some_and(|overlay| overlay.active.is_some());
    let closing_running =
        cx.has_global::<SessionOverlay>() && cx.global::<SessionOverlay>().active.is_some();
    if login_running {
        eprintln!(
            "Dojo ignored {}: a transition is already running",
            occasion.key()
        );
        return Ok(());
    }
    if closing_running {
        if occasion == Occasion::Lock && matches!(request, Request::Leave(_)) {
            ask_for_lock(cx);
        } else {
            eprintln!(
                "Dojo ignored {}: a transition is already running",
                occasion.key()
            );
        }
        return Ok(());
    }
    let settings = session_settings();
    let (now, unix) = (Instant::now(), unix_now());
    // A lock's farewell is the locker's: it travels in the handoff file, not the closing.
    let line = (settings.transition && settings.farewell && occasion != Occasion::Lock)
        .then(|| pick_line(occasion, unix, false));
    let (mut closing, mut effects) = request.begin(&settings, line, now);
    if occasion == Occasion::Lock && settings.farewell {
        hand_off_farewell(unix);
    }
    note(now, &format!("closing-begin occasion={}", occasion.key()));
    // Decoding starts before anything else is slow, so the wallpaper is there when the closing needs it.
    let wallpaper = cx.spawn(async move |cx| {
        let image = cx
            .background_executor()
            .spawn(async { host::decode_wallpaper() })
            .await;
        cx.update(|cx| set_wallpaper(image, cx));
    });
    let mut windows = Vec::new();
    if closing.phase() != Phase::Done {
        match open_windows(cx) {
            Ok(opened) => windows = opened,
            Err(error) => {
                eprintln!("Dojo could not draw the closing: {error:#}");
                let plain = SessionSettings {
                    transition: false,
                    ..settings
                };
                (closing, effects) = request.begin(&plain, None, now);
            }
        }
    }
    if let Some(line) = closing.farewell() {
        store_line(occasion, line.to_owned());
    }
    let lock_watch = if occasion == Occasion::Lock {
        watch_lock(now, SystemTime::now(), cx)
    } else {
        Vec::new()
    };
    let mut runner = runner(true);
    trace(now, &effects);
    let outcome = perform(&effects, runner.as_mut());
    if outcome.done {
        return Ok(());
    }
    let colors = *cx.global::<ThemeColors>();
    let error = gpui::rgb(host::error_color());
    let mut active = Active {
        closing,
        begun: now,
        shell_clock: wall_clock::top_bar_label(unix),
        dial_clock: wall_clock::dial_time(unix),
        colors,
        error,
        look: Rc::new(Look::new(colors, error, None)),
        windows,
        runner,
        timer: None,
        _lock_watch: lock_watch,
        _wallpaper: Some(wallpaper),
        presented: false,
    };
    arm(&mut active, cx);
    cx.set_global(SessionOverlay {
        active: Some(active),
    });
    Ok(())
}

fn open_windows(cx: &mut App) -> anyhow::Result<Vec<WindowHandle<ClosingView>>> {
    let mut windows = Vec::new();
    for display in cx.displays() {
        let opened = cx.open_window(
            WindowOptions {
                display_id: Some(display.id()),
                window_bounds: Some(WindowBounds::Windowed(display.bounds())),
                titlebar: None,
                app_id: Some("dojo".into()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                    namespace: format!("dojo-session-{}", u64::from(display.id())),
                    layer: layer_shell::Layer::Overlay,
                    anchor: layer_shell::Anchor::TOP
                        | layer_shell::Anchor::BOTTOM
                        | layer_shell::Anchor::LEFT
                        | layer_shell::Anchor::RIGHT,
                    exclusive_zone: Some(px(-1.)),
                    keyboard_interactivity: layer_shell::KeyboardInteractivity::Exclusive,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    ClosingView {
                        focus,
                        asked: false,
                        asked_open: false,
                    }
                })
            },
        );
        match opened {
            Ok(window) => windows.push(window),
            Err(error) => {
                remove(windows, cx);
                return Err(error).context("opening a closing window");
            }
        }
    }
    anyhow::ensure!(!windows.is_empty(), "no output to draw the closing on");
    Ok(windows)
}

fn remove(windows: Vec<WindowHandle<ClosingView>>, cx: &mut App) {
    for window in windows {
        let _ = window.update(cx, |_, window, _| window.remove_window());
    }
}

/// One line per frame for the nested capture runs (`tools/agent-input/morph-frames.py`), which
/// date the picture by it. A build without `record-session-actions` logs nothing.
#[cfg(feature = "record-session-actions")]
fn trace_frame(tag: &str, begun: Instant, progress: f32, pose: &transition::Pose, look: &Look) {
    let transition::Pose::Moving(layout) = pose else {
        return;
    };
    let over = paint::Over::at(progress, pose);
    eprintln!(
        "dojo-trace: {tag} since_begin_ms={} progress={progress:.4} cover={:.3} frame={:.3} open_w={:.1} open_h={:.1} radius={:.1} wallpaper={:.3} surround={:.2} clock_px={:.2} has_wallpaper={} unix_ms={}",
        begun.elapsed().as_millis(),
        over.workspace,
        over.frame,
        f32::from(layout.opening.size.width),
        f32::from(layout.opening.size.height),
        f32::from(layout.radius),
        layout.wallpaper,
        layout.surround,
        layout.clock_px,
        look.wallpaper.is_some(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis()),
    );
}

/// A parked overlay should never draw after the park itself; the capture runs count these.
#[cfg(feature = "record-session-actions")]
fn trace_parked() {
    eprintln!(
        "dojo-trace: parked-frame unix_ms={}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis())
    );
}

#[cfg(not(feature = "record-session-actions"))]
fn trace_parked() {}

#[cfg(not(feature = "record-session-actions"))]
fn trace_frame(_: &str, _: Instant, _: f32, _: &transition::Pose, _: &Look) {}

/// Feeds an event to the running closing and carries out what comes back.
fn dispatch(kind: EventKind, cx: &mut App) {
    if !cx.has_global::<SessionOverlay>() {
        return;
    }
    cx.update_global::<SessionOverlay, _>(|overlay, cx| {
        let Some(active) = overlay.active.as_mut() else {
            return;
        };
        let effects = active.closing.update(Event {
            now: Instant::now(),
            kind,
        });
        if kind == EventKind::LockConfirmed {
            note(active.begun, "lock-confirmed");
        }
        trace(active.begun, &effects);
        let outcome = perform(&effects, active.runner.as_mut());
        if outcome.dismiss && kind == EventKind::LockConfirmed && !active.windows.is_empty() {
            park(active, cx);
            overlay.active = None;
            return;
        }
        if outcome.dismiss {
            remove(std::mem::take(&mut active.windows), cx);
        }
        if outcome.done {
            overlay.active = None;
            return;
        }
        arm(active, cx);
        for window in &active.windows {
            let _ = window.update(cx, |_, window, _| window.refresh());
        }
    });
    publish(cx);
}

/// The lock is confirmed: instead of coming down, the closing's windows stay on the
/// closed state, one per output, and wait for the unlock. Nothing is scheduled and no
/// frame is requested from here until then.
fn park(active: &mut Active, cx: &mut App) {
    note(active.begun, "overlay-parked");
    let windows = std::mem::take(&mut active.windows);
    // The last closing frame may have been a few percent short of closed; draw the parked state once.
    refresh(&windows, cx);
    cx.set_global(OpeningOverlay {
        active: Some(OpeningActive {
            settings: SessionSettings::default(),
            line: None,
            opening: None,
            style: active.closing.style(),
            shell_clock: active.shell_clock.clone(),
            dial_clock: active.dial_clock.clone(),
            colors: active.colors,
            error: active.error,
            look: active.look.clone(),
            windows,
            timer: None,
            _wallpaper: active._wallpaper.take(),
            parked: true,
            _watch: std::mem::take(&mut active._lock_watch),
            _ready: ReadyFile::write(),
            begun: active.begun,
        }),
    });
}

/// The locker has let go of the session (its marker is gone, or logind's hint dropped).
/// A parked overlay opens into the desktop; anything else has nothing to do.
fn unlock_seen(source: &str, cx: &mut App) {
    if !cx.has_global::<OpeningOverlay>() {
        return;
    }
    cx.update_global::<OpeningOverlay, _>(|overlay, cx| {
        let Some(active) = overlay.active.as_mut() else {
            return;
        };
        if !active.parked || active.opening.is_some() {
            return;
        }
        let settings = session_settings();
        let now = Instant::now();
        note(active.begun, &format!("unlock-seen source={source}"));
        // An output that went away while locked took its window with it.
        active
            .windows
            .retain(|window| window.update(cx, |_, _, _| ()).is_ok());
        let (opening, effects) = Opening::begin(Occasion::Unlock, &settings, None, now);
        active.style = opening.style();
        active.settings = settings;
        active.opening = Some(opening);
        let unix = unix_now();
        active.shell_clock = wall_clock::top_bar_label(unix);
        active.dial_clock = wall_clock::dial_time(unix);
        if active.windows.is_empty() {
            overlay.active = None;
            return;
        }
        if finish_opening(overlay, &effects, cx) {
            return;
        }
        if let Some(active) = overlay.active.as_mut() {
            arm_opening(active, cx);
            refresh(&active.windows, cx);
        }
    });
}

/// The unlock opening's first frame is about to be drawn: its clock starts now.
fn opening_presented(cx: &mut App) {
    if !cx.has_global::<OpeningOverlay>() {
        return;
    }
    cx.update_global::<OpeningOverlay, _>(|overlay, cx| {
        let Some(opening) = overlay
            .active
            .as_mut()
            .and_then(|active| active.opening.as_mut())
        else {
            return;
        };
        let first = !opening.presented();
        let effects = opening.update(opening::Event {
            now: Instant::now(),
            kind: opening::EventKind::Presented,
        });
        if first && let Some(active) = overlay.active.as_ref() {
            note(active.begun, "opening-first-frame");
        }
        if finish_opening(overlay, &effects, cx) {
            return;
        }
        if let Some(active) = overlay.active.as_mut() {
            arm_opening(active, cx);
        }
    });
}

/// Sleeps until the closing next needs the shell: a deadline, or the end of the
/// farewell hold when the picture starts to fade. Nothing else wakes.
fn arm(active: &mut Active, cx: &mut App) {
    let now = Instant::now();
    let at = [
        active.closing.next_deadline(),
        active.closing.next_motion(now),
    ]
    .into_iter()
    .flatten()
    .min();
    active.timer = at.map(|at| {
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(at.saturating_duration_since(Instant::now()))
                .await;
            cx.update(|cx| dispatch(EventKind::Tick, cx));
        })
    });
}

fn set_wallpaper(image: Option<std::sync::Arc<gpui::RenderImage>>, cx: &mut App) {
    if image.is_none() {
        return;
    }
    // Decoding can outlast the closing: a parked overlay takes the picture then.
    if !cx.has_global::<SessionOverlay>() || cx.global::<SessionOverlay>().active.is_none() {
        return set_opening_wallpaper(image, cx);
    }
    cx.update_global::<SessionOverlay, _>(|overlay, cx| {
        let Some(active) = overlay.active.as_mut() else {
            return;
        };
        #[cfg(feature = "record-session-actions")]
        note(active.begun, "wallpaper-arrived");
        active.look = Rc::new(Look::new(active.colors, active.error, image));
        for window in &active.windows {
            let _ = window.update(cx, |_, window, _| window.refresh());
        }
    });
}

/// Leaves the lock's farewell for the locker, which plays it after taking the
/// lock. The line is chosen with the same history as every farewell, and the
/// file is in place before the locker starts. Best effort: no file, no farewell.
fn hand_off_farewell(unix: i64) {
    let line = pick_line(Occasion::Lock, unix, false);
    let written = handoff::directory()
        .context("XDG_RUNTIME_DIR is not an absolute path")
        .and_then(|directory| Ok(handoff::write_farewell(&directory, &line)?));
    match written {
        Ok(()) => store_line(Occasion::Lock, line),
        Err(error) => eprintln!("Dojo could not leave a farewell for the locker: {error:#}"),
    }
}

/// Reports the lock to the closing from two independent sources, whichever
/// speaks first: the locker's `locked-at` marker, and logind's `LockedHint`,
/// which the locker sets on the display session. Neither polls.
fn watch_lock(begun: Instant, since: SystemTime, cx: &mut App) -> Vec<Task<()>> {
    vec![
        cx.spawn(async move |cx| {
            if let Err(error) = follow_marker(begun, since, cx).await {
                eprintln!("Dojo cannot watch the lock marker; the hint or the cap will end the closing: {error:#}");
            }
        }),
        cx.spawn(async move |cx| {
            if let Err(error) = follow_locked_hint(begun, cx).await {
                eprintln!("Dojo cannot watch logind's LockedHint; the marker or the cap will end the closing: {error:#}");
            }
        }),
    ]
}

/// Waits on inotify for a `locked-at` written since `since`. A marker left by
/// an earlier lock, or by a compositor that has since gone, is older and ignored.
async fn follow_marker(begun: Instant, since: SystemTime, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let directory = handoff::directory().context("XDG_RUNTIME_DIR is not an absolute path")?;
    std::fs::create_dir_all(&directory)?;
    let watcher = crate::file_events::Watcher::start(vec![directory.join(handoff::LOCKED_AT)])?;
    let (_sender, changes) = watcher.subscribe();
    let mut confirmed = false;
    while changes.recv().await.is_ok() {
        if !confirmed && handoff::lock_taken_since(&directory, since) {
            note(begun, "marker-seen");
            cx.update(|cx| dispatch(EventKind::LockConfirmed, cx));
            confirmed = true;
        }
        // The marker going is the unlock: the locker removes it just before it releases the session.
        if confirmed && !handoff::lock_marker_present(&directory) {
            cx.update(|cx| unlock_seen("marker-gone", cx));
            return Ok(());
        }
    }
    anyhow::bail!("the file watcher stopped")
}

async fn follow_locked_hint(begun: Instant, cx: &mut AsyncApp) -> anyhow::Result<()> {
    use futures_lite::StreamExt as _;
    let connection = logind::connect().await?;
    let session = logind::display_session(&connection).await?;
    let mut changes = std::pin::pin!(session.receive_property_changed::<bool>("LockedHint").await);
    let mut locked = session.get_property::<bool>("LockedHint").await?;
    while let Some(change) = changes.next().await {
        let now = change.get().await?;
        if now && !locked {
            note(begun, "hint-seen");
            cx.update(|cx| dispatch(EventKind::LockConfirmed, cx));
        } else if !now && locked {
            // The marker is the first signal; this is the one that survives a missed file event.
            cx.update(|cx| unlock_seen("hint-cleared", cx));
            return Ok(());
        }
        locked = now;
    }
    anyhow::bail!("the logind session signal stream ended")
}

#[derive(Default)]
struct OpeningOverlay {
    active: Option<OpeningActive>,
}
impl Global for OpeningOverlay {}

struct OpeningActive {
    settings: SessionSettings,
    line: Option<String>,
    /// `None` while the shell is still starting: the closed state, with no greeting.
    opening: Option<Opening>,
    style: Style,
    shell_clock: String,
    dial_clock: String,
    colors: ThemeColors,
    error: Rgba,
    look: Rc<Look>,
    windows: Vec<WindowHandle<ClosingView>>,
    /// The one deadline timer. Replacing it cancels the previous one.
    timer: Option<Task<()>>,
    _wallpaper: Option<Task<()>>,
    /// Held over a confirmed lock, closed and idle, until the unlock is seen.
    parked: bool,
    /// The lock watchers that report the unlock, moved here from the closing.
    _watch: Vec<Task<()>>,
    /// Present while parked and until the overlay is gone: tells the locker the shell opens.
    _ready: Option<ReadyFile>,
    /// For the timeline lines in the log.
    begun: Instant,
}

/// `overlay-ready`, removed when dropped: the overlay is dismissed or the shell exits.
struct ReadyFile(PathBuf);

impl ReadyFile {
    fn write() -> Option<ReadyFile> {
        ReadyFile::write_in(handoff::directory()?)
    }

    fn write_in(directory: PathBuf) -> Option<ReadyFile> {
        match handoff::write_overlay_ready(&directory, std::process::id()) {
            Ok(()) => Some(ReadyFile(directory)),
            Err(error) => {
                eprintln!("Dojo could not announce its parked overlay to the locker: {error}");
                None
            }
        }
    }
}

impl Drop for ReadyFile {
    fn drop(&mut self) {
        handoff::remove_overlay_ready(&self.0);
    }
}

/// A shell that starts holds no overlay, so any `overlay-ready` is a crashed shell's.
/// A shell that ends takes its own with it.
pub fn clear_overlay_ready() {
    if let Some(directory) = handoff::directory() {
        handoff::remove_overlay_ready(&directory);
    }
}

/// First thing in the shell's start-up: when this start is a login, covers every
/// output with the closed state so the compositor's frame colour carries straight
/// into the shell. Any failure means a normal start with no animation.
pub fn begin_login(cx: &mut App) {
    clear_overlay_ready();
    let start = login::detect(std::env::var_os("XDG_RUNTIME_DIR"));
    if !start.is_login() {
        return;
    }
    let settings = session_settings();
    if !settings.transition {
        return;
    }
    let windows = match open_windows(cx) {
        Ok(windows) => windows,
        Err(error) => {
            eprintln!("Dojo could not draw the login opening: {error:#}");
            return;
        }
    };
    let unix = unix_now();
    let line = settings
        .greeting
        .then(|| pick_line(Occasion::Login, unix, login::fresh_boot()));
    let colors = *cx.global::<ThemeColors>();
    let error = gpui::rgb(host::error_color());
    let wallpaper = cx.spawn(async move |cx| {
        let image = cx
            .background_executor()
            .spawn(async { host::decode_wallpaper() })
            .await;
        cx.update(|cx| set_opening_wallpaper(image, cx));
    });
    cx.set_global(OpeningOverlay {
        active: Some(OpeningActive {
            settings,
            line,
            opening: None,
            style: if settings.reduced_motion {
                Style::Crossfade
            } else {
                Style::Morph
            },
            shell_clock: wall_clock::top_bar_label(unix),
            dial_clock: wall_clock::dial_time(unix),
            colors,
            error,
            look: Rc::new(Look::new(colors, error, None)),
            windows,
            timer: None,
            _wallpaper: Some(wallpaper),
            parked: false,
            _watch: Vec::new(),
            _ready: None,
            begun: Instant::now(),
        }),
    });
}

/// Last thing in the shell's start-up: the greeting starts when everything else
/// is up, so start-up work cannot eat it. Does nothing when no login is covered.
pub fn start_login(cx: &mut App) {
    if !cx.has_global::<OpeningOverlay>() {
        return;
    }
    cx.update_global::<OpeningOverlay, _>(|overlay, cx| {
        let Some(active) = overlay.active.as_mut() else {
            return;
        };
        let (opening, effects) = Opening::begin(
            Occasion::Login,
            &active.settings,
            active.line.clone(),
            Instant::now(),
        );
        if let Some(line) = opening.greeting() {
            store_line(Occasion::Login, line.to_owned());
        }
        active.opening = Some(opening);
        if finish_opening(overlay, &effects, cx) {
            return;
        }
        if let Some(active) = overlay.active.as_mut() {
            arm_opening(active, cx);
            refresh(&active.windows, cx);
        }
    });
}

fn refresh(windows: &[WindowHandle<ClosingView>], cx: &mut App) {
    for window in windows {
        let _ = window.update(cx, |_, window, _| window.refresh());
    }
}

/// Applies the machine's effects. True when the opening is over.
fn finish_opening(overlay: &mut OpeningOverlay, effects: &[opening::Effect], cx: &mut App) -> bool {
    let Some(active) = overlay.active.as_mut() else {
        return true;
    };
    #[cfg(feature = "record-session-actions")]
    if effects.contains(&opening::Effect::Done) && hold_unlock_end(active, cx) {
        return true;
    }
    if effects.contains(&opening::Effect::DismissOverlay) {
        remove(std::mem::take(&mut active.windows), cx);
    }
    if effects.contains(&opening::Effect::Done) {
        if active.parked {
            note(active.begun, "unlock-done");
        }
        remove(std::mem::take(&mut active.windows), cx);
        overlay.active = None;
        return true;
    }
    false
}

/// A capture run can ask the finished unlock to stay on screen at its last frame,
/// so that frame can be compared with the resting desktop. Release builds have no such switch.
#[cfg(feature = "record-session-actions")]
fn hold_unlock_end(active: &mut OpeningActive, cx: &mut App) -> bool {
    let Some(hold) = std::env::var("DOJO_SESSION_HOLD_UNLOCK_END_MS")
        .ok()
        .and_then(|text| text.parse::<u64>().ok())
        .filter(|_| active.parked)
    else {
        return false;
    };
    note(active.begun, "unlock-end-held");
    active.timer = Some(cx.spawn(async move |cx| {
        cx.background_executor()
            .timer(std::time::Duration::from_millis(hold))
            .await;
        cx.update(|cx| {
            cx.update_global::<OpeningOverlay, _>(|overlay, cx| {
                if let Some(mut active) = overlay.active.take() {
                    note(active.begun, "unlock-dismissed");
                    remove(std::mem::take(&mut active.windows), cx);
                }
            })
        });
    }));
    refresh(&active.windows, cx);
    true
}

fn dispatch_opening(kind: opening::EventKind, cx: &mut App) {
    if !cx.has_global::<OpeningOverlay>() {
        return;
    }
    cx.update_global::<OpeningOverlay, _>(|overlay, cx| {
        let Some(active) = overlay.active.as_mut() else {
            return;
        };
        let Some(opening) = active.opening.as_mut() else {
            return;
        };
        let effects = opening.update(opening::Event {
            now: Instant::now(),
            kind,
        });
        if finish_opening(overlay, &effects, cx) {
            return;
        }
        if let Some(active) = overlay.active.as_mut() {
            arm_opening(active, cx);
            refresh(&active.windows, cx);
        }
    });
}

/// Sleeps until the opening next needs the shell: a deadline, or the end of the
/// greeting hold when the picture starts to fade. Nothing else wakes.
fn arm_opening(active: &mut OpeningActive, cx: &mut App) {
    let now = Instant::now();
    let at = active.opening.as_ref().and_then(|opening| {
        [opening.next_deadline(), opening.next_motion(now)]
            .into_iter()
            .flatten()
            .min()
    });
    active.timer = at.map(|at| {
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(at.saturating_duration_since(Instant::now()))
                .await;
            cx.update(|cx| dispatch_opening(opening::EventKind::Tick, cx));
        })
    });
}

fn set_opening_wallpaper(image: Option<std::sync::Arc<gpui::RenderImage>>, cx: &mut App) {
    if image.is_none() || !cx.has_global::<OpeningOverlay>() {
        return;
    }
    cx.update_global::<OpeningOverlay, _>(|overlay, cx| {
        let Some(active) = overlay.active.as_mut() else {
            return;
        };
        active.look = Rc::new(Look::new(active.colors, active.error, image));
        refresh(&active.windows, cx);
    });
}

/// The login opening at one instant, drawn by the same painter as a closing run backwards.
fn opening_frame(active: &OpeningActive, window: &mut Window) -> Vec<gpui::AnyElement> {
    let now = Instant::now();
    let (progress, greeting) = match &active.opening {
        Some(opening) => {
            if opening.animating(now) {
                window.request_animation_frame();
            }
            (
                opening.progress(now),
                opening
                    .greeting()
                    .map(|text| (text, opening.greeting_alpha(now))),
            )
        }
        None => (1., None),
    };
    let viewport = window.viewport_size();
    let pose = transition::layout(viewport, SHELL_CONTROLS_WIDTH, progress, active.style);
    if active.parked && active.opening.is_some() {
        trace_frame("unlock-frame", active.begun, progress, &pose, &active.look);
    } else if active.parked {
        trace_parked();
    }
    paint::closing(
        &pose,
        progress,
        &active.shell_clock,
        &active.dial_clock,
        greeting,
        &active.look,
        viewport,
    )
}

struct ClosingView {
    focus: FocusHandle,
    /// Whether this window has asked to be told of its first presented frame.
    asked: bool,
    /// The same for the unlock opening, which waits for its first frame.
    asked_open: bool,
}

/// The cover's first frame is about to be drawn: start the closing's clock now,
/// not when the window was requested. Runs inside the frame, before it is drawn.
fn presented(cx: &mut App) {
    if !cx.has_global::<SessionOverlay>() {
        return;
    }
    cx.update_global::<SessionOverlay, _>(|overlay, cx| {
        let Some(active) = overlay.active.as_mut() else {
            return;
        };
        let first = !active.presented;
        active.presented = true;
        let effects = active.closing.update(Event {
            now: Instant::now(),
            kind: EventKind::Presented,
        });
        if first {
            note(active.begun, "first-frame");
        }
        trace(active.begun, &effects);
        arm(active, cx);
    });
}

/// A key or pointer press skips ahead and never cancels. It runs after the
/// current window update, because the closing may remove this very window.
fn press(cx: &mut gpui::Context<ClosingView>) {
    cx.spawn(async move |_, cx| {
        cx.update(|cx| {
            dispatch(EventKind::Input, cx);
            dispatch_opening(opening::EventKind::Input, cx);
        })
    })
    .detach();
}

impl Render for ClosingView {
    fn render(&mut self, window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let root = div()
            .size_full()
            .track_focus(&self.focus)
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .on_key_down(cx.listener(|_, _, _, cx| press(cx)))
            .on_any_mouse_down(cx.listener(|_, _, _, cx| press(cx)));
        if let Some(active) = cx
            .try_global::<OpeningOverlay>()
            .and_then(|overlay| overlay.active.as_ref())
        {
            if let Some(opening) = &active.opening
                && !opening.presented()
                && !self.asked_open
            {
                self.asked_open = true;
                window.on_next_frame(|_, cx| opening_presented(cx));
            }
            return root.children(opening_frame(active, window));
        }
        let Some(active) = cx
            .try_global::<SessionOverlay>()
            .and_then(|overlay| overlay.active.as_ref())
        else {
            return root;
        };
        if !self.asked && !active.presented {
            self.asked = true;
            window.on_next_frame(|_, cx| presented(cx));
        }
        let (closing, now) = (&active.closing, Instant::now());
        if closing.animating(now) {
            window.request_animation_frame();
        }
        let viewport = window.viewport_size();
        let progress = closing.progress(now);
        let pose = transition::layout(viewport, SHELL_CONTROLS_WIDTH, progress, closing.style());
        trace_frame("frame", active.begun, progress, &pose, &active.look);
        let farewell = closing
            .farewell()
            .map(|text| (text, closing.farewell_alpha(now)));
        root.children(paint::closing(
            &pose,
            progress,
            &active.shell_clock,
            &active.dial_clock,
            farewell,
            &active.look,
            viewport,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ready_file_exists_exactly_as_long_as_the_overlay_holds_it() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("dojo");
        let ready = ReadyFile::write_in(directory.clone()).expect("written");
        assert_eq!(
            std::fs::read_to_string(directory.join(handoff::OVERLAY_READY)).unwrap(),
            format!("{}\n", std::process::id())
        );
        drop(ready);
        assert!(!directory.join(handoff::OVERLAY_READY).exists());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    }

    #[derive(Default)]
    struct Recorder {
        actions: Vec<Action>,
        fail: bool,
    }

    impl ActionRunner for Recorder {
        fn run(&mut self, action: Action) -> Result<(), String> {
            self.actions.push(action);
            if self.fail {
                Err("refused".into())
            } else {
                Ok(())
            }
        }
    }

    /// Plays a closing to its end the way the shell's timer would.
    fn effects_of(occasion: Occasion, settings: SessionSettings) -> Vec<Effect> {
        let t0 = Instant::now();
        let (mut closing, mut effects) = Closing::begin(occasion, &settings, None, t0);
        while let Some(at) = closing.next_deadline() {
            effects.extend(closing.update(Event {
                now: at,
                kind: EventKind::Tick,
            }));
        }
        effects
    }

    #[test]
    fn each_closing_occasion_runs_its_action_once_through_the_runner() {
        let table = [
            (Occasion::Logout, Action::Logout),
            (Occasion::Suspend, Action::Suspend),
            (Occasion::Hibernate, Action::Hibernate),
            (Occasion::Restart, Action::Restart),
            (Occasion::Shutdown, Action::Shutdown),
        ];
        for (occasion, action) in table {
            for transition in [true, false] {
                let settings = SessionSettings {
                    transition,
                    ..Default::default()
                };
                let mut recorder = Recorder::default();
                let outcome = perform(&effects_of(occasion, settings), &mut recorder);
                assert_eq!(recorder.actions, [action], "{occasion:?} {transition}");
                assert!(outcome.done, "{occasion:?} {transition}");
            }
        }
    }

    #[test]
    fn a_lock_starts_the_locker_and_again_only_when_the_watchdog_fires() {
        let mut recorder = Recorder::default();
        let outcome = perform(
            &effects_of(Occasion::Lock, SessionSettings::default()),
            &mut recorder,
        );
        assert_eq!(recorder.actions, [Action::StartLocker, Action::StartLocker]);
        assert_eq!(
            outcome,
            Outcome {
                dismiss: true,
                done: true
            }
        );
        let mut plain = Recorder::default();
        let off = SessionSettings {
            transition: false,
            ..Default::default()
        };
        perform(&effects_of(Occasion::Lock, off), &mut plain);
        assert_eq!(plain.actions, [Action::StartLocker]);
    }

    #[test]
    fn a_failing_action_is_reported_and_does_not_stop_the_cover_from_coming_down() {
        let mut recorder = Recorder {
            fail: true,
            ..Default::default()
        };
        let outcome = perform(
            &effects_of(Occasion::Shutdown, SessionSettings::default()),
            &mut recorder,
        );
        assert_eq!(recorder.actions, [Action::Shutdown]);
        assert_eq!(
            outcome,
            Outcome {
                dismiss: true,
                done: true
            }
        );
    }

    #[test]
    fn without_a_shell_the_action_runs_directly() {
        for occasion in Occasion::ALL {
            let mut recorder = Recorder::default();
            let Some(departure) = Departure::of(occasion) else {
                continue;
            };
            run_directly(departure, &mut recorder).unwrap();
            assert_eq!(recorder.actions, [departure.action()], "{occasion:?}");
        }
        let mut recorder = Recorder::default();
        run_directly(Departure::parse("suspend-then-hibernate").unwrap(), &mut recorder).unwrap();
        assert_eq!(recorder.actions, [Action::SuspendThenHibernate]);
        let mut failing = Recorder {
            fail: true,
            ..Default::default()
        };
        assert!(run_directly(Departure::of(Occasion::Lock).unwrap(), &mut failing).is_err());
    }

    /// Plays a farewell-only closing to its end, as the shell's timer would.
    fn farewell_effects(occasion: Occasion) -> Vec<Effect> {
        let t0 = Instant::now();
        let (mut closing, mut effects) =
            Closing::farewell_only(occasion, &SessionSettings::default(), None, t0);
        while let Some(at) = closing.next_deadline() {
            effects.extend(closing.update(Event {
                now: at,
                kind: EventKind::Tick,
            }));
        }
        effects
    }

    #[test]
    fn a_farewell_closing_performs_nothing_through_the_runner() {
        for occasion in [
            Occasion::Logout,
            Occasion::Suspend,
            Occasion::Hibernate,
            Occasion::Restart,
            Occasion::Shutdown,
        ] {
            let mut recorder = Recorder::default();
            let outcome = perform(&farewell_effects(occasion), &mut recorder);
            assert!(recorder.actions.is_empty(), "{occasion:?}");
            assert_eq!(
                outcome,
                Outcome {
                    dismiss: true,
                    done: true
                },
                "{occasion:?}"
            );
        }
    }

    #[test]
    fn a_closing_started_by_the_shell_and_one_started_by_a_farewell_each_act_once() {
        let mut recorder = Recorder::default();
        perform(
            &effects_of(Occasion::Shutdown, SessionSettings::default()),
            &mut recorder,
        );
        perform(&farewell_effects(Occasion::Shutdown), &mut recorder);
        assert_eq!(recorder.actions, [Action::Shutdown]);
    }

    #[test]
    fn a_locker_asked_for_during_a_sleeping_closing_reaches_the_runner_once() {
        let t0 = Instant::now();
        let (mut closing, _) =
            Closing::farewell_only(Occasion::Suspend, &SessionSettings::default(), None, t0);
        let mut recorder = Recorder::default();
        perform(&closing.lock_requested(), &mut recorder);
        perform(&closing.lock_requested(), &mut recorder);
        assert_eq!(recorder.actions, [Action::StartLocker]);
    }

    #[cfg(feature = "record-session-actions")]
    #[test]
    fn the_recording_runner_receives_the_actions_in_order() {
        use dojo::session::action::RecordingRunner;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("actions");
        let mut recorder = RecordingRunner::to_file(&path);
        perform(
            &effects_of(Occasion::Shutdown, SessionSettings::default()),
            &mut recorder,
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "shutdown\n");
    }
}
