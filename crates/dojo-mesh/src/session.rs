//! Whether the operator's graphical session is locked. Niri quietly ignores requests to start
//! programs while the screen is locked, so a window sent to a locked machine would vanish.
//! Dojo's lock screen sets logind's `LockedHint`, which is what this reads.

use std::process::{Command, Stdio};

/// The `Display` session of `loginctl show-user` output (the user's graphical session).
pub fn display_session(show_user: &str) -> Option<String> {
    show_user.lines().find_map(|line| line.strip_prefix("Display=")).map(str::trim).filter(|id| !id.is_empty()).map(String::from)
}

/// `LockedHint` of `loginctl show-session` output.
pub fn locked_hint(show_session: &str) -> Option<bool> {
    show_session.lines().find_map(|line| line.strip_prefix("LockedHint=")).map(|value| value.trim() == "yes")
}

fn loginctl(args: &[&str]) -> Option<String> {
    let output = Command::new("loginctl").args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    output.status.success().then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether this user's graphical session is locked; `None` when it cannot be told.
pub fn locked() -> Option<bool> {
    let user = std::env::var("USER").ok().filter(|user| !user.is_empty())?;
    let session = display_session(&loginctl(&["show-user", &user, "-p", "Display"])?)?;
    locked_hint(&loginctl(&["show-session", &session, "-p", "LockedHint"])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_graphical_session_and_its_lock() {
        assert_eq!(display_session("Display=c2\n").as_deref(), Some("c2"));
        assert_eq!(display_session("Display=\n"), None);
        assert_eq!(locked_hint("LockedHint=yes\n"), Some(true));
        assert_eq!(locked_hint("LockedHint=no\n"), Some(false));
        assert_eq!(locked_hint("Other=1\n"), None);
    }
}
