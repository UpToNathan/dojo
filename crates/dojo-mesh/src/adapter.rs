//! Handoff adapters: for one window, what it shows, as a [`Handoff`] the receiver can open.
//!
//! Everything an adapter reads from this machine goes through [`Probe`], so the rules are tested
//! without a browser, kitty or nvim running.

use crate::handoff::{Closes, Handoff, MAX_TABS, Program, Scope, is_web_url};
use crate::niri::Window;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What an adapter read from a window.
#[derive(Debug, PartialEq, Eq)]
pub struct Captured {
    pub handoff: Handoff,
    /// What closes here once the peer has opened it.
    pub closes: Closes,
    /// The ways this window could be sent, when there is a choice (a browser window: its front tab
    /// or all of it). Empty when there is only one way.
    pub scopes: Vec<Scope>,
    /// How many tabs the whole window holds, when the browser said.
    pub tab_count: Option<usize>,
    /// What the receiver will not have, said to the operator.
    pub notes: Vec<String>,
}

/// What adapters may ask this machine.
pub trait Probe {
    /// `kitty @ ls` on a kitty's socket, authenticated with the adapter's password, which kitty
    /// wants encrypted with that instance's public key.
    fn kitty_ls(&self, socket: &Path, password: &str, public_key: &str) -> Result<String, String>;
    /// The public key a kitty gives the programs it runs (`KITTY_PUBLIC_KEY` in their environment).
    fn kitty_public_key(&self, pid: i64) -> Option<String>;
    /// A process's children.
    fn children(&self, pid: i64) -> Vec<i64>;
    /// `[file, line, column, modified buffers]` from an nvim server, as JSON.
    fn nvim_state(&self, socket: &Path) -> Result<String, String>;
    /// Recent history of a Chromium-family profile: (url, title), newest first.
    fn history(&self, profile: &Path) -> Result<Vec<(String, String)>, String>;
    /// The Dojo Tabs extension's live report, if it is connected.
    fn tabs(&self, runtime_dir: &Path) -> Option<crate::tabs::Snapshot>;
    fn read(&self, path: &Path) -> Option<String>;
    fn list(&self, dir: &Path) -> Vec<String>;
}

pub struct Context<'a> {
    pub probe: &'a dyn Probe,
    pub entries: &'a [(String, String)],
    pub runtime_dir: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
}

/// The best handoff for a window.
pub fn capture(window: &Window, context: &Context, scope: Scope) -> Result<Captured, String> {
    let app_id = window.app_id.as_deref().unwrap_or_default();
    let specific = match app_id {
        "kitty" => Some(kitty(window, context)),
        _ => browser_profile(app_id, context).map(|profile| browser(window, &profile, context, scope)),
    };
    if scope == Scope::Window && specific.is_none() || scope == Scope::Window && app_id == "kitty" {
        return Err("this window has only one way to be sent".into());
    }
    match specific {
        Some(Ok(captured)) => Ok(captured),
        Some(Err(Fallback::Refuse(reason))) => Err(reason),
        Some(Err(Fallback::Relaunch(why))) => relaunch(app_id, context).map(|mut captured| {
            captured.notes.insert(0, why);
            captured
        }),
        None => relaunch(app_id, context),
    }
}

/// Why a specific adapter gave up: either the window must stay (data would be lost), or the
/// application can still be started fresh.
enum Fallback {
    Refuse(String),
    Relaunch(String),
}

fn relaunch(app_id: &str, context: &Context) -> Result<Captured, String> {
    let entry = crate::desktop::for_app(app_id, context.entries).ok_or_else(|| format!("{} has no desktop entry here, so it cannot be started elsewhere", if app_id.is_empty() { "this window" } else { app_id }))?;
    Ok(Captured { handoff: Handoff::Relaunch { desktop_entry: entry }, closes: Closes::Window, scopes: Vec::new(), tab_count: None, notes: vec!["it is started fresh: what the window was showing does not carry over".into()] })
}

// ---- browsers ---------------------------------------------------------------------------------

/// Chromium-family browsers by window id, with their configuration directory under ~/.config.
const BROWSERS: &[(&str, &str, &str)] = &[("helium", "net.imput.helium", "Helium"), ("chromium", "chromium", "Chromium"), ("google-chrome", "google-chrome", "Google Chrome")];

fn browser_profile(app_id: &str, context: &Context) -> Option<(PathBuf, &'static str)> {
    let (_, dir, name) = BROWSERS.iter().find(|(id, _, _)| id.eq_ignore_ascii_case(app_id))?;
    Some((context.config_dir.as_ref()?.join(dir), name))
}

/// What a browser window shows. With the Dojo Tabs extension connected the browser itself says which
/// window this is and what it holds, so the front tab's address is exact and the whole window can
/// be sent. Without it only the front tab can be sent, found by matching the window's title in the
/// browser's history.
fn browser(window: &Window, (root, name): &(PathBuf, &str), context: &Context, scope: Scope) -> Result<Captured, Fallback> {
    let title = window.title.as_deref().unwrap_or_default();
    let page_title = title.strip_suffix(&format!(" - {name}")).unwrap_or(title);
    if page_title.is_empty() {
        return Err(Fallback::Relaunch("the page could not be identified".into()));
    }
    let reported = context.runtime_dir.as_deref().and_then(|dir| context.probe.tabs(dir));
    let live = reported.as_ref().and_then(|snapshot| crate::tabs::window_for(snapshot, title, name));
    let entry = crate::desktop::for_app(window.app_id.as_deref().unwrap_or_default(), context.entries);
    let can_send_window = live.is_some() && entry.is_some();
    let scopes = if can_send_window { vec![Scope::Tab, Scope::Window] } else { Vec::new() };
    let tab_count = live.map(|window| window.tabs.len());

    if scope == Scope::Window {
        let (Some(live), Some(entry)) = (live, entry) else {
            return Err(Fallback::Refuse("the whole window can be sent only while the Dojo Tabs extension is connected and sees this window; the front tab can still be sent".into()));
        };
        let all: Vec<&crate::tabs::Tab> = live.tabs.iter().collect();
        if all.len() > MAX_TABS {
            return Err(Fallback::Refuse(format!("{} tabs is more than the {MAX_TABS} that can be sent at once", all.len())));
        }
        let urls: Vec<String> = all.iter().filter(|tab| is_web_url(&tab.url)).map(|tab| tab.url.clone()).collect();
        let others = all.len() - urls.len();
        if urls.is_empty() {
            return Err(Fallback::Refuse("none of this window's tabs is a web page that can be sent".into()));
        }
        let all_urls: Vec<String> = all.iter().map(|tab| tab.url.clone()).collect();
        let closes = if others == 0 {
            Closes::BrowserWindow { title: title.to_string(), browser: name.to_string(), urls: all_urls }
        } else {
            Closes::Nothing { why: format!("{others} tab{} cannot be sent: browser pages such as settings", if others == 1 { "" } else { "s" }) }
        };
        let mut notes = vec!["pinned tabs and tab groups do not carry over".to_string()];
        if others > 0 {
            notes.push(format!("{others} tab{} (browser pages) {} not sent", if others == 1 { "" } else { "s" }, if others == 1 { "was" } else { "were" }));
        }
        return Ok(Captured { handoff: Handoff::BrowserWindow { desktop_entry: entry, urls }, closes, scopes, tab_count, notes });
    }

    // The front tab: exact when the browser reported it, otherwise the newest visit with its title.
    let reported_url = live.and_then(|window| window.tabs.iter().find(|tab| tab.active)).map(|tab| tab.url.clone()).filter(|url| is_web_url(url));
    let url = match reported_url {
        Some(url) => Some(url),
        None => {
            let profiles: Vec<PathBuf> = context.probe.list(root).into_iter().filter(|entry| entry == "Default" || entry.starts_with("Profile ")).map(|entry| root.join(entry)).collect();
            profiles.iter().find_map(|profile| {
                let history = context.probe.history(profile).ok()?;
                history.into_iter().find(|(url, visit)| visit == page_title && is_web_url(url)).map(|(url, _)| url)
            })
        }
    };
    match url {
        Some(url) => Ok(Captured { handoff: Handoff::Page { url }, closes: Closes::Tab { title: title.to_string() }, scopes, tab_count, notes: Vec::new() }),
        None => Err(Fallback::Relaunch(format!("the page \"{page_title}\" could not be found in {name}'s history"))),
    }
}

// ---- kitty, nvim and yazi ---------------------------------------------------------------------

#[derive(Deserialize)]
struct OsWindow {
    #[serde(default)]
    is_focused: bool,
    #[serde(default)]
    tabs: Vec<Tab>,
}

#[derive(Deserialize)]
struct Tab {
    #[serde(default)]
    is_active: bool,
    #[serde(default)]
    windows: Vec<KittyWindow>,
}

#[derive(Deserialize)]
struct KittyWindow {
    #[serde(default)]
    is_active: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    foreground_processes: Vec<Process>,
}

#[derive(Deserialize)]
struct Process {
    pid: i64,
    #[serde(default)]
    cmdline: Vec<String>,
    #[serde(default)]
    cwd: String,
}

/// The adapter's kitty password, from the file Home Manager's activation writes.
pub fn kitty_password(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("remote_control_password")?.trim_start();
        let rest = rest.strip_prefix('"')?;
        let (password, after) = rest.split_once('"')?;
        (after.trim() == "ls" && !password.is_empty()).then(|| password.to_string())
    })
}

fn kitty(window: &Window, context: &Context) -> Result<Captured, Fallback> {
    let relaunch = |why: &str| Fallback::Relaunch(why.to_string());
    let runtime = context.runtime_dir.as_ref().ok_or_else(|| relaunch("there is no runtime directory to find kitty in"))?;
    let pid = window.pid.ok_or_else(|| relaunch("Niri gave no process for this window"))?;
    let password = context
        .config_dir
        .as_ref()
        .and_then(|dir| context.probe.read(&dir.join("kitty/dojo-remote-control.conf")))
        .and_then(|text| kitty_password(&text))
        .ok_or_else(|| relaunch("kitty's remote control is not set up for Dojo, so the terminal's directory is unknown"))?;
    let public_key = context.probe.kitty_public_key(pid).ok_or_else(|| relaunch("kitty's public key could not be read, so it cannot be asked safely"))?;
    let listing = context.probe.kitty_ls(&runtime.join(format!("kitty-{pid}.sock")), &password, &public_key).map_err(|error| relaunch(&format!("kitty did not answer: {error}")))?;
    let os_windows: Vec<OsWindow> = serde_json::from_str(&listing).map_err(|error| relaunch(&format!("kitty's answer could not be read: {error}")))?;
    let title = window.title.as_deref().unwrap_or_default();
    let chosen = os_windows
        .iter()
        .find(|os| window.is_focused && os.is_focused)
        .or_else(|| os_windows.iter().find(|os| active(os).is_some_and(|w| w.title == title)))
        .or_else(|| (os_windows.len() == 1).then(|| &os_windows[0]))
        .and_then(active)
        .ok_or_else(|| relaunch("the terminal could not be matched to a kitty window"))?;

    let mut notes = Vec::new();
    let foreground = chosen.foreground_processes.iter().rev().find(|process| !process.cmdline.is_empty());
    let program_name = foreground.and_then(|process| Path::new(&process.cmdline[0]).file_name()).map(|name| name.to_string_lossy().into_owned());
    let program = match (program_name.as_deref(), foreground) {
        (Some("nvim"), Some(process)) => Some(nvim(process, runtime, context)?),
        (Some("yazi"), Some(process)) => Some(Program::Files { directory: yazi_directory(&chosen.title).unwrap_or_else(|| process.cwd.clone()) }),
        (Some(shell), _) if is_shell(shell) => None,
        (Some(other), _) => {
            notes.push(format!("{other} was running there; it does not move, and it ends when the window closes"));
            None
        }
        (None, _) => None,
    };
    if program.is_none() {
        notes.push("the shell starts fresh; its history and anything it was running stay here".into());
    }
    let directory = foreground.map(|process| process.cwd.clone()).filter(|cwd| !cwd.is_empty()).unwrap_or_else(|| chosen.cwd.clone());
    if directory.is_empty() {
        return Err(relaunch("kitty did not say which directory the terminal is in"));
    }
    Ok(Captured { handoff: Handoff::Terminal { directory, program }, closes: Closes::Window, scopes: Vec::new(), tab_count: None, notes })
}

/// The active window of an OS window's active tab.
fn active(os: &OsWindow) -> Option<&KittyWindow> {
    os.tabs.iter().find(|tab| tab.is_active).and_then(|tab| tab.windows.iter().find(|window| window.is_active))
}

fn is_shell(name: &str) -> bool {
    matches!(name, "sh" | "bash" | "zsh" | "fish" | "rush" | "nu" | "dash" | "ksh")
}

fn yazi_directory(title: &str) -> Option<String> {
    let path = title.strip_prefix("Yazi: ")?.trim();
    path.starts_with('/').then(|| path.to_string())
}

fn nvim(process: &Process, runtime: &Path, context: &Context) -> Result<Program, Fallback> {
    // The nvim in the terminal is its interface; the editor itself is usually a child (`nvim
    // --embed`), and the server socket is named after whichever process runs the editor.
    let candidates = std::iter::once(process.pid).chain(context.probe.children(process.pid));
    let mut last_error = String::from("no nvim server was found");
    let mut answer = None;
    for pid in candidates {
        match context.probe.nvim_state(&runtime.join(format!("nvim.{pid}.0"))) {
            Ok(state) => {
                answer = Some(state);
                break;
            }
            Err(error) => last_error = error,
        }
    }
    let state = answer.ok_or_else(|| Fallback::Relaunch(format!("nvim did not answer ({last_error}), so the file is unknown")))?;
    let (file, line, column, modified): (String, u32, u32, u32) = serde_json::from_str(&state).map_err(|error| Fallback::Relaunch(format!("nvim's answer could not be read: {error}")))?;
    if modified > 0 {
        return Err(Fallback::Refuse(format!("nvim has {modified} unsaved buffer{}; save first, or they would be lost when the window closes", if modified == 1 { "" } else { "s" })));
    }
    if file.is_empty() {
        return Err(Fallback::Relaunch("nvim has no file open".into()));
    }
    Ok(Program::Editor { file, line, column })
}

// ---- the real machine -------------------------------------------------------------------------

pub struct System;

fn run(program: &str, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().map_err(|error| format!("{program}: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().chars().take(300).collect());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

impl Probe for System {
    fn kitty_ls(&self, socket: &Path, password: &str, public_key: &str) -> Result<String, String> {
        // The password travels in the environment, never on the command line.
        run(
            "kitty",
            &["@", "--to", &format!("unix:{}", socket.display()), "--password-env", "DOJO_KITTY_RC_PASSWORD", "ls"],
            &[("DOJO_KITTY_RC_PASSWORD", password), ("KITTY_PUBLIC_KEY", public_key)],
        )
    }

    fn children(&self, pid: i64) -> Vec<i64> {
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else { return Vec::new() };
        tasks.flatten().filter_map(|task| std::fs::read_to_string(task.path().join("children")).ok()).flat_map(|text| text.split_whitespace().filter_map(|pid| pid.parse().ok()).collect::<Vec<i64>>()).collect()
    }

    fn kitty_public_key(&self, pid: i64) -> Option<String> {
        // kitty puts the key in the environment of each program it starts; any of its children has it.
        self.children(pid).iter().find_map(|child| {
            let environ = std::fs::read(format!("/proc/{child}/environ")).ok()?;
            environ.split(|byte| *byte == 0).find_map(|entry| std::str::from_utf8(entry).ok()?.strip_prefix("KITTY_PUBLIC_KEY=").map(String::from))
        })
    }

    fn nvim_state(&self, socket: &Path) -> Result<String, String> {
        let expression = "json_encode([expand('%:p'), line('.'), col('.'), len(filter(getbufinfo({'bufmodified': 1}), 'v:val.listed'))])";
        run("nvim", &["--headless", "--clean", "--server", &socket.to_string_lossy(), "--remote-expr", expression], &[]).map(|text| text.trim().to_string())
    }

    fn history(&self, profile: &Path) -> Result<Vec<(String, String)>, String> {
        // The browser holds the database open, so a private copy is read instead.
        let dir = std::env::temp_dir().join(format!("dojo-mesh-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let copy = dir.join("History");
        let result = std::fs::copy(profile.join("History"), &copy).map_err(|error| error.to_string()).and_then(|_| {
            run("sqlite3", &["-readonly", "-json", &copy.to_string_lossy(), "select url, title from urls order by last_visit_time desc limit 5000"], &[])
        });
        let _ = std::fs::remove_dir_all(&dir);
        let text = result?;
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        #[derive(Deserialize)]
        struct Row {
            url: String,
            #[serde(default)]
            title: String,
        }
        let rows: Vec<Row> = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        Ok(rows.into_iter().map(|row| (row.url, row.title)).collect())
    }

    fn tabs(&self, runtime_dir: &Path) -> Option<crate::tabs::Snapshot> {
        crate::tabs::load(&crate::tabs::snapshot_path(runtime_dir))
    }

    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }

    fn list(&self, dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).map(|listing| listing.flatten().map(|item| item.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Fake {
        kitty: Option<String>,
        nvim: Option<String>,
        history: Vec<(String, String)>,
        files: HashMap<PathBuf, String>,
        dirs: HashMap<PathBuf, Vec<String>>,
        snapshot: Option<crate::tabs::Snapshot>,
    }

    impl Probe for Fake {
        fn kitty_ls(&self, socket: &Path, password: &str, public_key: &str) -> Result<String, String> {
            assert_eq!(socket, Path::new("/run/user/1/kitty-42.sock"));
            assert_eq!(password, "s3cret");
            assert_eq!(public_key, "1:key");
            self.kitty.clone().ok_or("no kitty".into())
        }
        fn kitty_public_key(&self, pid: i64) -> Option<String> {
            assert_eq!(pid, 42);
            self.kitty.as_ref().map(|_| "1:key".to_string())
        }
        fn nvim_state(&self, socket: &Path) -> Result<String, String> {
            // The interface process 77 has no server; its child 78 runs the editor.
            if socket == Path::new("/run/user/1/nvim.78.0") { self.nvim.clone().ok_or("no nvim".into()) } else { Err("connection refused".into()) }
        }
        fn children(&self, pid: i64) -> Vec<i64> {
            if pid == 77 { vec![78] } else { Vec::new() }
        }
        fn history(&self, profile: &Path) -> Result<Vec<(String, String)>, String> {
            assert!(profile.ends_with("Default"));
            Ok(self.history.clone())
        }
        fn tabs(&self, _: &Path) -> Option<crate::tabs::Snapshot> {
            self.snapshot.clone()
        }
        fn read(&self, path: &Path) -> Option<String> {
            self.files.get(path).cloned()
        }
        fn list(&self, dir: &Path) -> Vec<String> {
            self.dirs.get(dir).cloned().unwrap_or_default()
        }
    }

    fn capture_tab(window: &Window, context: &Context) -> Result<Captured, String> {
        capture(window, context, Scope::Tab)
    }

    fn live_snapshot(tabs: &[(&str, &str, bool)]) -> crate::tabs::Snapshot {
        crate::tabs::Snapshot {
            v: 1,
            at: 0,
            windows: vec![crate::tabs::BrowserWindow {
                id: 4,
                focused: true,
                tabs: tabs.iter().enumerate().map(|(index, (url, title, active))| crate::tabs::Tab { id: index as i64, active: *active, pinned: false, url: url.to_string(), title: title.to_string() }).collect(),
            }],
        }
    }

    fn entries() -> Vec<(String, String)> {
        vec![("kitty".into(), "[Desktop Entry]\n".into()), ("helium".into(), "[Desktop Entry]\n".into())]
    }

    fn window(app_id: &str, title: &str) -> Window {
        Window { id: 1, title: Some(title.into()), app_id: Some(app_id.into()), pid: Some(42), is_focused: true }
    }

    fn with<'a>(fake: &'a Fake, entries: &'a [(String, String)]) -> Context<'a> {
        Context { probe: fake, entries, runtime_dir: Some("/run/user/1".into()), config_dir: Some("/home/a/.config".into()) }
    }

    fn kitty_fake(foreground: &str) -> Fake {
        let mut fake = Fake::default();
        fake.files.insert("/home/a/.config/kitty/dojo-remote-control.conf".into(), "remote_control_password \"s3cret\" ls\n".into());
        fake.kitty = Some(format!(
            r#"[{{"is_focused": false, "tabs": [{{"is_active": true, "windows": [{{"is_active": true, "title": "other", "cwd": "/elsewhere", "foreground_processes": []}}]}}]}},
                {{"is_focused": true, "tabs": [{{"is_active": false, "windows": []}}, {{"is_active": true, "windows": [{{"is_active": true, "title": "Yazi: /home/a/pics", "cwd": "/home/a", "foreground_processes": [{foreground}]}}]}}]}}]"#
        ));
        fake
    }

    #[test]
    fn a_shell_in_kitty_moves_as_its_directory() {
        let fake = kitty_fake(r#"{"pid": 50, "cmdline": ["/bin/rush"], "cwd": "/home/a/project"}"#);
        let entries = entries();
        let captured = capture_tab(&window("kitty", "x"), &with(&fake, &entries)).unwrap();
        assert_eq!(captured.handoff, Handoff::Terminal { directory: "/home/a/project".into(), program: None });
        assert!(captured.notes[0].contains("shell starts fresh"));
    }

    #[test]
    fn nvim_in_kitty_moves_as_its_file_and_position() {
        let mut fake = kitty_fake(r#"{"pid": 77, "cmdline": ["nvim", "a.rs"], "cwd": "/home/a/project"}"#);
        fake.nvim = Some(r#"["/home/a/project/a.rs", 12, 5, 0]"#.into());
        let entries = entries();
        let captured = capture_tab(&window("kitty", "x"), &with(&fake, &entries)).unwrap();
        assert_eq!(captured.handoff, Handoff::Terminal { directory: "/home/a/project".into(), program: Some(Program::Editor { file: "/home/a/project/a.rs".into(), line: 12, column: 5 }) });
        assert!(captured.notes.is_empty());
    }

    #[test]
    fn nvim_with_unsaved_changes_is_not_sent() {
        let mut fake = kitty_fake(r#"{"pid": 77, "cmdline": ["nvim"], "cwd": "/w"}"#);
        fake.nvim = Some(r#"["/w/a.rs", 1, 1, 2]"#.into());
        let entries = entries();
        let error = capture_tab(&window("kitty", "x"), &with(&fake, &entries)).unwrap_err();
        assert!(error.contains("2 unsaved buffers"), "{error}");
    }

    #[test]
    fn yazi_moves_as_the_directory_it_shows() {
        let fake = kitty_fake(r#"{"pid": 60, "cmdline": ["yazi"], "cwd": "/home/a"}"#);
        let entries = entries();
        let captured = capture_tab(&window("kitty", "x"), &with(&fake, &entries)).unwrap();
        assert_eq!(captured.handoff, Handoff::Terminal { directory: "/home/a".into(), program: Some(Program::Files { directory: "/home/a/pics".into() }) });
    }

    #[test]
    fn another_program_is_named_as_not_moving() {
        let fake = kitty_fake(r#"{"pid": 61, "cmdline": ["/usr/bin/cargo", "build"], "cwd": "/w"}"#);
        let entries = entries();
        let captured = capture_tab(&window("kitty", "x"), &with(&fake, &entries)).unwrap();
        assert_eq!(captured.handoff, Handoff::Terminal { directory: "/w".into(), program: None });
        assert!(captured.notes[0].starts_with("cargo was running there"), "{:?}", captured.notes);
    }

    #[test]
    fn without_dojos_kitty_password_the_terminal_is_relaunched_with_a_reason() {
        let mut fake = kitty_fake(r#"{"pid": 50, "cmdline": ["rush"], "cwd": "/w"}"#);
        fake.files.clear();
        let entries = entries();
        let captured = capture_tab(&window("kitty", "x"), &with(&fake, &entries)).unwrap();
        assert_eq!(captured.handoff, Handoff::Relaunch { desktop_entry: "kitty".into() });
        assert!(captured.notes[0].contains("remote control is not set up"), "{:?}", captured.notes);
    }

    #[test]
    fn the_kitty_password_must_be_limited_to_ls() {
        assert_eq!(kitty_password("remote_control_password \"abc\" ls\n").as_deref(), Some("abc"));
        assert_eq!(kitty_password("remote_control_password \"abc\" ls send-text\n"), None);
        assert_eq!(kitty_password("remote_control_password \"abc\"\n"), None, "an unlimited password is not used");
        assert_eq!(kitty_password("# nothing"), None);
    }

    #[test]
    fn a_browser_window_moves_as_the_page_whose_title_it_shows() {
        let mut fake = Fake::default();
        fake.dirs.insert("/home/a/.config/net.imput.helium".into(), vec!["Default".into(), "Local State".into()]);
        fake.history = vec![("https://new.example/x".into(), "Other".into()), ("https://example.com/page".into(), "The Page".into()), ("https://old.example/".into(), "The Page".into())];
        let entries = entries();
        let captured = capture_tab(&window("helium", "The Page - Helium"), &with(&fake, &entries)).unwrap();
        assert_eq!(captured.handoff, Handoff::Page { url: "https://example.com/page".into() });
    }

    #[test]
    fn a_page_not_in_history_relaunches_the_browser_and_says_why() {
        let mut fake = Fake::default();
        fake.dirs.insert("/home/a/.config/net.imput.helium".into(), vec!["Default".into()]);
        fake.history = vec![("file:///etc/x".into(), "Secret".into())];
        let entries = entries();
        let captured = capture_tab(&window("helium", "Secret - Helium"), &with(&fake, &entries)).unwrap();
        assert_eq!(captured.handoff, Handoff::Relaunch { desktop_entry: "helium".into() });
        assert!(captured.notes[0].contains("could not be found"), "{:?}", captured.notes);
    }

    #[test]
    fn an_application_without_its_own_adapter_is_relaunched() {
        let fake = Fake::default();
        let entries = vec![("org.example.App".to_string(), "[Desktop Entry]\n".to_string())];
        let captured = capture_tab(&window("org.example.App", "t"), &with(&fake, &entries)).unwrap();
        assert_eq!(captured.handoff, Handoff::Relaunch { desktop_entry: "org.example.App".into() });
        assert!(capture_tab(&window("mystery", "t"), &with(&fake, &entries)).unwrap_err().contains("mystery has no desktop entry"));
    }

    #[test]
    fn with_the_extension_the_front_tab_is_exact_and_the_whole_window_can_be_sent() {
        let mut fake = Fake::default();
        fake.snapshot = Some(live_snapshot(&[("https://a.example/", "A", false), ("https://b.example/page", "The Page", true), ("https://c.example/", "C", false)]));
        let entries = entries();
        let context = with(&fake, &entries);
        let tab = capture(&window("helium", "The Page - Helium"), &context, Scope::Tab).unwrap();
        assert_eq!(tab.handoff, Handoff::Page { url: "https://b.example/page".into() }, "no history was needed");
        assert_eq!(tab.closes, Closes::Tab { title: "The Page - Helium".into() });
        assert_eq!((tab.scopes.clone(), tab.tab_count), (vec![Scope::Tab, Scope::Window], Some(3)));

        let all = capture(&window("helium", "The Page - Helium"), &context, Scope::Window).unwrap();
        let urls: Vec<String> = ["https://a.example/", "https://b.example/page", "https://c.example/"].iter().map(|u| u.to_string()).collect();
        assert_eq!(all.handoff, Handoff::BrowserWindow { desktop_entry: "helium".into(), urls: urls.clone() });
        assert_eq!(all.closes, Closes::BrowserWindow { title: "The Page - Helium".into(), browser: "Helium".into(), urls });
        assert!(all.notes[0].contains("pinned tabs and tab groups do not carry over"));
    }

    #[test]
    fn a_window_with_browser_pages_is_sent_without_them_and_stays_open() {
        let mut fake = Fake::default();
        fake.snapshot = Some(live_snapshot(&[("https://a.example/", "The Page", true), ("chrome://settings/", "Settings", false), ("https://c.example/", "C", false)]));
        let entries = entries();
        let all = capture(&window("helium", "The Page - Helium"), &with(&fake, &entries), Scope::Window).unwrap();
        assert_eq!(all.handoff, Handoff::BrowserWindow { desktop_entry: "helium".into(), urls: vec!["https://a.example/".into(), "https://c.example/".into()] });
        assert!(matches!(&all.closes, Closes::Nothing { why } if why.contains("1 tab cannot be sent")), "{:?}", all.closes);
        assert!(all.notes.iter().any(|note| note.contains("1 tab (browser pages) was not sent")), "{:?}", all.notes);
    }

    #[test]
    fn without_the_extension_only_the_front_tab_can_be_sent() {
        let mut fake = Fake::default();
        fake.dirs.insert("/home/a/.config/net.imput.helium".into(), vec!["Default".into()]);
        fake.history = vec![("https://example.com/page".into(), "The Page".into())];
        let entries = entries();
        let context = with(&fake, &entries);
        let tab = capture(&window("helium", "The Page - Helium"), &context, Scope::Tab).unwrap();
        assert_eq!(tab.handoff, Handoff::Page { url: "https://example.com/page".into() });
        assert!(tab.scopes.is_empty(), "there is no choice to offer");
        let error = capture(&window("helium", "The Page - Helium"), &context, Scope::Window).unwrap_err();
        assert!(error.contains("Dojo Tabs extension"), "{error}");
    }

    #[test]
    fn other_applications_have_one_way_to_be_sent() {
        let fake = kitty_fake(r#"{"pid": 50, "cmdline": ["rush"], "cwd": "/w"}"#);
        let entries = entries();
        assert!(capture(&window("kitty", "x"), &with(&fake, &entries), Scope::Window).unwrap_err().contains("only one way"));
        assert!(capture(&window("org.example.App", "t"), &with(&Fake::default(), &entries), Scope::Window).unwrap_err().contains("only one way"));
    }

    #[test]
    fn an_oversized_window_is_refused_not_cut() {
        let mut fake = Fake::default();
        let many: Vec<(String, String, bool)> = (0..=MAX_TABS).map(|i| (format!("https://e.example/{i}"), format!("T{i}"), i == 0)).collect();
        let borrowed: Vec<(&str, &str, bool)> = many.iter().map(|(u, t, a)| (u.as_str(), t.as_str(), *a)).collect();
        fake.snapshot = Some(live_snapshot(&borrowed));
        let entries = entries();
        let error = capture(&window("helium", "T0 - Helium"), &with(&fake, &entries), Scope::Window).unwrap_err();
        assert!(error.contains("more than the 100"), "{error}");
    }
}
