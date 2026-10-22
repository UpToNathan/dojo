//! `dojo-lock`: takes the session lock with `ext-session-lock-v1` and draws
//! the lock screen on every output until a PAM conversation succeeds.
//!
//! Exit status: 0 when unlocked, refused (another locker holds the lock) or
//! ended by the compositor; 75 when the event loop ends any other way, such
//! as a lost connection, so a supervisor restarts it while the session stays
//! locked; 64 for unknown arguments.

use std::process::ExitCode;

#[path = "dojo-lock/app.rs"]
mod app;
#[path = "dojo-lock/host.rs"]
mod host;
#[path = "dojo-lock/lifecycle.rs"]
mod lifecycle;

use dojo::lock::Settings;
use dojo::lock::worker::Helper;
use dojo::session::transition::SHELL_CONTROLS_WIDTH;

const EXIT_USAGE: u8 = 64;

fn main() -> ExitCode {
    app::mark_start();
    dojo::lock::secret::protect_process();
    let wallpaper = std::thread::spawn(host::decode_wallpaper);
    let Some((helper, reduced_motion)) = arguments(std::env::args_os().skip(1).collect()) else {
        eprintln!("usage: dojo-lock");
        return ExitCode::from(EXIT_USAGE);
    };

    let account = host::Account::current();
    let session = dojo::config::ConfigStore::user_default()
        .and_then(|store| store.session_settings())
        .unwrap_or_else(|error| {
            eprintln!("dojo-lock: using default session settings: {error:#}");
            Default::default()
        });
    let marker = lifecycle::Marker::from_env();
    let settings = Settings {
        reduced_motion: reduced_motion || session.reduced_motion,
        user: account.full_name(),
        greeting: host::greetings(session.greeting, &account, app::now()),
        controls_width: SHELL_CONTROLS_WIDTH,
        lock_age_origin: marker.as_ref().and_then(lifecycle::Marker::read),
        farewell: host::take_farewell(session.farewell),
        login: None,
    };
    ExitCode::from(app::run(app::Options {
        settings,
        wallpaper,
        helper,
        marker,
    }))
}

/// The PAM helper beside this binary, and whether to force reduced motion.
fn arguments(arguments: Vec<std::ffi::OsString>) -> Option<(Helper, bool)> {
    let program = std::env::current_exe()
        .ok()?
        .with_file_name("dojo-lock-pam");
    options(Helper::new(program), arguments)
}

/// Release builds take no arguments.
#[cfg(not(feature = "fake-pam-stack"))]
fn options(helper: Helper, arguments: Vec<std::ffi::OsString>) -> Option<(Helper, bool)> {
    arguments.is_empty().then_some((helper, false))
}

/// Test builds accept `--pam-confdir DIR` for stand-in stacks and
/// `--reduce-motion`, because GPUI on Linux never reports a desktop setting.
#[cfg(feature = "fake-pam-stack")]
fn options(mut helper: Helper, arguments: Vec<std::ffi::OsString>) -> Option<(Helper, bool)> {
    let mut reduced_motion = false;
    let mut rest = arguments.into_iter();
    while let Some(flag) = rest.next() {
        match flag.to_str()? {
            "--pam-confdir" => helper = helper.with_confdir(rest.next()?.into()),
            "--reduce-motion" => reduced_motion = true,
            _ => return None,
        }
    }
    Some((helper, reduced_motion))
}
