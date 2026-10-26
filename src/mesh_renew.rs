//! Keeps this machine's mesh memberships from lapsing. A few minutes after the shell starts, and then
//! every few hours, it runs `dojo-mesh renew --due`, which renews the roster entries that are within a
//! month of expiring, using only signing keys that need no touch. On a machine with no mesh, or no
//! signing key, the command has nothing to do and says nothing. It speaks up only when something is
//! due that it cannot renew, so the person is asked by hand (the mesh panel shows the same).

use std::process::{Command, Stdio};
use std::time::Duration;

const FIRST: Duration = Duration::from_secs(120);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// A reminder is not repeated more often than this.
const REMIND: Duration = Duration::from_secs(24 * 60 * 60);

pub fn start() {
    let _ = std::thread::Builder::new().name("dojo-mesh-renew".into()).spawn(|| {
        std::thread::sleep(FIRST);
        let mut reminded: Option<std::time::Instant> = None;
        loop {
            if let Some(message) = run_once() {
                if reminded.is_none_or(|at| at.elapsed() >= REMIND) {
                    eprintln!("Dojo mesh: {message}");
                    reminded = Some(std::time::Instant::now());
                }
            }
            std::thread::sleep(EVERY);
        }
    });
}

/// What to tell the person, if anything.
fn run_once() -> Option<String> {
    let output = Command::new(crate::mesh_ui::mesh_binary()).args(["renew", "--due"]).stdin(Stdio::null()).output().ok()?;
    match output.status.code() {
        Some(0) => {
            let said = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = said.lines().find(|line| line.starts_with("renewed ")) {
                eprintln!("Dojo mesh: {line}");
            }
            None
        }
        // Not a roster node: nothing to keep up.
        Some(2) => None,
        _ => Some(String::from_utf8_lossy(&output.stderr).lines().rev().find(|line| !line.trim().is_empty())?.trim().trim_start_matches("dojo-mesh: ").to_string()),
    }
}
