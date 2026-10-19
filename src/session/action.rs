//! What the shell does when a closing ends, and the seam that lets tests record it.
//!
//! Real power actions are never run by tests or agents: the shell's runner is
//! the only implementation that touches the machine, and the recording runner
//! exists only in test builds and under the `record-session-actions` feature.

use super::Occasion;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    StartLocker,
    Logout,
    Suspend,
    /// Suspend now and hibernate after the delay in `sleep.conf`. Stasis's idle plan asks for it.
    SuspendThenHibernate,
    Hibernate,
    Restart,
    Shutdown,
}

impl Action {
    /// Stable lower-case name, written one per line by the recording runner.
    pub fn key(self) -> &'static str {
        match self {
            Action::StartLocker => "start-locker",
            Action::Logout => "logout",
            Action::Suspend => "suspend",
            Action::SuspendThenHibernate => "suspend-then-hibernate",
            Action::Hibernate => "hibernate",
            Action::Restart => "restart",
            Action::Shutdown => "shutdown",
        }
    }
}

impl Occasion {
    /// The action a closing performs. Opening occasions have none.
    pub fn action(self) -> Option<Action> {
        match self {
            Occasion::Lock => Some(Action::StartLocker),
            Occasion::Logout => Some(Action::Logout),
            Occasion::Suspend => Some(Action::Suspend),
            Occasion::Hibernate => Some(Action::Hibernate),
            Occasion::Restart => Some(Action::Restart),
            Occasion::Shutdown => Some(Action::Shutdown),
            Occasion::Unlock | Occasion::Login => None,
        }
    }
}

impl Occasion {
    /// The machine sleeps when this occasion's action runs, so it must not be held awake.
    pub fn sleeps(self) -> bool {
        matches!(self, Occasion::Suspend | Occasion::Hibernate)
    }
}

/// An occasion and the action that ends it. Most occasions have one action; a
/// suspend can also end as suspend-then-hibernate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Departure {
    occasion: Occasion,
    action: Action,
}

/// The words `dojo --leave` accepts. Each occasion's own action comes first.
const DEPARTURES: [(&str, Occasion, Action); 7] = [
    ("lock", Occasion::Lock, Action::StartLocker),
    ("logout", Occasion::Logout, Action::Logout),
    ("suspend", Occasion::Suspend, Action::Suspend),
    ("hibernate", Occasion::Hibernate, Action::Hibernate),
    ("restart", Occasion::Restart, Action::Restart),
    ("shutdown", Occasion::Shutdown, Action::Shutdown),
    ("suspend-then-hibernate", Occasion::Suspend, Action::SuspendThenHibernate),
];

impl Departure {
    /// The departure an occasion makes by default. Opening occasions have none.
    pub fn of(occasion: Occasion) -> Option<Departure> {
        occasion.action().map(|action| Departure { occasion, action })
    }

    /// The departure named `word`, as `dojo --leave` spells it.
    pub fn parse(word: &str) -> Option<Departure> {
        DEPARTURES
            .iter()
            .find(|(name, ..)| *name == word)
            .map(|&(_, occasion, action)| Departure { occasion, action })
    }

    /// The occasion `dojo --farewell` may play for `word`: a closing that ends in
    /// the machine or the session going away. A lock plays its own farewell in the
    /// locker, and suspend-then-hibernate looks the same as a suspend.
    pub fn parse_farewell(word: &str) -> Option<Occasion> {
        Departure::parse(word)
            .filter(|departure| {
                departure.action != Action::StartLocker
                    && Some(departure.action) == departure.occasion.action()
            })
            .map(|departure| departure.occasion)
    }

    pub fn occasion(self) -> Occasion {
        self.occasion
    }

    pub fn action(self) -> Action {
        self.action
    }

    /// The word that names this departure to `dojo --leave`.
    pub fn word(self) -> &'static str {
        DEPARTURES
            .iter()
            .find(|(_, occasion, action)| *occasion == self.occasion && *action == self.action)
            .map(|(name, ..)| *name)
            .expect("every departure is in the table")
    }
}

pub trait ActionRunner {
    fn run(&mut self, action: Action) -> Result<(), String>;
}

/// Records actions in order instead of performing them.
///
/// Compiled only for tests and under `record-session-actions`, so a release
/// build cannot construct one.
#[cfg(any(test, feature = "record-session-actions"))]
#[derive(Debug, Default)]
pub struct RecordingRunner {
    pub actions: Vec<Action>,
    file: Option<std::path::PathBuf>,
}

#[cfg(any(test, feature = "record-session-actions"))]
impl RecordingRunner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Also appends each action's [`Action::key`] as a line to `path`.
    pub fn to_file(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            actions: Vec::new(),
            file: Some(path.into()),
        }
    }
}

#[cfg(any(test, feature = "record-session-actions"))]
impl ActionRunner for RecordingRunner {
    fn run(&mut self, action: Action) -> Result<(), String> {
        use std::io::Write;
        if let Some(path) = &self.file {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            writeln!(file, "{}", action.key())
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
        self.actions.push(action);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Direction;

    #[test]
    fn occasions_map_to_their_actions() {
        let table = [
            (Occasion::Lock, Some(Action::StartLocker)),
            (Occasion::Unlock, None),
            (Occasion::Login, None),
            (Occasion::Logout, Some(Action::Logout)),
            (Occasion::Suspend, Some(Action::Suspend)),
            (Occasion::Hibernate, Some(Action::Hibernate)),
            (Occasion::Restart, Some(Action::Restart)),
            (Occasion::Shutdown, Some(Action::Shutdown)),
        ];
        for (occasion, action) in table {
            assert_eq!(occasion.action(), action, "{occasion:?}");
        }
    }

    #[test]
    fn only_closing_occasions_can_be_left() {
        for occasion in Occasion::ALL {
            let departure = Departure::of(occasion);
            assert_eq!(departure.is_some(), occasion.action().is_some(), "{occasion:?}");
            if let Some(departure) = departure {
                assert_eq!(Departure::parse(departure.word()), Some(departure), "{occasion:?}");
                assert_eq!(departure.occasion(), occasion);
                assert_eq!(Some(departure.action()), occasion.action());
                assert_eq!(departure.word(), occasion.key());
            }
        }
        for word in ["", "Lock", "lock ", "reboot", "power-off", "unlock", "login", "start-locker"] {
            assert_eq!(Departure::parse(word), None, "{word:?}");
        }
    }

    #[test]
    fn suspend_then_hibernate_is_a_suspend_that_ends_differently() {
        let departure = Departure::parse("suspend-then-hibernate").unwrap();
        assert_eq!(departure.occasion(), Occasion::Suspend);
        assert_eq!(departure.action(), Action::SuspendThenHibernate);
        assert_eq!(departure.word(), "suspend-then-hibernate");
        assert_eq!(Action::SuspendThenHibernate.key(), "suspend-then-hibernate");
        assert_ne!(Some(departure), Departure::of(Occasion::Suspend));
    }

    #[test]
    fn a_farewell_names_the_closings_that_end_the_session_or_the_machine() {
        let table = [
            ("logout", Some(Occasion::Logout)),
            ("suspend", Some(Occasion::Suspend)),
            ("hibernate", Some(Occasion::Hibernate)),
            ("restart", Some(Occasion::Restart)),
            ("shutdown", Some(Occasion::Shutdown)),
            ("lock", None),
            ("suspend-then-hibernate", None),
            ("unlock", None),
            ("login", None),
            ("", None),
            ("Shutdown", None),
        ];
        for (word, occasion) in table {
            assert_eq!(Departure::parse_farewell(word), occasion, "{word:?}");
        }
    }

    #[test]
    fn only_suspend_and_hibernate_sleep() {
        let sleeping: Vec<_> = Occasion::ALL.into_iter().filter(|o| o.sleeps()).collect();
        assert_eq!(sleeping, [Occasion::Suspend, Occasion::Hibernate]);
    }

    #[test]
    fn exactly_the_closing_occasions_have_an_action() {
        for occasion in Occasion::ALL {
            assert_eq!(
                occasion.action().is_some(),
                occasion.direction() == Direction::Closing,
                "{occasion:?}"
            );
        }
    }

    #[test]
    fn recording_runner_keeps_order_in_memory_and_in_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("actions");
        let mut runner = RecordingRunner::to_file(&path);
        runner.run(Action::StartLocker).unwrap();
        runner.run(Action::Logout).unwrap();
        assert_eq!(runner.actions, [Action::StartLocker, Action::Logout]);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "start-locker\nlogout\n"
        );
    }

    #[test]
    fn recording_runner_reports_an_unwritable_file_and_records_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let mut runner = RecordingRunner::to_file(directory.path().join("missing/actions"));
        assert!(runner.run(Action::Shutdown).is_err());
        assert!(runner.actions.is_empty());
    }
}
