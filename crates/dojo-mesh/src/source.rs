//! Closing what was sent, on the machine it came from, once the peer has opened it. A window
//! closes whole. A browser window holds tabs and only the one in front was sent, so only that tab
//! closes: the window is checked to still show that tab, then sent Ctrl+W through an input seat of
//! its own, so the person's focus does not move.

use crate::handoff::Closes;
use crate::niri::Window;
use std::path::PathBuf;

/// What closing needs from the desktop; the real one is Niri plus `wtype`.
pub trait Desktop {
    fn windows(&self) -> Result<Vec<Window>, String>;
    fn close_window(&self, id: u64) -> Result<(), String>;
    /// Presses Ctrl+W in that window.
    fn press_close_tab(&self, window: u64) -> Result<(), String>;
}

/// Closes what was sent and says what it closed. An error means nothing was closed.
pub fn close(desktop: &dyn Desktop, window: u64, closes: &Closes) -> Result<String, String> {
    match closes {
        Closes::Window => {
            desktop.close_window(window)?;
            Ok("closed the window here".into())
        }
        Closes::Tab { title } => {
            let windows = desktop.windows()?;
            let found = windows.iter().find(|candidate| candidate.id == window).ok_or("the window is gone, so there was nothing to close")?;
            if found.title.as_deref() != Some(title.as_str()) {
                return Err(format!("the window no longer shows \"{title}\", so no tab was closed; close it yourself"));
            }
            desktop.press_close_tab(window)?;
            Ok("closed that tab here".into())
        }
    }
}

pub struct System {
    pub socket: PathBuf,
    /// Whether the operator started this. Closing a tab needs Niri to end its pause on windows the
    /// person has used, which only the operator may ask for: the agent must not override that.
    pub operator: bool,
}

impl Desktop for System {
    fn windows(&self) -> Result<Vec<Window>, String> {
        crate::niri::windows(&self.socket)
    }

    fn close_window(&self, id: u64) -> Result<(), String> {
        crate::niri::close_window(&self.socket, id)
    }

    fn press_close_tab(&self, window: u64) -> Result<(), String> {
        if !self.operator {
            return Err("closing a tab is for the operator, so the tab was not closed; the agent cannot override the person's input".into());
        }
        let tab_stays = |message: String| format!("{message}, so the tab was not closed; close it yourself");
        let mut seat = crate::niri::AgentSeat::connect(&self.socket).map_err(&tab_stays)?;
        seat.yield_human_input().map_err(&tab_stays)?;
        seat.create("dojo-mesh").map_err(&tab_stays)?;
        seat.focus(window).map_err(&tab_stays)?;
        seat.close_tab_keys(window).map_err(&tab_stays)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fake {
        log: RefCell<Vec<String>>,
        title_now: RefCell<String>,
    }

    impl Fake {
        fn new(title: &str) -> Self {
            Self { log: RefCell::new(Vec::new()), title_now: RefCell::new(title.into()) }
        }
    }

    impl Desktop for Fake {
        fn windows(&self) -> Result<Vec<Window>, String> {
            Ok(vec![Window { id: 7, title: Some(self.title_now.borrow().clone()), app_id: Some("helium".into()), pid: Some(1), is_focused: false }])
        }
        fn close_window(&self, id: u64) -> Result<(), String> {
            self.log.borrow_mut().push(format!("close-window {id}"));
            Ok(())
        }
        fn press_close_tab(&self, window: u64) -> Result<(), String> {
            self.log.borrow_mut().push(format!("ctrl+w {window}"));
            Ok(())
        }
    }

    #[test]
    fn a_window_closes_whole() {
        let fake = Fake::new("T");
        assert_eq!(close(&fake, 7, &Closes::Window).unwrap(), "closed the window here");
        assert_eq!(*fake.log.borrow(), ["close-window 7"]);
    }

    #[test]
    fn a_tab_is_closed_with_ctrl_w_in_its_window_never_by_closing_the_window() {
        let fake = Fake::new("Page - Helium");
        assert_eq!(close(&fake, 7, &Closes::Tab { title: "Page - Helium".into() }).unwrap(), "closed that tab here");
        assert_eq!(*fake.log.borrow(), ["ctrl+w 7"]);
    }

    #[test]
    fn a_window_that_moved_on_to_another_tab_is_left_alone() {
        let fake = Fake::new("Other - Helium");
        let error = close(&fake, 7, &Closes::Tab { title: "Page - Helium".into() }).unwrap_err();
        assert!(error.contains("no longer shows \"Page - Helium\""), "{error}");
        assert!(fake.log.borrow().is_empty(), "no key was sent");
    }

    #[test]
    fn a_vanished_window_is_reported() {
        struct Empty;
        impl Desktop for Empty {
            fn windows(&self) -> Result<Vec<Window>, String> {
                Ok(Vec::new())
            }
            fn close_window(&self, _: u64) -> Result<(), String> {
                unreachable!()
            }
            fn press_close_tab(&self, _: u64) -> Result<(), String> {
                unreachable!()
            }
        }
        assert!(close(&Empty, 7, &Closes::Tab { title: "T".into() }).unwrap_err().contains("the window is gone"));
    }
}
