//! Handoff adapters: for one window, what it shows, as a [`Handoff`] the receiver can open.

use crate::handoff::Handoff;
use crate::niri::Window;

/// What an adapter read from a window.
#[derive(Debug, PartialEq, Eq)]
pub struct Captured {
    pub handoff: Handoff,
    /// What the receiver will not have, said to the operator.
    pub notes: Vec<String>,
}

/// The best handoff for a window. `entries` are this machine's desktop entries (id, contents).
pub fn capture(window: &Window, entries: &[(String, String)]) -> Result<Captured, String> {
    let app_id = window.app_id.as_deref().unwrap_or_default();
    let entry = crate::desktop::for_app(app_id, entries).ok_or_else(|| format!("{} has no desktop entry here, so it cannot be started elsewhere", if app_id.is_empty() { "this window" } else { app_id }))?;
    Ok(Captured { handoff: Handoff::Relaunch { desktop_entry: entry }, notes: vec!["it is started fresh: what the window was showing does not carry over".into()] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(app_id: Option<&str>) -> Window {
        Window { id: 1, title: Some("t".into()), app_id: app_id.map(String::from), pid: Some(1), is_focused: true }
    }

    #[test]
    fn an_application_without_its_own_adapter_is_relaunched() {
        let entries = vec![("org.example.App".to_string(), "[Desktop Entry]\\n".to_string())];
        let captured = capture(&window(Some("org.example.App")), &entries).unwrap();
        assert_eq!(captured.handoff, Handoff::Relaunch { desktop_entry: "org.example.App".into() });
        assert_eq!(captured.notes.len(), 1);
    }

    #[test]
    fn a_window_with_no_known_application_cannot_be_sent() {
        assert!(capture(&window(Some("mystery")), &[]).unwrap_err().contains("mystery has no desktop entry"));
        assert!(capture(&window(None), &[]).unwrap_err().contains("this window"));
    }
}
