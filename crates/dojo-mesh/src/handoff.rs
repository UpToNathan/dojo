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
    /// A terminal in a directory on the receiver, optionally running one known program. What an
    /// older sender sends; terminals are now `RemoteTerminal`.
    Terminal { directory: String, program: Option<Program> },
    /// A terminal that is a session on `host`, in a directory there, optionally running one known
    /// program there. The receiver opens it connected to `host` over ssh; when `host` is the
    /// receiver itself, that is an ordinary local terminal, which is how a session is sent back to
    /// the machine it lives on. Without `host`, the sender is the host, and the receiver learns who
    /// the sender is from the connection, never from this message.
    RemoteTerminal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
        directory: String,
        program: Option<Program>,
    },
    /// All the tabs of a browser window, opened together in a new window of the receiver's own
    /// browser (a Chromium-family one, found by its desktop entry).
    BrowserWindow { desktop_entry: String, urls: Vec<String> },
    /// The same application, started fresh from the receiver's own desktop entry.
    Relaunch { desktop_entry: String },
    /// A T3 Code thread, opened in the receiver's own T3 Code. The thread is not moved: T3 Connect
    /// keeps it in step between the machines, and T3 opens it from its own `t3code://` link.
    T3Thread { environment: String, thread: String },
}

/// How much of a browser window is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// The tab in front.
    Tab,
    /// Every tab of the window.
    Window,
}

impl Scope {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "tab" => Some(Self::Tab),
            "window" => Some(Self::Window),
            _ => None,
        }
    }
}

/// Browsers whose `--new-window` is known, by desktop entry id.
const NEW_WINDOW_BROWSERS: &[&str] = &["helium", "chromium", "google-chrome", "brave-browser", "microsoft-edge", "vivaldi-stable"];

/// The most tabs sent in one window.
pub const MAX_TABS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Program {
    /// nvim on a file, at a line and column (both from 1).
    Editor { file: String, line: u32, column: u32 },
    /// yazi in a directory.
    Files { directory: String },
}

/// What closes on the sending machine once the receiver has opened the handoff. A browser window
/// holds many tabs and only one was sent, so only that tab closes; anything else closes whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "kebab-case")]
pub enum Closes {
    Window,
    /// The tab in front, which gives the window its title.
    Tab { title: String },
    /// A browser window, but only while it still holds exactly the tabs that were sent: a tab
    /// opened since must not be lost with it.
    BrowserWindow { title: String, browser: String, urls: Vec<String> },
    /// Nothing closes; the original stays, and `why` says why.
    Nothing { why: String },
}

/// Where a remote terminal's session lives, as the sender's summary says it.
fn session_of(host: &Option<String>) -> String {
    match host {
        Some(host) => format!("connected to {host}"),
        None => "connected back here".into(),
    }
}

/// One line saying exactly what is being sent and what closes here.
pub fn summary(handoff: &Handoff, closes: &Closes) -> String {
    let what = match handoff {
        Handoff::Page { url } => format!("the page {url}"),
        Handoff::Terminal { directory, program: None } => format!("a terminal in {directory}"),
        Handoff::Terminal { program: Some(Program::Editor { file, line, .. }), .. } => format!("nvim on {file} at line {line}"),
        Handoff::Terminal { directory, program: Some(Program::Files { directory: shown }) } => format!("yazi in {shown} (terminal in {directory})"),
        Handoff::RemoteTerminal { host, directory, program: None } => format!("a terminal in {directory} {}", session_of(host)),
        Handoff::RemoteTerminal { host, program: Some(Program::Editor { file, line, .. }), .. } => format!("nvim on {file} at line {line}, {}", session_of(host)),
        Handoff::RemoteTerminal { host, directory, program: Some(Program::Files { directory: shown }) } => format!("yazi in {shown} (terminal in {directory}), {}", session_of(host)),
        Handoff::BrowserWindow { urls, .. } => format!("a browser window with {} tab{}", urls.len(), if urls.len() == 1 { "" } else { "s" }),
        Handoff::Relaunch { desktop_entry } => format!("a fresh {desktop_entry}"),
        Handoff::T3Thread { thread, .. } => format!("the T3 Code thread {thread}"),
    };
    match closes {
        Closes::Window => format!("{what}; the window closes here"),
        Closes::Tab { .. } => format!("{what}; only this tab closes here, the window's other tabs stay"),
        Closes::BrowserWindow { .. } => format!("{what}; the window closes here if its tabs are unchanged"),
        Closes::Nothing { why } => format!("{what}; the original stays ({why})"),
    }
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
    plan_with(handoff, exists, has_entry, |_| None)
}

/// Like [`plan`], with `exec_of` giving a desktop entry's `Exec` line (a browser window is opened
/// by the browser's own launcher).
pub fn plan_with(
    handoff: &Handoff,
    exists: impl Fn(&Path) -> bool,
    has_entry: impl Fn(&str) -> bool,
    exec_of: impl Fn(&str) -> Option<String>,
) -> Result<(Vec<String>, Vec<String>), Reply> {
    let refuse = |reason: String| Reply::Refused { reason };
    let mut notes = Vec::new();
    let command = match handoff {
        Handoff::Page { url } => {
            if !is_web_url(url) {
                return Err(refuse(format!("only http and https pages can be sent, not {url:?}")));
            }
            vec!["xdg-open".into(), url.clone()]
        }
        Handoff::T3Thread { environment, thread } => {
            let safe = |id: &str| !id.is_empty() && id.len() <= 200 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'));
            if !safe(environment) || !safe(thread) {
                return Err(refuse("that is not a T3 Code environment and thread id".into()));
            }
            if !has_entry("com.t3tools.T3Code") && !has_entry("t3code") {
                return Err(refuse("this machine has no T3 Code".into()));
            }
            notes.push("T3 Code opens the thread from its link; it must know that environment (T3 Connect) to show it".into());
            // Processes started by T3 carry ELECTRON_RUN_AS_NODE, which would turn its own launcher into plain Node.
            vec!["env".into(), "-u".into(), "ELECTRON_RUN_AS_NODE".into(), "xdg-open".into(), format!("t3code://threads/{environment}/{thread}")]
        }
        Handoff::RemoteTerminal { .. } => return Err(refuse("a terminal session can only be opened knowing which machine it lives on".into())),
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
        Handoff::BrowserWindow { desktop_entry, urls } => {
            if !NEW_WINDOW_BROWSERS.contains(&desktop_entry.as_str()) {
                return Err(refuse(format!("{desktop_entry:?} is not a browser whose new window can be opened this way")));
            }
            if urls.is_empty() || urls.len() > MAX_TABS {
                return Err(refuse(format!("a window of {} tabs cannot be sent (1 to {MAX_TABS})", urls.len())));
            }
            if let Some(bad) = urls.iter().find(|url| !is_web_url(url)) {
                return Err(refuse(format!("only http and https pages can be sent, not {bad:?}")));
            }
            let exec = exec_of(desktop_entry).ok_or_else(|| refuse(format!("this machine has no {desktop_entry} browser")))?;
            let mut command = exec_command(&exec).ok_or_else(|| refuse(format!("{desktop_entry}'s launcher could not be read")))?;
            command.push("--new-window".into());
            command.extend(urls.iter().cloned());
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

/// The command that opens a `RemoteTerminal` here, given `home`, the machine the session lives on
/// (the handoff's `host`, else the sender). When `home` is this machine it is an ordinary terminal
/// in the directory; otherwise it is a new kitty running `dojo-mesh shell` to `home`, and `mesh` is
/// this installation's own `dojo-mesh`. `exists` and `has_entry` ask this machine.
pub fn plan_session(
    directory: &str,
    program: &Option<Program>,
    home: &str,
    here: bool,
    mesh: &Path,
    exists: impl Fn(&Path) -> bool,
    has_entry: impl Fn(&str) -> bool,
) -> Result<(Vec<String>, Vec<String>), Reply> {
    let refuse = |reason: String| Reply::Refused { reason };
    if here {
        let (command, mut notes) = plan(&Handoff::Terminal { directory: directory.to_string(), program: program.clone() }, exists, has_entry)?;
        notes.push("this machine is where the session lives, so it is an ordinary terminal here".into());
        return Ok((command, notes));
    }
    let valid_name = !home.is_empty() && home.len() <= 63 && home.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid_name {
        return Err(refuse(format!("{home:?} is not the name of a machine")));
    }
    if absolute(directory).is_none() {
        return Err(refuse(format!("{directory:?} is not an absolute path")));
    }
    let mut command = vec!["kitty".to_string(), "--title".to_string(), format!("{home}: {directory}"), mesh.to_string_lossy().into_owned(), "shell".to_string(), home.to_string(), "--directory".to_string(), directory.to_string()];
    if let Some(program) = program {
        let file_ok = match program {
            Program::Editor { file, .. } => absolute(file).is_some(),
            Program::Files { directory } => absolute(directory).is_some(),
        };
        if !file_ok {
            return Err(refuse("the program's path is not absolute".into()));
        }
        command.push("--program".into());
        command.push(serde_json::to_string(program).map_err(|error| refuse(error.to_string()))?);
    }
    Ok((command, vec![format!("connected to {home} over ssh; the session lives there")]))
}

/// What the sender runs on itself for a program: the argument list `dojo-mesh shell` starts there.
pub fn program_argv(program: &Program) -> Vec<String> {
    match program {
        Program::Editor { file, line, column } => vec!["nvim".into(), format!("+call cursor({}, {})", line.max(&1), column.max(&1)), "--".into(), file.clone()],
        Program::Files { directory } => vec!["yazi".into(), directory.clone()],
    }
}

/// An http or https address with no control characters or spaces.
pub fn is_web_url(url: &str) -> bool {
    let scheme = url.split_once("://").map(|(scheme, _)| scheme.to_ascii_lowercase());
    matches!(scheme.as_deref(), Some("http" | "https")) && !url.contains(|c: char| c.is_control() || c == ' ')
}

/// The program and arguments of a desktop entry's `Exec` line, without its field codes (`%U` and
/// the like, which the caller supplies itself). `None` if it is empty or its quoting is broken.
pub fn exec_command(exec: &str) -> Option<Vec<String>> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut chars = exec.trim().chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => current.push(chars.next()?),
            c if c.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if started {
        words.push(current);
    }
    // Field codes stand for what the caller passes; `%%` is a literal percent sign.
    let words: Vec<String> = words.into_iter().filter(|word| !(word.len() == 2 && word.starts_with('%') && word != "%%")).map(|word| word.replace("%%", "%")).collect();
    (!words.is_empty()).then_some(words)
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
    fn the_summary_says_what_is_sent_and_what_closes() {
        let page = Handoff::Page { url: "https://example.com/a".into() };
        assert_eq!(summary(&page, &Closes::Tab { title: "A".into() }), "the page https://example.com/a; only this tab closes here, the window's other tabs stay");
        let editor = Handoff::Terminal { directory: "/w".into(), program: Some(Program::Editor { file: "/w/a.rs".into(), line: 9, column: 1 }) };
        assert_eq!(summary(&editor, &Closes::Window), "nvim on /w/a.rs at line 9; the window closes here");
        assert_eq!(summary(&Handoff::Terminal { directory: "/w".into(), program: None }, &Closes::Window), "a terminal in /w; the window closes here");
        assert_eq!(summary(&Handoff::Relaunch { desktop_entry: "app".into() }, &Closes::Window), "a fresh app; the window closes here");
        assert_eq!(serde_json::to_string(&Closes::Tab { title: "T".into() }).unwrap(), r#"{"scope":"tab","title":"T"}"#);
    }

    #[test]
    fn a_browser_window_opens_all_its_tabs_together_in_a_new_window_of_the_browser() {
        let handoff = Handoff::BrowserWindow { desktop_entry: "helium".into(), urls: vec!["https://a.example/".into(), "https://b.example/x?y=1".into()] };
        let (command, _) = plan_with(&handoff, world(&[]), |_| true, |entry| (entry == "helium").then(|| "env X=1 helium --flag %U".to_string())).unwrap();
        assert_eq!(command, ["env", "X=1", "helium", "--flag", "--new-window", "https://a.example/", "https://b.example/x?y=1"]);
        assert_eq!(summary(&handoff, &Closes::BrowserWindow { title: "T".into(), browser: "Helium".into(), urls: vec![] }), "a browser window with 2 tabs; the window closes here if its tabs are unchanged");
    }

    #[test]
    fn a_browser_window_is_refused_unless_it_is_a_known_browser_with_only_web_tabs() {
        let exec = |_: &str| Some("helium %U".to_string());
        let window = |entry: &str, urls: Vec<&str>| Handoff::BrowserWindow { desktop_entry: entry.into(), urls: urls.into_iter().map(String::from).collect() };
        for handoff in [
            window("kitty", vec!["https://a.example/"]),
            window("helium", vec![]),
            window("helium", vec!["file:///etc/passwd"]),
            window("helium", vec!["https://a.example/", "chrome://settings"]),
            window("helium", vec!["https://a.example/ b"]),
        ] {
            assert!(matches!(plan_with(&handoff, world(&[]), |_| true, exec), Err(Reply::Refused { .. })), "{handoff:?}");
        }
        let many = Handoff::BrowserWindow { desktop_entry: "helium".into(), urls: vec!["https://a.example/".into(); MAX_TABS + 1] };
        assert!(matches!(plan_with(&many, world(&[]), |_| true, exec), Err(Reply::Refused { .. })));
        let ok = window("helium", vec!["https://a.example/"]);
        assert!(matches!(plan_with(&ok, world(&[]), |_| true, |_| None), Err(Reply::Refused { .. })), "no such browser here");
    }

    #[test]
    fn exec_lines_lose_their_field_codes_and_keep_their_quoting() {
        assert_eq!(exec_command("helium %U").unwrap(), ["helium"]);
        assert_eq!(exec_command("\"/opt/My App/run\" --x=\"a b\" %u %f 100%%").unwrap(), ["/opt/My App/run", "--x=a b", "100%"]);
        assert_eq!(exec_command("  a   b  ").unwrap(), ["a", "b"]);
        assert_eq!(exec_command("%U"), None);
        assert_eq!(exec_command("\"unclosed"), None);
        assert_eq!(exec_command(""), None);
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
    fn a_t3_thread_opens_through_its_link_with_the_inherited_node_mode_cleared() {
        let handoff = Handoff::T3Thread { environment: "env-1".into(), thread: "thread:provider:codex:1".into() };
        let (command, notes) = plan(&handoff, |_| false, |entry| entry == "com.t3tools.T3Code").unwrap();
        assert_eq!(command, ["env", "-u", "ELECTRON_RUN_AS_NODE", "xdg-open", "t3code://threads/env-1/thread:provider:codex:1"]);
        assert!(!notes.is_empty());
    }

    #[test]
    fn a_t3_thread_is_refused_without_t3_or_with_an_id_that_could_change_the_link() {
        let ok = Handoff::T3Thread { environment: "env-1".into(), thread: "t-1".into() };
        assert!(matches!(plan(&ok, |_| false, |_| false), Err(Reply::Refused { reason }) if reason.contains("no T3 Code")));
        for (environment, thread) in [("env/../x", "t-1"), ("env-1", "t 1"), ("env-1", "t?a=b"), ("", "t-1"), ("env-1", "a/b")] {
            let bad = Handoff::T3Thread { environment: environment.into(), thread: thread.into() };
            assert!(matches!(plan(&bad, |_| false, |_| true), Err(Reply::Refused { .. })), "{environment} {thread}");
        }
    }

    #[test]
    fn a_session_runs_the_shell_command_to_the_machine_it_lives_on() {
        let mesh = Path::new("/opt/dojo/dojo-mesh");
        let (command, notes) = plan_session("/home/a/project", &None, "orchard", false, mesh, |_| true, |_| true).unwrap();
        assert_eq!(command, ["kitty", "--title", "orchard: /home/a/project", "/opt/dojo/dojo-mesh", "shell", "orchard", "--directory", "/home/a/project"]);
        assert!(notes[0].contains("orchard"));
        let program = Some(Program::Editor { file: "/home/a/project/main.rs".into(), line: 3, column: 7 });
        let (command, _) = plan_session("/home/a/project", &program, "orchard", false, mesh, |_| true, |_| true).unwrap();
        assert_eq!(command[8], "--program");
        assert_eq!(serde_json::from_str::<Program>(&command[9]).unwrap(), program.unwrap());
    }

    #[test]
    fn a_session_sent_to_the_machine_it_lives_on_is_an_ordinary_terminal_there() {
        let mesh = Path::new("/opt/dojo/dojo-mesh");
        let (command, notes) = plan_session("/home/a/project", &None, "orchard", true, mesh, |_| true, |_| true).unwrap();
        assert_eq!(command, ["kitty", "--directory", "/home/a/project"]);
        assert!(notes.iter().any(|note| note.contains("where the session lives")));
        let files = Some(Program::Files { directory: "/home/a/pics".into() });
        let (command, _) = plan_session("/home/a", &files, "orchard", true, mesh, |_| true, |_| true).unwrap();
        assert_eq!(command, ["kitty", "--directory", "/home/a", "yazi", "/home/a/pics"]);
        let (command, notes) = plan_session("/gone/dir", &None, "orchard", true, mesh, |path| path == Path::new("/"), |_| true).unwrap();
        assert_eq!(command, ["kitty", "--directory", "/"], "a directory that is gone opens in the nearest one");
        assert!(notes.iter().any(|note| note.contains("does not exist here")));
    }

    #[test]
    fn a_session_refuses_names_and_paths_that_are_not_what_they_claim() {
        let mesh = Path::new("/opt/dojo/dojo-mesh");
        let refused = |directory: &str, program: &Option<Program>, home: &str| matches!(plan_session(directory, program, home, false, mesh, |_| true, |_| true), Err(Reply::Refused { .. }));
        assert!(refused("project", &None, "orchard"), "a relative directory");
        assert!(refused("/home/a", &Some(Program::Files { directory: "pics".into() }), "orchard"), "a relative program path");
        for home in ["", "-o ProxyCommand=x", "a b", "orchard;id", "a/b"] {
            assert!(refused("/home/a", &None, home), "{home:?}");
        }
        // Without the machine's name there is nothing to connect to.
        let handoff = Handoff::RemoteTerminal { host: None, directory: "/home/a".into(), program: None };
        assert!(matches!(plan(&handoff, |_| true, |_| true), Err(Reply::Refused { .. })));
    }

    #[test]
    fn a_program_is_run_by_its_words_on_the_machine_it_is_sent_from() {
        assert_eq!(program_argv(&Program::Editor { file: "/w/a b.rs".into(), line: 0, column: 2 }), ["nvim", "+call cursor(1, 2)", "--", "/w/a b.rs"]);
        assert_eq!(program_argv(&Program::Files { directory: "/w".into() }), ["yazi", "/w"]);
    }

    #[test]
    fn a_remote_terminal_summary_says_which_machine_it_is_connected_to() {
        let back = summary(&Handoff::RemoteTerminal { host: None, directory: "/w".into(), program: None }, &Closes::Window);
        assert!(back.contains("connected back here") && back.contains("the window closes here"), "{back}");
        let named = summary(&Handoff::RemoteTerminal { host: Some("orchard".into()), directory: "/w".into(), program: None }, &Closes::Window);
        assert!(named.contains("connected to orchard"), "{named}");
    }

    #[test]
    fn a_remote_terminal_without_a_host_keeps_the_old_wire_form() {
        let text = serde_json::to_string(&Handoff::RemoteTerminal { host: None, directory: "/w".into(), program: None }).unwrap();
        assert!(!text.contains("host"), "{text}");
        assert_eq!(serde_json::from_str::<Handoff>(&text).unwrap(), Handoff::RemoteTerminal { host: None, directory: "/w".into(), program: None });
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
