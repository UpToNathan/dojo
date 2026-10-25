//! Desktop entries: which application a window belongs to, and whether this machine has it.

use std::path::{Path, PathBuf};

/// The directories that hold `.desktop` files, most specific first.
pub fn directories(data_home: Option<PathBuf>, home: Option<PathBuf>, data_dirs: Option<String>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = data_home.filter(|dir| dir.is_absolute()).or_else(|| home.as_ref().map(|home| home.join(".local/share"))) {
        dirs.push(dir.join("applications"));
    }
    if let Some(home) = &home {
        dirs.push(home.join(".nix-profile/share/applications"));
    }
    let data_dirs = data_dirs.filter(|text| !text.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(data_dirs.split(':').filter(|dir| dir.starts_with('/')).map(|dir| Path::new(dir).join("applications")));
    dirs.dedup();
    dirs
}

pub fn from_process() -> Vec<PathBuf> {
    directories(std::env::var_os("XDG_DATA_HOME").map(PathBuf::from), std::env::var_os("HOME").map(PathBuf::from), std::env::var("XDG_DATA_DIRS").ok())
}

/// The desktop entries in `dirs`: id (file name without `.desktop`) and contents.
pub fn entries(dirs: &[PathBuf]) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for dir in dirs {
        let Ok(listing) = std::fs::read_dir(dir) else { continue };
        for item in listing.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".desktop") else { continue };
            if found.iter().any(|(seen, _)| seen == id) {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(item.path()) {
                found.push((id.to_string(), text));
            }
        }
    }
    found
}

/// The entry for a window's application id: the entry of that name (any case), else the one whose
/// `StartupWMClass` is that id. A deleted entry (`Hidden`) never matches; one kept out of menus
/// (`NoDisplay`) still does by name, since it is how such applications start.
pub fn for_app(app_id: &str, entries: &[(String, String)]) -> Option<String> {
    let flag = |text: &str, key: &str| text.lines().any(|line| line.trim() == format!("{key}=true"));
    let visible = |text: &str| !flag(text, "NoDisplay") && !flag(text, "Hidden");
    let by_name = entries.iter().find(|(id, text)| id.eq_ignore_ascii_case(app_id) && !flag(text, "Hidden"));
    let by_class = || entries.iter().find(|(_, text)| visible(text) && text.lines().any(|line| line.trim().strip_prefix("StartupWMClass=").is_some_and(|class| class.eq_ignore_ascii_case(app_id))));
    by_name.or_else(by_class).map(|(id, _)| id.clone())
}

/// The `Exec` line of an entry that is not marked deleted, searching `dirs` in order.
pub fn exec_of(id: &str, dirs: &[PathBuf]) -> Option<String> {
    let text = dirs.iter().find_map(|dir| std::fs::read_to_string(dir.join(format!("{id}.desktop"))).ok())?;
    if text.lines().any(|line| line.trim() == "Hidden=true") {
        return None;
    }
    let in_entry = text.lines().skip_while(|line| line.trim() != "[Desktop Entry]").skip(1).take_while(|line| !line.trim_start().starts_with('['));
    in_entry.into_iter().find_map(|line| line.strip_prefix("Exec=")).map(|exec| exec.trim().to_string())
}

pub fn has(id: &str, dirs: &[PathBuf]) -> bool {
    dirs.iter().any(|dir| dir.join(format!("{id}.desktop")).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, text: &str) -> (String, String) {
        (id.to_string(), format!("[Desktop Entry]\nName=x\n{text}"))
    }

    #[test]
    fn finds_an_entry_by_name_or_window_class() {
        let all = vec![
            entry("org.gnome.Nautilus", ""),
            entry("helium", "StartupWMClass=Helium-browser"),
            entry("deleted", "Hidden=true"),
            entry("quiet.App", "NoDisplay=true"),
            entry("menu-alias", "NoDisplay=true\nStartupWMClass=quiet-class"),
        ];
        assert_eq!(for_app("org.gnome.nautilus", &all).as_deref(), Some("org.gnome.Nautilus"));
        assert_eq!(for_app("helium-browser", &all).as_deref(), Some("helium"));
        assert_eq!(for_app("deleted", &all), None);
        assert_eq!(for_app("quiet.app", &all).as_deref(), Some("quiet.App"), "kept out of menus but launchable by name");
        assert_eq!(for_app("quiet-class", &all), None, "a hidden alias is not found by window class");
        assert_eq!(for_app("unknown", &all), None);
    }

    #[test]
    fn directories_follow_the_xdg_order() {
        let dirs = directories(None, Some("/h".into()), None);
        assert_eq!(dirs, [PathBuf::from("/h/.local/share/applications"), "/h/.nix-profile/share/applications".into(), "/usr/local/share/applications".into(), "/usr/share/applications".into()]);
        let dirs = directories(Some("/d".into()), None, Some("/a:relative:/b".into()));
        assert_eq!(dirs, [PathBuf::from("/d/applications"), "/a/applications".into(), "/b/applications".into()]);
    }

    #[test]
    fn reads_entries_from_disk_first_directory_winning() {
        let root = std::env::temp_dir().join(format!("dojo-mesh-desktop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (first, second) = (root.join("one"), root.join("two"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("app.desktop"), "first").unwrap();
        std::fs::write(second.join("app.desktop"), "second").unwrap();
        std::fs::write(second.join("other.desktop"), "x").unwrap();
        std::fs::write(second.join("notes.txt"), "x").unwrap();
        let dirs = vec![first, second];
        let mut found = entries(&dirs);
        found.sort();
        assert_eq!(found, [("app".to_string(), "first".to_string()), ("other".to_string(), "x".to_string())]);
        assert!(has("other", &dirs) && !has("missing", &dirs));
        assert_eq!(exec_of("other", &dirs), None, "no Exec line");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
