//! Launcher commands: built-in entries that open a view, and which of them the user hid from the
//! empty launcher.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CommandId {
    Emoji,
    ClipboardHistory,
    Windows,
    Snippets,
    PickColor,
    Processes,
    Services,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: CommandId,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub keywords: &'static [&'static str],
    pub glyph: &'static str,
}

/// Every command Dojo defines, in the order a fresh install lists them. Whether a command is
/// available is a separate question: see [`available`].
pub const ALL: [Command; 7] = [
    Command {
        id: CommandId::Emoji,
        title: "Search Emoji",
        subtitle: "Find an emoji and copy it",
        keywords: &["emoji", "emoticon", "smiley", "symbol"],
        glyph: "😀",
    },
    Command {
        id: CommandId::ClipboardHistory,
        title: "Clipboard History",
        subtitle: "Search what you copied",
        keywords: &["clipboard", "history", "paste", "copied"],
        glyph: "📋",
    },
    Command {
        id: CommandId::Windows,
        title: "Switch Windows",
        subtitle: "Jump to an open window",
        keywords: &["windows", "switch", "focus", "alt tab"],
        glyph: "🪟",
    },
    Command {
        id: CommandId::Snippets,
        title: "Snippets",
        subtitle: "Copy saved text",
        keywords: &["snippets", "text", "template", "boilerplate"],
        glyph: "✂️",
    },
    Command {
        id: CommandId::PickColor,
        title: "Pick Color",
        subtitle: "Pick a color from the screen",
        keywords: &["color", "colour", "picker", "eyedropper", "hex"],
        glyph: "🎨",
    },
    Command {
        id: CommandId::Processes,
        title: "Processes",
        subtitle: "See what is running and stop it",
        keywords: &["processes", "kill", "task manager", "cpu", "memory"],
        glyph: "📈",
    },
    Command {
        id: CommandId::Services,
        title: "Services",
        subtitle: "Manage user services",
        keywords: &["services", "systemd", "units", "restart", "daemon"],
        glyph: "⚙️",
    },
];

/// The commands whose views exist. A command joins this list when its view is wired in; until
/// then it is neither suggested nor found by searching.
pub const IMPLEMENTED: &[CommandId] = &[
    CommandId::Emoji,
    CommandId::ClipboardHistory,
    CommandId::Windows,
    CommandId::Snippets,
    CommandId::PickColor,
    CommandId::Processes,
    CommandId::Services,
];

pub fn available() -> Vec<Command> {
    ALL.iter()
        .copied()
        .filter(|command| IMPLEMENTED.contains(&command.id))
        .collect()
}

impl CommandId {
    /// The name used on the command line (`dojo --launcher-view emoji`) and in saved state.
    pub fn key(self) -> &'static str {
        match self {
            Self::Emoji => "emoji",
            Self::ClipboardHistory => "clipboard",
            Self::Windows => "windows",
            Self::Snippets => "snippets",
            Self::PickColor => "color",
            Self::Processes => "processes",
            Self::Services => "services",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        ALL.iter()
            .map(|command| command.id)
            .find(|id| id.key() == key)
    }

    /// Key under which launches of the command are remembered in the launch history.
    pub fn history_id(self) -> String {
        format!("command:{}", self.key())
    }

    pub fn command(self) -> &'static Command {
        ALL.iter()
            .find(|command| command.id == self)
            .expect("every command id is listed in ALL")
    }
}

/// Commands the user removed from the empty launcher. They are still found by typing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HiddenCommands {
    keys: BTreeSet<String>,
}

impl HiddenCommands {
    pub fn contains(&self, id: CommandId) -> bool {
        self.keys.contains(id.key())
    }

    /// Flips the command and says whether it is hidden afterwards.
    pub fn toggle(&mut self, id: CommandId) -> bool {
        if !self.keys.remove(id.key()) {
            self.keys.insert(id.key().to_owned());
            return true;
        }
        false
    }

    /// A missing or damaged file hides nothing.
    pub fn load(path: &Path) -> Self {
        let keys = fs::read_to_string(path)
            .ok()
            .and_then(|contents| serde_json::from_str::<BTreeSet<String>>(&contents).ok())
            .unwrap_or_default();
        Self {
            keys: keys
                .into_iter()
                .filter(|key| CommandId::from_key(key).is_some())
                .collect(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("hidden commands path has no parent directory")?;
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
        let mut file = fs::File::create(&temporary)
            .with_context(|| format!("creating {}", temporary.display()))?;
        file.write_all(&serde_json::to_vec(&self.keys)?)
            .and_then(|()| file.sync_all())
            .with_context(|| format!("writing {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("replacing {} with {}", path.display(), temporary.display()))
    }
}

/// `$XDG_STATE_HOME/dojo/launcher-hidden-commands.json`.
pub fn hidden_path() -> Result<PathBuf> {
    let state_home = match std::env::var_os("XDG_STATE_HOME") {
        Some(path) => PathBuf::from(path),
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("neither XDG_STATE_HOME nor HOME is set")?
            .join(".local/state"),
    };
    if !state_home.is_absolute() {
        bail!("XDG_STATE_HOME must be absolute");
    }
    Ok(state_home.join("dojo/launcher-hidden-commands.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_round_trip() {
        let mut seen = BTreeSet::new();
        for command in ALL {
            assert!(
                seen.insert(command.id.key()),
                "duplicate {}",
                command.id.key()
            );
            assert_eq!(CommandId::from_key(command.id.key()), Some(command.id));
            assert_eq!(command.id.command(), &command);
            assert!(!command.title.is_empty() && !command.keywords.is_empty());
        }
        assert_eq!(CommandId::from_key("nonsense"), None);
        assert_eq!(CommandId::Emoji.history_id(), "command:emoji");
    }

    #[test]
    fn only_implemented_commands_are_available() {
        let ids = available().into_iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids, IMPLEMENTED);
    }

    #[test]
    fn hiding_toggles_and_persists_and_ignores_damage() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("dojo/hidden.json");
        assert_eq!(HiddenCommands::load(&path), HiddenCommands::default());

        let mut hidden = HiddenCommands::default();
        assert!(hidden.toggle(CommandId::Windows));
        assert!(hidden.contains(CommandId::Windows));
        hidden.save(&path)?;
        assert_eq!(HiddenCommands::load(&path), hidden);

        assert!(!hidden.toggle(CommandId::Windows));
        assert!(!hidden.contains(CommandId::Windows));

        fs::write(&path, r#"["windows", "ghost"]"#)?;
        let loaded = HiddenCommands::load(&path);
        assert!(loaded.contains(CommandId::Windows));
        assert_eq!(loaded.keys.len(), 1, "unknown keys are dropped");

        fs::write(&path, "not json")?;
        assert_eq!(HiddenCommands::load(&path), HiddenCommands::default());
        Ok(())
    }
}
