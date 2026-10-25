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
    /// The addresses of the tabs the browser window titled `title` holds now, in order, if the
    /// browser can say (its Dojo Tabs extension is connected and the window is identifiable).
    fn browser_tab_urls(&self, title: &str, browser: &str) -> Option<Vec<String>>;
}

/// Closes what was sent and says what it closed. An error means nothing was closed.
pub fn close(desktop: &dyn Desktop, window: u64, closes: &Closes) -> Result<String, String> {
    match closes {
        Closes::Window => {
            desktop.close_window(window)?;
            Ok("closed the window here".into())
        }
        Closes::Nothing { why } => Ok(format!("left it open here ({why})")),
        Closes::BrowserWindow { title, browser, urls } => {
            // The tabs are re-read now: one opened since the list was taken would be lost with the window.
            let windows = desktop.windows()?;
            let found = windows.iter().find(|candidate| candidate.id == window).ok_or("the window is gone, so there was nothing to close")?;
            if found.title.as_deref() != Some(title.as_str()) {
                return Err(format!("the window no longer shows \"{title}\", so it was left open"));
            }
            match desktop.browser_tab_urls(title, browser) {
                Some(now) if now == *urls => {
                    desktop.close_window(window)?;
                    Ok("closed the window here".into())
                }
                Some(_) => Err("the window's tabs changed while sending, so it was left open".into()),
                None => Err("the browser could not confirm the window's tabs, so it was left open".into()),
            }
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
    /// The runtime directory the Dojo Tabs report is kept in.
    pub runtime_dir: Option<PathBuf>,
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

    fn browser_tab_urls(&self, title: &str, browser: &str) -> Option<Vec<String>> {
        let snapshot = crate::tabs::load(&crate::tabs::snapshot_path(self.runtime_dir.as_ref()?))?;
        let window = crate::tabs::window_for(&snapshot, title, browser)?;
        Some(window.tabs.iter().map(|tab| tab.url.clone()).collect())
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
        urls: RefCell<Option<Vec<String>>>,
    }

    impl Fake {
        fn new(title: &str) -> Self {
            Self { log: RefCell::new(Vec::new()), title_now: RefCell::new(title.into()), urls: RefCell::new(None) }
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
        fn browser_tab_urls(&self, _: &str, _: &str) -> Option<Vec<String>> {
            self.urls.borrow().clone()
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
            fn browser_tab_urls(&self, _: &str, _: &str) -> Option<Vec<String>> {
                None
            }
        }
        assert!(close(&Empty, 7, &Closes::Tab { title: "T".into() }).unwrap_err().contains("the window is gone"));
    }

    fn whole_window(urls: &[&str]) -> Closes {
        Closes::BrowserWindow { title: "Page - Helium".into(), browser: "Helium".into(), urls: urls.iter().map(|url| url.to_string()).collect() }
    }

    #[test]
    fn a_browser_window_closes_only_while_it_holds_exactly_the_tabs_that_were_sent() {
        let fake = Fake::new("Page - Helium");
        *fake.urls.borrow_mut() = Some(vec!["https://a.example/".into(), "https://b.example/".into()]);
        assert_eq!(close(&fake, 7, &whole_window(&["https://a.example/", "https://b.example/"])).unwrap(), "closed the window here");
        assert_eq!(*fake.log.borrow(), ["close-window 7"]);
    }

    #[test]
    fn a_tab_opened_while_sending_keeps_the_window_open() {
        let fake = Fake::new("Page - Helium");
        *fake.urls.borrow_mut() = Some(vec!["https://a.example/".into(), "https://new.example/".into()]);
        let error = close(&fake, 7, &whole_window(&["https://a.example/"])).unwrap_err();
        assert!(error.contains("tabs changed while sending"), "{error}");
        assert!(fake.log.borrow().is_empty(), "nothing was closed");
    }

    #[test]
    fn a_browser_that_cannot_confirm_its_tabs_keeps_the_window_open() {
        let fake = Fake::new("Page - Helium");
        let error = close(&fake, 7, &whole_window(&["https://a.example/"])).unwrap_err();
        assert!(error.contains("could not confirm"), "{error}");
        assert!(fake.log.borrow().is_empty());
        let moved = Fake::new("Other - Helium");
        *moved.urls.borrow_mut() = Some(vec!["https://a.example/".into()]);
        assert!(close(&moved, 7, &whole_window(&["https://a.example/"])).unwrap_err().contains("no longer shows"));
    }

    #[test]
    fn nothing_closes_when_the_original_is_to_stay() {
        let fake = Fake::new("T");
        let done = close(&fake, 7, &Closes::Nothing { why: "2 tabs are browser pages".into() }).unwrap();
        assert!(done.contains("left it open here (2 tabs are browser pages)"), "{done}");
        assert!(fake.log.borrow().is_empty());
    }
}
