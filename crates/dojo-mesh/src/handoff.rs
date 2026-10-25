//! What a sent window carries, and how the receiving node opens it.
//!
//! A handoff describes a thing to open (a page, a directory, a file at a line, a desktop entry),
//! never a command. The receiver turns it into a command itself, from a fixed set, so a peer can
//! ask for a window but cannot make this node run whatever it likes.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Handoff {
    /// A web page, opened in the receiver's browser.
    Page { url: String },
    /// A terminal in a directory, optionally running one known program.
    Terminal { directory: String, program: Option<Program> },
    /// The same application, started fresh from the receiver's own desktop entry.
    Relaunch { desktop_entry: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Program {
    /// nvim on a file, at a line and column (both from 1).
    Editor { file: String, line: u32, column: u32 },
    /// yazi in a directory.
    Files { directory: String },
}

/// What the receiver answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Reply {
    Opened { notes: Vec<String> },
    /// Files the receiver does not have; nothing was opened.
    Missing { files: Vec<String> },
    Refused { reason: String },
}

/// Files the receiver must already have for this handoff to open.
pub fn required_files(handoff: &Handoff) -> Vec<String> {
    match handoff {
        Handoff::Terminal { program: Some(Program::Editor { file, .. }), .. } => vec![file.clone()],
        _ => Vec::new(),
    }
}

/// The command that opens a handoff here, with notes on what differs from the sender's window.
/// `exists` and `has_entry` ask this machine; they are parameters so the rule can be tested.
pub fn plan(handoff: &Handoff, exists: impl Fn(&Path) -> bool, has_entry: impl Fn(&str) -> bool) -> Result<(Vec<String>, Vec<String>), Reply> {
    let refuse = |reason: String| Reply::Refused { reason };
    let mut notes = Vec::new();
    let command = match handoff {
        Handoff::Page { url } => {
            let scheme = url.split_once("://").map(|(scheme, _)| scheme.to_ascii_lowercase());
            if !matches!(scheme.as_deref(), Some("http" | "https")) || url.contains(char::is_control) {
                return Err(refuse(format!("only http and https pages can be sent, not {url:?}")));
            }
            vec!["xdg-open".into(), url.clone()]
        }
        Handoff::Terminal { directory, program } => {
            let missing: Vec<String> = required_files(handoff).into_iter().filter(|file| !absolute(file).is_some_and(|path| exists(&path))).collect();
            if !missing.is_empty() {
                return Err(Reply::Missing { files: missing });
            }
            let wanted = absolute(directory).ok_or_else(|| refuse(format!("{directory:?} is not an absolute path")))?;
            let here = nearest_existing(&wanted, &exists);
            if here != wanted {
                notes.push(format!("{} does not exist here, so the terminal opened in {}", wanted.display(), here.display()));
            }
            let here = here.to_string_lossy().into_owned();
            let mut command = vec!["kitty".to_string(), "--directory".to_string(), here.clone()];
            match program {
                None => {}
                Some(Program::Editor { file, line, column }) => {
                    command.extend(["nvim".to_string(), format!("+call cursor({}, {})", line.max(&1), column.max(&1)), "--".to_string(), file.clone()]);
                }
                Some(Program::Files { directory }) => {
                    let wanted = absolute(directory).ok_or_else(|| refuse(format!("{directory:?} is not an absolute path")))?;
                    let target = nearest_existing(&wanted, &exists);
                    command.extend(["yazi".to_string(), target.to_string_lossy().into_owned()]);
                }
            }
            command
        }
        Handoff::Relaunch { desktop_entry } => {
            if desktop_entry.is_empty() || !desktop_entry.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
                return Err(refuse(format!("{desktop_entry:?} is not a desktop entry name")));
            }
            if !has_entry(desktop_entry) {
                return Err(refuse(format!("this machine has no {desktop_entry} application")));
            }
            notes.push("started fresh: this application cannot carry what it was showing".into());
            vec!["gtk-launch".into(), desktop_entry.clone()]
        }
    };
    Ok((command, notes))
}

fn absolute(path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(path);
    (path.is_absolute() && !path.as_os_str().is_empty() && !path.to_string_lossy().contains('\0')).then_some(path)
}

/// `path` itself if it exists, otherwise its closest existing ancestor (at worst `/`).
pub fn nearest_existing(path: &Path, exists: impl Fn(&Path) -> bool) -> PathBuf {
    path.ancestors().find(|candidate| exists(candidate)).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn world(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |path: &Path| path == Path::new("/") || set.contains(path)
    }

    fn no_entries(_: &str) -> bool {
        false
    }

    #[test]
    fn handoffs_travel_as_tagged_json() {
        let handoff = Handoff::Terminal { directory: "/w".into(), program: Some(Program::Editor { file: "/w/a.rs".into(), line: 3, column: 7 }) };
        let text = serde_json::to_string(&handoff).unwrap();
        assert_eq!(text, r#"{"kind":"terminal","directory":"/w","program":{"kind":"editor","file":"/w/a.rs","line":3,"column":7}}"#);
        assert_eq!(serde_json::from_str::<Handoff>(&text).unwrap(), handoff);
        assert_eq!(serde_json::to_string(&Reply::Missing { files: vec!["/x".into()] }).unwrap(), r#"{"status":"missing","files":["/x"]}"#);
    }

    #[test]
    fn a_page_opens_in_the_browser_and_only_web_pages_are_allowed() {
        let (command, notes) = plan(&Handoff::Page { url: "https://example.com/a?b".into() }, world(&[]), no_entries).unwrap();
        assert_eq!(command, ["xdg-open", "https://example.com/a?b"]);
        assert!(notes.is_empty());
        for url in ["file:///etc/passwd", "javascript:alert(1)", "example.com", "https://a\nb"] {
            assert!(matches!(plan(&Handoff::Page { url: url.into() }, world(&[]), no_entries), Err(Reply::Refused { .. })), "{url}");
        }
    }

    #[test]
    fn a_terminal_opens_in_the_directory_or_the_nearest_one_that_exists() {
        let (command, notes) = plan(&Handoff::Terminal { directory: "/home/a/project".into(), program: None }, world(&["/home/a/project"]), no_entries).unwrap();
        assert_eq!(command, ["kitty", "--directory", "/home/a/project"]);
        assert!(notes.is_empty());
        let (command, notes) = plan(&Handoff::Terminal { directory: "/home/a/project/deep".into(), program: None }, world(&["/home/a"]), no_entries).unwrap();
        assert_eq!(command, ["kitty", "--directory", "/home/a"]);
        assert_eq!(notes, ["/home/a/project/deep does not exist here, so the terminal opened in /home/a"]);
    }

    #[test]
    fn an_editor_needs_its_file_and_opens_at_the_line() {
        let handoff = Handoff::Terminal { directory: "/w".into(), program: Some(Program::Editor { file: "/w/a.rs".into(), line: 12, column: 4 }) };
        let (command, _) = plan(&handoff, world(&["/w", "/w/a.rs"]), no_entries).unwrap();
        assert_eq!(command, ["kitty", "--directory", "/w", "nvim", "+call cursor(12, 4)", "--", "/w/a.rs"]);
        assert_eq!(plan(&handoff, world(&["/w"]), no_entries).unwrap_err(), Reply::Missing { files: vec!["/w/a.rs".into()] });
    }

    #[test]
    fn yazi_opens_in_its_directory() {
        let handoff = Handoff::Terminal { directory: "/w".into(), program: Some(Program::Files { directory: "/w/sub".into() }) };
        let (command, _) = plan(&handoff, world(&["/w", "/w/sub"]), no_entries).unwrap();
        assert_eq!(command, ["kitty", "--directory", "/w", "yazi", "/w/sub"]);
    }

    #[test]
    fn a_relaunch_needs_the_same_application_here() {
        let handoff = Handoff::Relaunch { desktop_entry: "org.example.App".into() };
        let (command, notes) = plan(&handoff, world(&[]), |entry| entry == "org.example.App").unwrap();
        assert_eq!(command, ["gtk-launch", "org.example.App"]);
        assert_eq!(notes.len(), 1);
        assert!(matches!(plan(&handoff, world(&[]), no_entries), Err(Reply::Refused { .. })));
        for bad in ["", "../evil", "a b", "a/b"] {
            assert!(matches!(plan(&Handoff::Relaunch { desktop_entry: bad.into() }, world(&[]), |_| true), Err(Reply::Refused { .. })), "{bad}");
        }
    }

    #[test]
    fn relative_paths_are_refused() {
        assert!(matches!(plan(&Handoff::Terminal { directory: "relative".into(), program: None }, world(&[]), no_entries), Err(Reply::Refused { .. })));
        let editor = Handoff::Terminal { directory: "/w".into(), program: Some(Program::Editor { file: "a.rs".into(), line: 1, column: 1 }) };
        assert!(matches!(plan(&editor, world(&["/w"]), no_entries), Err(Reply::Missing { .. })), "a relative file is never found");
    }
}
