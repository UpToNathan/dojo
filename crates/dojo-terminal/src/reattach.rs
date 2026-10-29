//! `--reattach`: the session to open again, and the window that may already show it.
use dojo_rex::SessionSummary;
use serde_json::Value;

pub const APP_ID: &str = "dojo-terminal";

/// The most recently used session (by ID) that still exists with at least one tab; None means
/// a new one.
pub fn choose(recent: &[String], live: &[SessionSummary]) -> Option<SessionSummary> {
    recent.iter().find_map(|id| live.iter().find(|session| &session.id == id && session.tabs > 0)).cloned()
}

/// The Niri window id of a `dojo-terminal` window titled `title`, from `niri msg --json windows`.
pub fn find_window(windows: &Value, title: &str) -> Option<u64> {
    windows
        .as_array()?
        .iter()
        .find(|window| window["app_id"] == APP_ID && window["title"] == title)
        .and_then(|window| window["id"].as_u64())
}

/// Focuses the window already showing `title`; true when there was one.
pub fn focus_existing(title: &str) -> bool {
    let Ok(output) = std::process::Command::new("niri").args(["msg", "--json", "windows"]).output() else { return false };
    let Ok(windows) = serde_json::from_slice::<Value>(&output.stdout) else { return false };
    let Some(id) = find_window(&windows, title) else { return false };
    std::process::Command::new("niri")
        .args(["msg", "action", "focus-window", "--id", &id.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn summary(id: &str, tabs: usize) -> SessionSummary {
        SessionSummary { id: id.into(), label: format!("name-{id}"), tabs, directory: None, title: None, status: None }
    }

    #[test]
    fn the_first_recent_session_by_id_that_still_has_tabs_wins() {
        let live = [summary("a", 1), summary("b", 0), summary("c", 2)];
        let recent = ["gone".to_owned(), "b".to_owned(), "c".to_owned(), "a".to_owned()];
        assert_eq!(choose(&recent, &live).map(|s| s.label), Some("name-c".into()));
        assert_eq!(choose(&[], &live), None, "nothing recent: a new session");
        assert_eq!(choose(&["gone".to_owned()], &live), None);
    }

    #[test]
    fn a_window_matches_by_app_id_and_title() {
        let windows = json!([
            {"id": 3, "app_id": "kitty", "title": "work"},
            {"id": 7, "app_id": "dojo-terminal", "title": "play"},
            {"id": 9, "app_id": "dojo-terminal", "title": "work"},
        ]);
        assert_eq!(find_window(&windows, "work"), Some(9));
        assert_eq!(find_window(&windows, "none"), None);
        assert_eq!(find_window(&json!({}), "work"), None);
    }
}
