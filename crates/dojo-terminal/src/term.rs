//! The terminal window's state and the handlers Rune calls: setup, poll, keys, layout.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use dojo_rex::layout::Rect;
use dojo_rex::{Control, KeyAction, KeyInput, Screen, Session, SessionOptions, SessionSummary, SessionView, Target};
use rune::{Key, KeyEvent, Tx, Waker};

use crate::args::Args;
use crate::backend::Backend;
use crate::bindings::{self, Action};
use crate::draw::{self, Metrics};
use crate::grid;
use crate::keys::{self, Held};
use crate::picker::{self, Accept, Picker};
use crate::{launch, tabs};

pub static ARGS: OnceLock<Args> = OnceLock::new();
/// The configured colour file (`settings`), set once at start.
pub static THEME: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();

/// Until the probe is measured: a typical monospace advance at the font's size.
pub fn guess_cell() -> (f32, f32) {
    let font = draw::font();
    (font.size * 0.6, font.line_height)
}
/// The window's first size, before the compositor says otherwise.
pub const WINDOW_SIZE: (f32, f32) = (960.0, 600.0);
/// The gap between panes, which shows the separator colour.
pub const GAP: f32 = 1.0;

#[derive(Default)]
pub struct Pane {
    pub screen: Option<Screen>,
    grid: (u16, u16),
    /// The pixels of the screen's images, by image key, kept while shown.
    pub pixels: HashMap<String, rune::Pixels>,
}

pub struct Term {
    /// The window title shown, which follows the session's label.
    title: String,
    backend: Option<Backend>,
    pub view: SessionView,
    pub panes: HashMap<String, Pane>,
    themed: HashSet<String>,
    pub cell: Option<(f32, f32)>,
    pub area: Option<(f32, f32)>,
    pub scale: f32,
    held: Option<Held>,
    /// When a wake for key repeat is already scheduled.
    timer: Option<Instant>,
    pub picker: Option<Picker>,
    listing: Arc<Mutex<Option<Vec<SessionSummary>>>>,
    waker: Option<Waker>,
}

pub fn init() -> Term {
    Term {
        title: String::new(),
        backend: None,
        view: SessionView::default(),
        panes: HashMap::new(),
        themed: HashSet::new(),
        cell: None,
        area: None,
        scale: 1.0,
        held: None,
        timer: None,
        picker: None,
        listing: Arc::default(),
        waker: None,
    }
}

pub fn panel() -> bool {
    ARGS.get().is_some_and(|args| args.panel.is_some())
}

pub fn inset() -> f32 {
    if panel() { 0.0 } else { draw::WINDOW_INSET }
}

/// The window's size before layout has reported it.
fn first_size() -> (f32, f32) {
    match ARGS.get().and_then(|args| args.panel.as_ref()) {
        Some(panel) => (panel.width, panel.height),
        None => WINDOW_SIZE,
    }
}

fn theme() -> Option<serde_json::Value> {
    let path = THEME.get()?.as_ref()?;
    let source = std::fs::read_to_string(path).ok()?;
    Some(crate::settings::theme_payload(&source))
}

/// Opens `target` as a view of its Rex session. The first open honours `-- COMMAND` and `--cwd`;
/// later ones, and every new pane, run the desktop's shell.
fn open_session(target: Target, first: bool, grid: (u16, u16), wake: Waker) -> anyhow::Result<Backend> {
    let args = ARGS.get().expect("arguments parsed");
    let shell = launch::from_environment();
    let command = if first && !args.command.is_empty() { args.command.clone() } else { shell.clone() };
    let session = Session::open(
        SessionOptions {
            socket: dojo_rex::default_socket(),
            target,
            command,
            pane_command: shell,
            cwd: if first { args.cwd.clone() } else { None },
            cols: grid.0,
            rows: grid.1,
            client_name: "dojo-terminal".into(),
        },
        move || wake(),
    )?;
    dojo_rex::recent::touch(session.id());
    Ok(Backend::Rex(session))
}

pub fn setup(s: &mut Term, wake: Waker) {
    let args = ARGS.get().expect("arguments parsed");
    s.waker = Some(wake.clone());
    let grid = s.initial_grid();
    let opened = if args.demo {
        Some(Backend::demo(grid.0, grid.1))
    } else {
        args.target.clone().map(|target| open_session(target, true, grid, wake))
    };
    match opened {
        Some(Ok(backend)) => s.backend = Some(backend),
        Some(Err(error)) => {
            eprintln!("dojo-terminal: {error:#}");
            std::process::exit(1);
        }
        None => {}
    }
    if args.picker {
        s.open_picker();
    }
    s.refresh_view();
}

impl Term {
    pub fn metrics(&self) -> Metrics {
        let (cell_width, cell_height) = self.cell.unwrap_or_else(guess_cell);
        Metrics { cell_width, cell_height, inset: 0.0, scale: self.scale }
    }

    /// The window's logical size.
    pub fn size(&self) -> (f32, f32) {
        self.area.unwrap_or_else(first_size)
    }

    /// Where the panes go: inside the window's inset and below the tab bar.
    pub fn pane_area(&self) -> Rect {
        let (w, h) = self.size();
        let bar = if tabs::visible(&self.view) { tabs::height(self.metrics()) } else { 0.0 };
        let i = inset();
        [i, i + bar, (w - 2.0 * i).max(1.0), (h - 2.0 * i - bar).max(1.0)]
    }

    pub fn layout(&self) -> Vec<(String, Rect)> {
        self.view.panes(self.pane_area(), GAP)
    }

    pub fn focused(&self) -> Option<String> {
        self.view.focused_block()
    }

    fn initial_grid(&self) -> (u16, u16) {
        let [_, _, w, h] = self.pane_area();
        let (cw, ch) = self.cell.unwrap_or_else(guess_cell);
        grid::cols_rows(w, h, cw, ch)
    }

    /// Takes the session's view when it changed. True when the panes need redrawing.
    fn refresh_view(&mut self) -> bool {
        let Some(backend) = &self.backend else { return false };
        let view = backend.view();
        if view == self.view {
            return false;
        }
        self.view = view;
        let blocks: HashSet<String> = self.view.windows.iter().filter_map(|w| w.layout.as_ref()).flat_map(|l| l.blocks()).collect();
        self.panes.retain(|block, _| blocks.contains(block));
        self.themed.retain(|block| blocks.contains(block));
        let fresh: Vec<&String> = blocks.iter().filter(|block| !self.themed.contains(*block)).collect();
        if !fresh.is_empty() {
            if let Some(theme) = theme() {
                for block in &fresh {
                    backend.set_theme(block, theme.clone());
                }
            }
            let fresh: Vec<String> = fresh.into_iter().cloned().collect();
            self.themed.extend(fresh);
        }
        self.sync();
        true
    }

    /// Sizes every visible pane to its rectangle, and fetches the screens it hasn't got yet.
    pub fn sync(&mut self) {
        let Some((cw, ch)) = self.cell else { return };
        let (scale, layout) = (self.scale, self.layout());
        let Some(backend) = &mut self.backend else { return };
        for (block, [_, _, w, h]) in layout {
            let pane = self.panes.entry(block.clone()).or_default();
            let grid = grid::cols_rows(w, h, cw, ch);
            if pane.grid != grid {
                pane.grid = grid;
                backend.resize(&block, grid.0, grid.1, grid::cell_px(cw, scale), grid::cell_px(ch, scale));
            }
            if pane.screen.is_none() {
                pane.screen = backend.screen(&block, true);
            }
        }
    }

    /// Reads the visible panes that changed. True when any did.
    fn refresh_panes(&mut self) -> bool {
        let layout = self.layout();
        let Some(backend) = &mut self.backend else { return false };
        let mut changed = false;
        for (block, _) in layout {
            let pane = self.panes.entry(block.clone()).or_default();
            if let Some(screen) = backend.screen(&block, false) {
                pane.pixels.retain(|key, _| screen.images.iter().any(|image| &image.key == key));
                for image in &screen.images {
                    pane.pixels.entry(image.key.clone()).or_insert_with(|| {
                        let pixels = rune::Pixels::new();
                        pixels.set(image.width, image.height, image.rgba.to_vec());
                        pixels
                    });
                }
                pane.screen = Some(screen);
                changed = true;
            }
        }
        changed
    }

    pub fn focus_tab(&mut self, window: &str) {
        if let Some(backend) = &self.backend {
            backend.focus_tab(window);
        }
    }

    pub fn focus_pane(&mut self, block: &str) {
        if let Some(backend) = &self.backend {
            backend.focus_pane(block);
        }
    }

    fn send(&mut self, key: &KeyInput) {
        let Some(block) = self.focused() else { return };
        if let Some(backend) = &mut self.backend {
            backend.key(&block, key);
        }
    }

    fn paste(&mut self, pasted: &str) {
        let Some(block) = self.focused() else { return };
        if pasted.is_empty() {
            return;
        }
        let clean = pasted.replace('\x1b', "");
        if let Some(backend) = &mut self.backend {
            backend.write(&block, format!("\x1b[200~{clean}\x1b[201~").as_bytes());
        }
        self.refresh_panes();
    }

    fn open_picker(&mut self) {
        self.picker = Some(Picker::default());
        let (slot, wake) = (self.listing.clone(), self.waker.clone());
        std::thread::spawn(move || {
            let listed = Control::connect(&dojo_rex::default_socket(), "dojo-terminal-picker").and_then(|control| dojo_rex::list_sessions(&control));
            let entries = listed.unwrap_or_else(|error| {
                eprintln!("dojo-terminal: listing sessions: {error:#}");
                Vec::new()
            });
            *slot.lock().unwrap() = Some(entries);
            if let Some(wake) = wake {
                wake();
            }
        });
    }

    /// Shows another session in this window.
    fn switch(&mut self, target: Target) {
        let Some(wake) = self.waker.clone() else { return };
        match open_session(target.clone(), false, self.initial_grid(), wake) {
            Ok(backend) => {
                self.backend = Some(backend);
                self.view = SessionView::default();
                self.panes.clear();
                self.themed.clear();
                self.held = None;
                self.picker = None;
                self.refresh_view();
            }
            Err(error) => eprintln!("dojo-terminal: opening {target:?}: {error:#}"),
        }
    }

    fn picker_key(&mut self, event: &KeyEvent, tx: &mut Tx) -> bool {
        if bindings::action(event, tx.physical_key()) == Some(Action::Picker) {
            self.close_picker();
            return true;
        }
        let Some(picker) = &mut self.picker else { return false };
        let typing = !(event.mods.ctrl || event.mods.cmd || event.mods.alt || event.mods.logo);
        match &event.key {
            Key::Escape => self.close_picker(),
            Key::Up => picker.step(false),
            Key::Down => picker.step(true),
            Key::Backspace => picker.backspace(),
            Key::Text(typed) if typing && !typed.chars().any(char::is_control) => picker.type_text(typed),
            Key::Enter => match picker.accept() {
                Accept::Switch(id) => self.switch(Target::Id(id)),
                Accept::Create(label) => self.switch(Target::Label(label)),
                Accept::Nothing => {}
            },
            _ => {}
        }
        true
    }

    fn close_picker(&mut self) {
        self.picker = None;
        if self.backend.is_none() {
            std::process::exit(0);
        }
    }

    fn run(&mut self, action: Action, tx: &mut Tx) {
        match action {
            Action::Picker => self.open_picker(),
            Action::Paste => tx.read_clipboard(|s: &mut Term, content, _| s.paste(&content.text())),
            other => {
                if let Some(backend) = &self.backend {
                    backend.act(other);
                }
            }
        }
    }
}

pub fn on_key(s: &mut Term, event: &KeyEvent, tx: &mut Tx) -> bool {
    tx.instant();
    if s.picker.is_some() {
        return s.picker_key(event, tx);
    }
    if let Some(action) = bindings::action(event, tx.physical_key()) {
        if !tx.key_repeat() {
            s.run(action, tx);
        }
        return true;
    }
    let action = if tx.key_repeat() { KeyAction::Repeat } else { KeyAction::Press };
    let Some(input) = keys::key_input(event, tx.physical_key(), tx.key_base(), action) else { return false };
    s.send(&input);
    if panel() && action == KeyAction::Press {
        // The layer-shell path has no repeat of its own.
        s.held = Some(Held::new(input, Instant::now()));
        arm(s, keys::REPEAT_DELAY, tx);
    }
    s.refresh_panes();
    true
}

pub fn on_key_up(s: &mut Term, event: &KeyEvent, tx: &mut Tx) -> bool {
    if s.picker.is_some() {
        return true;
    }
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
    if let Some(label) = s.backend.as_ref().map(|backend| backend.view().label).filter(|label| !label.is_empty() && *label != s.title) {
        tx.set_title(label.clone());
        s.title = label;
    }
    if s.backend.as_ref().and_then(Backend::ended).is_some() {
        std::process::exit(0);
    }
    // Every commit lands at once: a terminal shows output in the frame it arrives.
    tx.instant();
    let mut changed = false;
    let listed = s.listing.lock().unwrap().take();
    if let (Some(entries), Some(picker)) = (listed, &mut s.picker) {
        picker.entries = Some(picker::order(entries, &dojo_rex::recent::load()));
        changed = true;
    }
    changed |= s.refresh_view();
    changed |= s.refresh_panes();
    let now = Instant::now();
    if s.timer.is_some_and(|at| at <= now) {
        s.timer = None;
    }
    if let Some(held) = &mut s.held {
        let (repeat, left) = held.due(now);
        if let Some(input) = repeat {
            s.send(&input);
            changed |= s.refresh_panes();
        }
        if s.timer.is_none() {
            arm(s, left, tx);
        }
    }
    changed
}
