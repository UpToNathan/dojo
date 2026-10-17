//! Session vocabulary shared by the resident shell and the lock program.
//!
//! Everything but [`paint`] is pure and holds no GPUI `Window` or `Context`.
//! [`paint`] draws the closed state for both programs. The only host access is
//! the one-file-per-occasion persistence boundary in [`history`] and the config
//! read in [`settings`].

pub mod action;
pub mod closing;
pub mod handoff;
pub mod history;
pub mod host;
pub mod login;
pub mod logind;
pub mod message;
pub mod opening;
pub mod paint;
pub mod settings;
pub mod transition;

/// Why the desktop frame closes or opens. See the glossary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Occasion {
    Lock,
    Unlock,
    Login,
    Logout,
    Suspend,
    Hibernate,
    Restart,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Closing,
    Opening,
}

impl Occasion {
    pub const ALL: [Occasion; 8] = [
        Occasion::Lock,
        Occasion::Unlock,
        Occasion::Login,
        Occasion::Logout,
        Occasion::Suspend,
        Occasion::Hibernate,
        Occasion::Restart,
        Occasion::Shutdown,
    ];

    pub fn direction(self) -> Direction {
        match self {
            Occasion::Unlock | Occasion::Login => Direction::Opening,
            Occasion::Lock
            | Occasion::Logout
            | Occasion::Suspend
            | Occasion::Hibernate
            | Occasion::Restart
            | Occasion::Shutdown => Direction::Closing,
        }
    }

    /// Stable lower-case name, used as the history file name.
    pub fn key(self) -> &'static str {
        match self {
            Occasion::Lock => "lock",
            Occasion::Unlock => "unlock",
            Occasion::Login => "login",
            Occasion::Logout => "logout",
            Occasion::Suspend => "suspend",
            Occasion::Hibernate => "hibernate",
            Occasion::Restart => "restart",
            Occasion::Shutdown => "shutdown",
        }
    }
}

/// What asked for a lock. Optional: a lock without a reason uses the plain pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Manual,
    Idle,
    Suspend,
}

impl Reason {
    /// An unknown reason is `None`, which selects the plain pool.
    pub fn parse(text: &str) -> Option<Reason> {
        match text {
            "manual" => Some(Reason::Manual),
            "idle" => Some(Reason::Idle),
            "suspend" => Some(Reason::Suspend),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_occasions_are_unlock_and_login_only() {
        let opening: Vec<_> = Occasion::ALL
            .into_iter()
            .filter(|occasion| occasion.direction() == Direction::Opening)
            .collect();
        assert_eq!(opening, [Occasion::Unlock, Occasion::Login]);
    }

    #[test]
    fn keys_are_unique_file_names() {
        let mut keys: Vec<_> = Occasion::ALL.into_iter().map(Occasion::key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Occasion::ALL.len());
        assert_eq!(Occasion::Hibernate.key(), "hibernate");
    }

    #[test]
    fn reason_parses_known_words_and_ignores_unknown() {
        assert_eq!(Reason::parse("idle"), Some(Reason::Idle));
        assert_eq!(Reason::parse("manual"), Some(Reason::Manual));
        assert_eq!(Reason::parse("suspend"), Some(Reason::Suspend));
        assert_eq!(Reason::parse("lid"), None);
    }
}
