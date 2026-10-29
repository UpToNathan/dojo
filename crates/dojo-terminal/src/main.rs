//! A Rune front end for Rex terminal sessions: a window, or a layer-shell panel.
mod args;
mod backend;
mod draw;
mod grid;
mod keys;
mod output;
mod settings;
mod term;

use rune::{App, AppSpec, DisplayTarget, KeyPolicy, NativeDisplayId, OverlayInput, OverlayOptions, OverlayPlacement, OverlaySpec};

use args::Panel;
use term::ARGS;

fn main() {
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("dojo-terminal: {message}\n{}", args::USAGE);
            std::process::exit(2);
        }
    };
    let settings = settings::load().unwrap_or_else(|error| {
        eprintln!("dojo-terminal: {error:#}; using the default settings");
        settings::Settings::default()
    });
    let _ = draw::FONT.set(settings.font.style());
    let _ = backend::CURSOR.set(settings.cursor);
    let panel = args.panel.clone();
    ARGS.set(args).expect("arguments set once");
    // Rune parses its own tooling flags from the command line; this app owns its arguments.
    let rune_args = ["dojo-terminal".to_owned()];
    match panel {
        Some(panel) => run_panel(panel, rune_args),
        None => {
            let spec = AppSpec {
                setup: Some(term::setup),
                poll: Some(term::poll),
                on_key: Some(term::on_key),
                ..AppSpec::new("Terminal", term::WINDOW_SIZE, term::init, term::view)
            };
            App::new(spec).args(rune_args).run();
        }
    }
}

fn run_panel(panel: Panel, rune_args: [String; 1]) {
    // The app's own state is unused: the panel is the only surface and has its own.
    let mut app = App::new(AppSpec::new("Terminal", (1.0, 1.0), term::init, term::view)).windowless().args(rune_args);
    let display = panel
        .output
        .as_deref()
        .and_then(|name| {
            let global = output::global_for(name);
            if global.is_none() {
                eprintln!("dojo-terminal: no output named {name}; using the default");
            }
            global
        })
        .map_or(DisplayTarget::Primary, |global| DisplayTarget::Native(NativeDisplayId::Wayland(global)));
    let options = OverlayOptions {
        placement: Some(OverlayPlacement::Rect { x: panel.x, y: panel.y, width: panel.width, height: panel.height }),
        display,
        keys: KeyPolicy::OnDemand,
        input: OverlayInput::All,
        ..Default::default()
    };
    let overlay = OverlaySpec::new("dojo-terminal", (panel.width, panel.height), term::view)
        .options(options)
        .setup(term::setup)
        .poll(term::poll)
        .on_key(term::on_key)
        .on_focus(term::on_focus);
    let handle = app.overlay(overlay, term::init());
    handle.show();
    handle.request_keys();
    app.run();
}
