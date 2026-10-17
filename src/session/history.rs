//! The last line shown per occasion, so selection never repeats it.
//!
//! One small file per occasion under `$XDG_STATE_HOME/dojo/session/`. Parsing and
//! formatting are pure; `load` and `store` are the thin file boundary.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use super::Occasion;

pub fn parse(contents: &str) -> Option<String> {
    let line = contents.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

pub fn format(line: &str) -> String {
    let flat: Vec<&str> = line.split_whitespace().collect();
    format!("{}\n", flat.join(" "))
}

/// `$XDG_STATE_HOME/dojo/session`, else `~/.local/state/dojo/session`.
/// A relative `XDG_STATE_HOME` is ignored, as the XDG specification requires.
pub fn directory_from(xdg_state_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let state = xdg_state_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/state"))
        })?;
    Some(state.join("dojo/session"))
}

pub fn default_directory() -> Option<PathBuf> {
    directory_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

/// A missing or unreadable file means no previous line.
pub fn load(directory: &Path, occasion: Occasion) -> Option<String> {
    parse(&std::fs::read_to_string(directory.join(occasion.key())).ok()?)
}

/// Writes through a temporary file in the same directory and renames it into place.
pub fn store(directory: &Path, occasion: Occasion, line: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(format(line).as_bytes())?;
    file.persist(directory.join(occasion.key()))
        .map(drop)
        .map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_takes_the_first_non_empty_line_trimmed() {
        assert_eq!(parse("Hajime, Nathan\n"), Some("Hajime, Nathan".into()));
        assert_eq!(parse("  Onwards \nignored"), Some("Onwards".into()));
        assert_eq!(parse(""), None);
        assert_eq!(parse("\n\n"), None);
        assert_eq!(parse("   \n"), None);
    }

    #[test]
    fn format_writes_one_flat_line() {
        assert_eq!(format("Hajime, Nathan"), "Hajime, Nathan\n");
        assert_eq!(format("a\nb\t c"), "a b c\n");
    }

    #[test]
    fn directory_prefers_an_absolute_xdg_state_home() {
        let dir = |xdg: Option<&str>, home: Option<&str>| {
            directory_from(xdg.map(Into::into), home.map(Into::into))
        };
        assert_eq!(
            dir(Some("/state"), Some("/home/n")),
            Some("/state/dojo/session".into())
        );
        assert_eq!(
            dir(None, Some("/home/n")),
            Some("/home/n/.local/state/dojo/session".into())
        );
        assert_eq!(
            dir(Some("relative"), Some("/home/n")),
            Some("/home/n/.local/state/dojo/session".into())
        );
        assert_eq!(dir(Some(""), Some("/home/n")), dir(None, Some("/home/n")));
        assert_eq!(dir(None, None), None);
    }

    #[test]
    fn lines_round_trip_per_occasion_and_survive_a_missing_directory() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("nested/dojo/session");
        assert_eq!(load(&directory, Occasion::Unlock), None);

        store(&directory, Occasion::Unlock, "Welcome back, Nathan").unwrap();
        store(&directory, Occasion::Lock, "Perimeter secure").unwrap();
        assert_eq!(
            load(&directory, Occasion::Unlock),
            Some("Welcome back, Nathan".into())
        );
        assert_eq!(
            load(&directory, Occasion::Lock),
            Some("Perimeter secure".into())
        );
        assert_eq!(load(&directory, Occasion::Login), None);

        store(&directory, Occasion::Unlock, "Hajime, Nathan").unwrap();
        assert_eq!(
            load(&directory, Occasion::Unlock),
            Some("Hajime, Nathan".into())
        );
        let mut names: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, ["lock", "unlock"]);
    }

    #[test]
    fn an_unreadable_path_means_no_previous_line() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("unlock")).unwrap();
        assert_eq!(load(root.path(), Occasion::Unlock), None);
    }
}
