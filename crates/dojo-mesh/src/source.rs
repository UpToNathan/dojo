//! Closing what was sent, on the machine it came from, once the peer has opened it. A window
//! closes whole. A browser window holds tabs and only the one in front was sent, so only that tab
//! closes: the window is focused, checked to still show that tab, and sent Ctrl+W.

use crate::handoff::Closes;
use crate::niri::Window;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// What closing needs from the desktop; the real one is Niri plus `wtype`.
pub trait Desktop {
    fn windows(&self) -> Result<Vec<Window>, String>;
    fn focus(&self, id: u64) -> Result<(), String>;
    fn close_window(&self, id: u64) -> Result<(), String>;
    /// Presses Ctrl+W in the focused window.
    fn press_close_tab(&self) -> Result<(), String>;
    fn wait(&self, milliseconds: u64);
}

/// How long the window is given to take focus before the tab is left alone.
const FOCUS_WAIT_MS: u64 = 1500;

/// Closes what was sent and says what it closed. An error means nothing was closed.
pub fn close(desktop: &dyn Desktop, window: u64, closes: &Closes) -> Result<String, String> {
    match closes {
        Closes::Window => {
            desktop.close_window(window)?;
            Ok("closed the window here".into())
        }
        Closes::Tab { title } => {
            let shows = |desktop: &dyn Desktop| -> Result<(bool, bool), String> {
                let windows = desktop.windows()?;
                let found = windows.iter().find(|candidate| candidate.id == window).ok_or("the window is gone, so there was nothing to close")?;
                Ok((found.title.as_deref() == Some(title.as_str()), found.is_focused))
            };
            if !shows(desktop)?.0 {
                return Err(format!("the window no longer shows \"{title}\", so no tab was closed; close it yourself"));
            }
            desktop.focus(window)?;
            let mut waited = 0;
            loop {
                let (same, focused) = shows(desktop)?;
                if !same {
                    return Err(format!("the window no longer shows \"{title}\", so no tab was closed; close it yourself"));
                }
                if focused {
                    break;
                }
                if waited >= FOCUS_WAIT_MS {
                    return Err("the browser window did not take focus, so no tab was closed; close it yourself".into());
                }
                desktop.wait(50);
                waited += 50;
            }
            desktop.press_close_tab()?;
            Ok("closed that tab here".into())
        }
    }
}

pub struct System {
    pub socket: PathBuf,
}

impl Desktop for System {
    fn windows(&self) -> Result<Vec<Window>, String> {
        crate::niri::windows(&self.socket)
    }

    fn focus(&self, id: u64) -> Result<(), String> {
        crate::niri::focus_window(&self.socket, id)
    }

    fn close_window(&self, id: u64) -> Result<(), String> {
        crate::niri::close_window(&self.socket, id)
    }

    fn press_close_tab(&self) -> Result<(), String> {
        let status = Command::new("wtype")
            .args(["-M", "ctrl", "w", "-m", "ctrl"])
            .stdin(Stdio::null())
            .status()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound { "wtype is not installed, so the tab was not closed; close it yourself".to_string() } else { format!("could not run wtype: {error}") }
            })?;
        status.success().then_some(()).ok_or_else(|| "wtype could not press the key, so the tab was not closed; close it yourself".to_string())
    }

    fn wait(&self, milliseconds: u64) {
        std::thread::sleep(Duration::from_millis(milliseconds));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        /// Focus is gained after this many waits (None: never).
        focus_after: Option<u32>,
        waits: RefCell<u32>,
        log: RefCell<Vec<String>>,
        change_title_on_focus: Option<String>,
        title_now: RefCell<String>,
    }

    impl Fake {
        fn new(title: &str) -> Self {
            Self { focus_after: Some(0), waits: RefCell::new(0), log: RefCell::new(Vec::new()), change_title_on_focus: None, title_now: RefCell::new(title.into()) }
        }
    }

    impl Desktop for Fake {
        fn windows(&self) -> Result<Vec<Window>, String> {
            let focused = self.log.borrow().iter().any(|entry| entry == "focus") && self.focus_after.is_some_and(|after| *self.waits.borrow() >= after);
            Ok(vec![Window { id: 7, title: Some(self.title_now.borrow().clone()), app_id: Some("helium".into()), pid: Some(1), is_focused: focused }])
        }
        fn focus(&self, id: u64) -> Result<(), String> {
            assert_eq!(id, 7);
            self.log.borrow_mut().push("focus".into());
            if let Some(new) = &self.change_title_on_focus {
                *self.title_now.borrow_mut() = new.clone();
            }
            Ok(())
        }
        fn close_window(&self, id: u64) -> Result<(), String> {
            self.log.borrow_mut().push(format!("close-window {id}"));
            Ok(())
        }
        fn press_close_tab(&self) -> Result<(), String> {
            self.log.borrow_mut().push("ctrl+w".into());
            Ok(())
        }
        fn wait(&self, _: u64) {
            *self.waits.borrow_mut() += 1;
        }
    }

    #[test]
    fn a_window_closes_whole() {
        let fake = Fake::new("T");
        assert_eq!(close(&fake, 7, &Closes::Window).unwrap(), "closed the window here");
        assert_eq!(*fake.log.borrow(), ["close-window 7"]);
    }

    #[test]
    fn a_tab_is_closed_by_focusing_its_window_and_pressing_ctrl_w_never_closing_the_window() {
        let mut fake = Fake::new("Page - Helium");
        fake.focus_after = Some(3);
        assert_eq!(close(&fake, 7, &Closes::Tab { title: "Page - Helium".into() }).unwrap(), "closed that tab here");
        assert_eq!(*fake.log.borrow(), ["focus", "ctrl+w"]);
    }

    #[test]
    fn a_window_that_moved_on_to_another_tab_is_left_alone() {
        let fake = Fake::new("Other - Helium");
        let error = close(&fake, 7, &Closes::Tab { title: "Page - Helium".into() }).unwrap_err();
        assert!(error.contains("no longer shows \"Page - Helium\""), "{error}");
        assert!(fake.log.borrow().is_empty(), "nothing was pressed or focused");
    }

    #[test]
    fn a_tab_that_changes_while_focusing_is_not_closed() {
        let mut fake = Fake::new("Page - Helium");
        fake.change_title_on_focus = Some("Switched - Helium".into());
        assert!(close(&fake, 7, &Closes::Tab { title: "Page - Helium".into() }).is_err());
        assert_eq!(*fake.log.borrow(), ["focus"], "no key was sent");
    }

    #[test]
    fn a_window_that_never_takes_focus_is_not_closed() {
        let mut fake = Fake::new("Page - Helium");
        fake.focus_after = None;
        let error = close(&fake, 7, &Closes::Tab { title: "Page - Helium".into() }).unwrap_err();
        assert!(error.contains("did not take focus"), "{error}");
        assert_eq!(*fake.log.borrow(), ["focus"]);
    }
}
