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
    /// The closing occasion named `word`, as `dojo --leave` spells it. Opening occasions have no closing.
    pub fn parse_closing(word: &str) -> Option<Occasion> {
        Occasion::ALL
            .into_iter()
            .find(|occasion| occasion.key() == word && occasion.action().is_some())
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
            let parsed = Occasion::parse_closing(occasion.key());
            assert_eq!(parsed, occasion.action().map(|_| occasion), "{occasion:?}");
        }
        for word in ["", "Lock", "lock ", "reboot", "power-off", "unlock"] {
            assert_eq!(Occasion::parse_closing(word), None, "{word:?}");
        }
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
