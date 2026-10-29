//! The apps that have sent notifications, newest first, so the settings window can offer
//! per-app choices for them. Kept in `$XDG_STATE_HOME/hokum/apps`, one name per line. The
//! installed desktop applications fill the rest of the list, so a rule can be set before an app
//! has sent anything, and give a notification's sender its proper name.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const KEEP: usize = 40;

fn path() -> PathBuf {
    crate::style::state_dir().join("apps")
}

fn list() -> &'static Mutex<Vec<String>> {
    static APPS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    APPS.get_or_init(|| {
        let text = std::fs::read_to_string(path()).unwrap_or_default();
        Mutex::new(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect())
    })
}

/// Notes that `app` sent something; saves when the list changes.
pub fn record(app: &str) {
    let app = app.trim();
    if app.is_empty() {
        return;
    }
    let mut apps = list().lock().unwrap();
    if apps.first().is_some_and(|a| a == app) {
        return;
    }
    apps.retain(|a| a != app);
    apps.insert(0, app.to_owned());
    apps.truncate(KEEP);
    let text = apps.join("\n") + "\n";
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, text);
}

pub fn seen() -> Vec<String> {
    list().lock().unwrap().clone()
}

/// An application with a desktop entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    /// The entry's `Name=`, which is what lists and rules call the app.
    pub name: String,
    /// The entry's `Icon=`: a theme name or a path.
    pub icon: Option<String>,
    /// What notifications may call it besides its name: the file's id (`org.signal.Signal`
    /// and `Signal`) and `StartupWMClass`, lowercased.
    ids: Vec<String>,
}

/// The `[Desktop Entry]` of a desktop file as an application a person launches.
fn parse(id: &str, text: &str) -> Option<Installed> {
    let (mut name, mut icon, mut wm_class) = (None, None, None);
    let (mut app, mut hidden, mut in_entry) = (false, false, false);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "Name" => name = Some(value.trim().to_owned()),
            "Icon" => icon = Some(value.trim().to_owned()),
            "StartupWMClass" => wm_class = Some(value.trim().to_lowercase()),
            "Type" => app = value.trim() == "Application",
            "NoDisplay" | "Hidden" => hidden |= value.trim() == "true",
            _ => {}
        }
    }
    let name = name.filter(|n| !n.is_empty())?;
    if !app || hidden {
        return None;
    }
    let stem = id.trim_end_matches(".desktop").to_lowercase();
    let mut ids = vec![stem.clone()];
    if let Some(last) = stem.rsplit('.').next().filter(|l| *l != stem) {
        ids.push(last.to_owned());
    }
    ids.extend(wm_class);
    Some(Installed { name, icon: icon.filter(|i| !i.is_empty()), ids })
}

/// Every launchable desktop application, by name, read once. An entry in an earlier data
/// directory hides a later one of the same file name, as the desktop entry spec says.
pub fn installed() -> &'static [Installed] {
    static INSTALLED: OnceLock<Vec<Installed>> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let mut seen_files = std::collections::HashSet::new();
        let mut apps: Vec<Installed> = Vec::new();
        for dir in crate::icons::data_dirs() {
            let Ok(read) = std::fs::read_dir(dir.join("applications")) else { continue };
            let mut files: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "desktop")).collect();
            files.sort();
            for file in files {
                let Some(id) = file.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
                if !seen_files.insert(id.clone()) {
                    continue;
                }
                let Some(app) = std::fs::read_to_string(&file).ok().and_then(|t| parse(&id, &t)) else { continue };
                match apps.iter_mut().find(|a| a.name.eq_ignore_ascii_case(&app.name)) {
                    Some(same) => same.ids.extend(app.ids),
                    None => apps.push(app),
                }
            }
        }
        apps.sort_by_key(|a| a.name.to_lowercase());
        apps
    })
}

/// The proper name of the application that sent a notification: the entry its `desktop-entry`
/// hint names, else the one its app name is the name or id of. None when it has no entry.
pub fn name_for(app: &str, desktop: Option<&str>) -> Option<String> {
    name_in(installed(), app, desktop)
}

fn name_in(apps: &[Installed], app: &str, desktop: Option<&str>) -> Option<String> {
    let key = |s: &str| s.trim().trim_end_matches(".desktop").to_lowercase();
    let by_id = |id: &str| {
        let id = key(id);
        (!id.is_empty()).then(|| apps.iter().find(|a| a.ids.contains(&id))).flatten()
    };
    desktop
        .and_then(by_id)
        .or_else(|| apps.iter().find(|a| a.name.eq_ignore_ascii_case(app.trim())))
        .or_else(|| by_id(app))
        .map(|a| a.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, text: &str) -> Installed {
        parse(id, text).unwrap()
    }

    #[test]
    fn a_desktop_entry_is_an_application_with_a_name_an_icon_and_ids() {
        let a = entry("org.signal.Signal.desktop", "[Desktop Entry]\nType=Application\nName=Signal\nIcon=signal-desktop\nStartupWMClass=Signal\n[Desktop Action x]\nName=Other\n");
        assert_eq!((a.name.as_str(), a.icon.as_deref()), ("Signal", Some("signal-desktop")));
        assert!(a.ids.contains(&"org.signal.signal".to_owned()) && a.ids.contains(&"signal".to_owned()));
        assert!(parse("x.desktop", "[Desktop Entry]\nType=Application\nName=Hidden thing\nNoDisplay=true\n").is_none());
        assert!(parse("x.desktop", "[Desktop Entry]\nType=Link\nName=A link\n").is_none());
        assert!(parse("x.desktop", "[Desktop Entry]\nType=Application\n").is_none());
    }

    #[test]
    fn a_sender_gets_the_name_of_its_entry_by_hint_name_or_id() {
        let apps = vec![
            entry("org.signal.Signal.desktop", "[Desktop Entry]\nType=Application\nName=Signal\n"),
            entry("firefox.desktop", "[Desktop Entry]\nType=Application\nName=Firefox Web Browser\n"),
        ];
        assert_eq!(name_in(&apps, "whatever", Some("org.signal.Signal")).as_deref(), Some("Signal"));
        assert_eq!(name_in(&apps, "signal", None).as_deref(), Some("Signal"));
        assert_eq!(name_in(&apps, "FIREFOX", None).as_deref(), Some("Firefox Web Browser"));
        assert_eq!(name_in(&apps, "Firefox Web Browser", None).as_deref(), Some("Firefox Web Browser"));
        assert_eq!(name_in(&apps, "unknown", Some("nothing")), None);
        assert_eq!(name_in(&apps, "", None), None);
    }
}
