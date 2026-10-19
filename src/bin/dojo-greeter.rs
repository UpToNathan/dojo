//! `dojo-greeter`: the lock program in login mode. It takes the session lock in the greeter's
//! Niri exactly as `dojo-lock` does, draws the lock screen with a user chip and a session chip,
//! and carries out the model's effects over the display manager's socket.
//!
//! Exit status: 0 once the display manager has started the session, leaving the last frame as
//! the closed state the shell's login opening continues from; 75 when the event loop ends any
//! other way (a lost compositor or display manager); 69 when the display manager cannot be
//! reached; 78 when it offers nothing to log in to or as; 64 for unknown arguments. Every
//! non-zero status makes `dojo-dm` start the greeter again.

use std::process::ExitCode;

use dojo::dm::client::{self, Client, HANDSHAKE_TIMEOUT};
use dojo::dm::greeter::{Answer, Bridge, first_human, session_names};
use dojo::lock::Login;

#[path = "dojo-greeter/app.rs"]
mod app;
#[path = "dojo-greeter/looks.rs"]
mod looks;

const EXIT_USAGE: u8 = 64;
const EXIT_UNAVAILABLE: u8 = 69;
const EXIT_CONFIG: u8 = 78;

fn main() -> ExitCode {
    app::mark_start();
    dojo::lock::secret::protect_process();
    let Some(reduced_motion) = options(std::env::args_os().skip(1).collect()) else {
        eprintln!("usage: dojo-greeter");
        return ExitCode::from(EXIT_USAGE);
    };

    let path = client::socket_path();
    let client = match Client::connect(&path) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("dojo-greeter: could not connect to {}: {error}", path.display());
            return ExitCode::from(EXIT_UNAVAILABLE);
        }
    };
    let init = match client.handshake(HANDSHAKE_TIMEOUT) {
        Ok(init) => init,
        Err(error) => {
            eprintln!("dojo-greeter: {error}");
            return ExitCode::from(EXIT_UNAVAILABLE);
        }
    };
    let Some(account) = init.last_login.clone().or_else(|| {
        std::fs::read_to_string("/etc/passwd")
            .ok()
            .and_then(|passwd| first_human(&passwd))
    }) else {
        eprintln!("dojo-greeter: there is no account to log in as");
        return ExitCode::from(EXIT_CONFIG);
    };
    let names = session_names(&init.sessions);
    let last = init
        .last_session
        .as_ref()
        .and_then(|id| init.sessions.iter().position(|entry| &entry.id == id))
        .and_then(|index| names.get(index).cloned())
        .unwrap_or_default();
    let login = match Login::new(account.as_str(), names, &last) {
        Ok(login) => login,
        Err(error) => {
            eprintln!("dojo-greeter: the display manager's init is unusable: {error:?}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    let answer = Answer {
        login: account.as_str().to_owned(),
        display_name: None,
        fingerprint: false,
    };
    let root = looks::state_directory();
    let initial = std::thread::spawn(move || looks::load(&root, answer));
    ExitCode::from(app::run(app::Options {
        client,
        bridge: Bridge::new(init.sessions),
        account,
        login,
        initial,
        reduced_motion,
    }))
}

/// Release builds take no arguments.
#[cfg(not(feature = "fake-pam-stack"))]
fn options(arguments: Vec<std::ffi::OsString>) -> Option<bool> {
    arguments.is_empty().then_some(false)
}

/// Test builds accept `--reduce-motion`, because GPUI on Linux never reports a desktop setting.
#[cfg(feature = "fake-pam-stack")]
fn options(arguments: Vec<std::ffi::OsString>) -> Option<bool> {
    let mut reduced_motion = false;
    for flag in arguments {
        match flag.to_str()? {
            "--reduce-motion" => reduced_motion = true,
            _ => return None,
        }
    }
    Some(reduced_motion)
}
