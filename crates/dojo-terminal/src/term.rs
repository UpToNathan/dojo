//! The terminal view's state and the handlers Rune calls: setup, poll, keys, layout.
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use dojo_rex::{Attachment, KeyAction, KeyInput, Options, Screen};
use rune::{Cx, El, KeyEvent, Tx, Waker, el, text};

use crate::args::Args;
use crate::backend::{Backend, Ended};
use crate::draw::{self, Metrics};
use crate::grid;
use crate::keys::{self, Held};

pub static ARGS: OnceLock<Args> = OnceLock::new();

/// Stand-in cell size until the probe has been laid out.
/// Until the probe is measured: a typical monospace advance at the font's size.
fn guess_cell() -> (f32, f32) {
    let font = draw::font();
    (font.size * 0.6, font.line_height)
}
/// The window's first size, before the compositor says otherwise.
pub const WINDOW_SIZE: (f32, f32) = (960.0, 600.0);
/// The probe is this many `M`s wide, so rounding its width to device pixels barely moves the
/// per-cell advance.
const PROBE_LEN: usize = 100;

pub struct Term {
    backend: Option<Backend>,
    screen: Option<Screen>,
    cell: Option<(f32, f32)>,
    area: Option<(f32, f32)>,
    grid: (u16, u16),
    held: Option<Held>,
    /// When a wake for key repeat is already scheduled.
    timer: Option<Instant>,
}

pub fn init() -> Term {
    Term { backend: None, screen: None, cell: None, area: None, grid: (0, 0), held: None, timer: None }
}

fn panel() -> bool {
    ARGS.get().is_some_and(|args| args.panel.is_some())
}

/// The configured colour file (`settings`), set once at start.
pub static THEME: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

/// Gives the session the configured colours on every open, so a theme switched since the last
/// open shows. A missing file leaves the session's colours as they are.
fn apply_theme(attachment: &Attachment) {
    let Some(Some(path)) = THEME.get() else { return };
    let Ok(source) = std::fs::read_to_string(path) else { return };
    if let Err(error) = attachment.set_theme(crate::settings::theme_payload(&source)) {
        eprintln!("dojo-terminal: applying {}: {error:#}", path.display());
    }
}

fn inset() -> f32 {
    if panel() { 0.0 } else { draw::WINDOW_INSET }
}

pub fn setup(s: &mut Term, wake: Waker) {
    let args = ARGS.get().expect("arguments parsed");
    let (width, height) = match &args.panel {
        Some(panel) => (panel.width, panel.height),
        None => (WINDOW_SIZE.0 - 2.0 * draw::WINDOW_INSET, WINDOW_SIZE.1 - 2.0 * draw::WINDOW_INSET),
    };
    let (cols, rows) = grid::cols_rows(width, height, guess_cell().0, guess_cell().1);
    let backend = if args.demo {
        Backend::demo(cols, rows)
    } else {
        Attachment::open(
            Options {
                socket: dojo_rex::default_socket(),
                label: args.session.clone(),
                command: args.command.clone(),
                cwd: args.cwd.clone(),
                cols,
                rows,
                client_name: "dojo-terminal".into(),
            },
            move || wake(),
        )
        .inspect(apply_theme)
        .map(Backend::Rex)
    };
    match backend {
        Ok(backend) => s.backend = Some(backend),
        Err(error) => {
            eprintln!("dojo-terminal: {error:#}");
            std::process::exit(1);
        }
    }
    s.grid = (cols, rows);
}

impl Term {
    fn metrics(&self) -> Metrics {
        let (cell_width, cell_height) = self.cell.unwrap_or_else(guess_cell);
        Metrics { cell_width, cell_height, inset: inset() }
    }

    /// Resizes the terminal when the area and cell size give a different grid.
    fn reconcile(&mut self, scale: f32) {
        let (Some((cw, ch)), Some((w, h))) = (self.cell, self.area) else { return };
        let grid = grid::cols_rows(w - 2.0 * inset(), h - 2.0 * inset(), cw, ch);
        if grid == self.grid {
            return;
        }
        self.grid = grid;
        if let Some(backend) = &mut self.backend {
            backend.resize(grid.0, grid.1, grid::cell_px(cw, scale), grid::cell_px(ch, scale));
        }
        self.refresh(true);
    }

    fn refresh(&mut self, force: bool) -> bool {
        match self.backend.as_mut().and_then(|backend| backend.screen(force)) {
            Some(screen) => {
                self.screen = Some(screen);
                true
            }
            None => false,
        }
    }

    fn send(&mut self, key: &KeyInput) {
        if let Some(backend) = &mut self.backend {
            backend.key(key);
        }
    }

    fn paste(&mut self, pasted: &str) {
        if pasted.is_empty() {
            return;
        }
        let clean = pasted.replace('\x1b', "");
        if let Some(backend) = &mut self.backend {
            backend.write(format!("\x1b[200~{clean}\x1b[201~").as_bytes());
        }
        self.refresh(false);
    }
}

pub fn view(s: &Term, cx: &mut Cx<Term>) -> El {
    let scale = cx.env().scale;
    let probe = text("M".repeat(PROBE_LEN))
        .font(draw::font())
        .key("probe")
        .absolute(-20000.0, 0.0)
        .on_layout(move |s: &mut Term, [_, _, w, h]| {
            s.cell = Some((w / PROBE_LEN as f32, h));
            s.reconcile(scale);
        });
    let body = match &s.screen {
        Some(screen) => draw::screen(screen, s.metrics()),
        None => el().w_full().h_full(),
    };
    el().key("terminal")
        .instant_changes()
        .w_full()
        .h_full()
        .on_layout(move |s: &mut Term, [_, _, w, h]| {
            s.area = Some((w, h));
            s.reconcile(scale);
        })
        .on_key_up(on_key_up)
        .child(body)
        .child(probe)
}

pub fn on_key(s: &mut Term, event: &KeyEvent, tx: &mut Tx) -> bool {
    if keys::is_paste(event) {
        tx.read_clipboard(|s: &mut Term, content, _| s.paste(&content.text()));
        return true;
    }
    let action = if tx.key_repeat() { KeyAction::Repeat } else { KeyAction::Press };
    let Some(input) = keys::key_input(event, tx.physical_key(), tx.key_base(), action) else { return false };
    s.send(&input);
    tx.instant();
    if panel() && action == KeyAction::Press {
        // The layer-shell path has no repeat of its own.
        s.held = Some(Held::new(input, Instant::now()));
        arm(s, keys::REPEAT_DELAY, tx);
    }
    s.refresh(false);
    true
}

fn on_key_up(s: &mut Term, event: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(input) = keys::key_input(event, tx.physical_key(), tx.key_base(), KeyAction::Release) else { return false };
    if s.held.as_ref().is_some_and(|held| held.input.code == input.code) {
        s.held = None;
    }
    s.send(&input);
    true
}

pub fn on_focus(s: &mut Term, focused: bool, _: &mut Tx) {
    if !focused {
        s.held = None;
    }
}

fn arm(s: &mut Term, after: Duration, tx: &mut Tx) {
    s.timer = Some(Instant::now() + after);
    tx.wake_after(after);
}

pub fn poll(s: &mut Term, _seconds: f64, tx: &mut Tx) -> bool {
    match s.backend.as_ref().and_then(Backend::ended) {
        Some(Ended::Clean) => std::process::exit(0),
        Some(Ended::Failed(error)) => {
            eprintln!("dojo-terminal: {error}");
            std::process::exit(1);
        }
        None => {}
    }
    // Every commit lands at once: a terminal shows output in the frame it arrives.
    tx.instant();
    let mut changed = s.refresh(false);
    let now = Instant::now();
    if s.timer.is_some_and(|at| at <= now) {
        s.timer = None;
    }
    if let Some(held) = &mut s.held {
        let (repeat, left) = held.due(now);
        if let Some(input) = repeat {
            s.send(&input);
            changed |= s.refresh(false);
        }
        if s.timer.is_none() {
            arm(s, left, tx);
        }
    }
    changed
}
