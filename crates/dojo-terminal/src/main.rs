//! A Rune front end for Rex terminal sessions: a window, or a layer-shell panel.
mod args;
mod backend;
mod bindings;
mod draw;
mod grid;
mod keys;
mod launch;
mod output;
mod picker;
mod reattach;
mod settings;
mod tabs;
mod term;
mod ui;

use rune::{App, AppSpec, DisplayTarget, KeyPolicy, NativeDisplayId, OverlayInput, OverlayOptions, OverlayPlacement, OverlaySpec};

use args::Panel;
use dojo_rex::Control;
use term::ARGS;

fn main() {
    let mut args = match args::parse(std::env::args().skip(1)) {
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
    let _ = term::THEME.set(settings.theme);
    resolve_session(&mut args);
    let panel = args.panel.clone();
    let title: &'static str = Box::leak(args.session.clone().unwrap_or_else(|| "Terminal".into()).into_boxed_str());
    rune::set_app_id(reattach::APP_ID);
    ARGS.set(args).expect("arguments set once");
    // Rune parses its own tooling flags from the command line; this app owns its arguments.
    // `DOJO_TERMINAL_RUNE_ARGS` hands Rune its headless flags (`--snap`, `--size`) for checks.
    let mut rune_args = vec!["dojo-terminal".to_owned()];
    rune_args.extend(std::env::var("DOJO_TERMINAL_RUNE_ARGS").unwrap_or_default().split_whitespace().map(str::to_owned));
    match panel {
        Some(panel) => run_panel(panel, rune_args, title),
        None => {
            let spec = AppSpec {
                setup: Some(term::setup),
                poll: Some(term::poll),
                on_key: Some(term::on_key),
                ..AppSpec::new(title, term::WINDOW_SIZE, term::init, ui::view)
            };
            App::new(spec).args(rune_args).run();
        }
    }
}

/// Settles which session to open: the one asked for, the most recent one for `--reattach`
/// (exiting instead when a window already shows it), `dojo`, or none for a bare `--picker`.
fn resolve_session(args: &mut args::Args) {
    if args.reattach {
        let live = Control::connect(&dojo_rex::default_socket(), "dojo-terminal-reattach")
            .and_then(|control| dojo_rex::list_sessions(&control))
            .unwrap_or_default();
        let label = reattach::choose(&dojo_rex::recent::load(), &live);
        if reattach::focus_existing(&label) {
            dojo_rex::recent::touch(&label);
            std::process::exit(0);
        }
        args.session = Some(label);
    } else if args.session.is_none() && !args.picker {
        args.session = Some(reattach::FALLBACK.to_owned());
    }
}

fn run_panel(panel: Panel, rune_args: Vec<String>, title: &'static str) {
    // The app's own state is unused: the panel is the only surface and has its own.
    let mut app = App::new(AppSpec::new(title, (1.0, 1.0), term::init, ui::view)).windowless().args(rune_args);
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
    let overlay = OverlaySpec::new("dojo-terminal", (panel.width, panel.height), ui::view)
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
