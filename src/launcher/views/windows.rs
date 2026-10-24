//! The Switch Windows command: list Niri's windows and focus the one you pick.

use dojo::launcher_core::rows::{Row, RowAction, RowIcon};
use dojo::launcher_core::windows::{self, WindowItem};
use nucleo_matcher::Matcher;

use super::loader::{LoadState, Loader};
use super::{Host, Notify, Outcome, View};

type List = fn() -> anyhow::Result<Vec<WindowItem>>;
type Focus = fn(u64) -> anyhow::Result<()>;

pub struct WindowsView {
    loader: Option<Loader<Vec<WindowItem>>>,
    /// The window id each row stands for, by the row's `Item` position.
    shown: Vec<u64>,
    matcher: Matcher,
    list: List,
    focus: Focus,
}

impl WindowsView {
    pub fn new() -> Self {
        Self::with(windows::list, windows::focus)
    }

    fn with(list: List, focus: Focus) -> Self {
        Self {
            loader: None,
            shown: Vec::new(),
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
            list,
            focus,
        }
    }
}

fn icon(item: &WindowItem) -> &'static str {
    if item.is_urgent {
        "❗"
    } else if item.is_focused {
        "▶"
    } else {
        "▣"
    }
}

impl View for WindowsView {
    fn placeholder(&self) -> &str {
        "Search windows…"
    }

    fn open(&mut self, notify: Notify) {
        let list = self.list;
        self.loader = Some(Loader::start(notify, move || {
            list().map_err(|error| format!("Could not list windows: {error:#}"))
        }));
    }

    fn rows(&mut self, query: &str) -> Vec<Row> {
        self.shown.clear();
        let Some(loader) = &self.loader else {
            return Vec::new();
        };
        let matcher = &mut self.matcher;
        let shown = &mut self.shown;
        loader.with(|state| match state {
            LoadState::Ready(items) => windows::rank(items, query, matcher)
                .into_iter()
                .map(|index| {
                    let item = &items[index];
                    shown.push(item.id);
                    let place = match (&item.workspace, item.is_floating) {
                        (Some(workspace), true) => format!("{workspace} · floating"),
                        (Some(workspace), false) => workspace.clone(),
                        (None, _) => "no workspace".into(),
                    };
                    Row {
                        id: String::new(),
                        title: item.display_title().to_owned(),
                        subtitle: format!("{} · {place}", item.app_name()),
                        icon: RowIcon::Glyph(icon(item).into()),
                        action: RowAction::Item(shown.len() - 1),
                    }
                })
                .collect(),
            _ => Vec::new(),
        })
    }

    fn activate(&mut self, row: &Row, _query: &str, _host: &Host) -> Outcome {
        let RowAction::Item(index) = row.action else {
            return Outcome::Stay;
        };
        let Some(&id) = self.shown.get(index) else {
            return Outcome::Stay;
        };
        match (self.focus)(id) {
            Ok(()) => Outcome::Close,
            Err(error) => Outcome::Error(format!("Could not focus the window: {error:#}")),
        }
    }

    fn empty_text(&self, query: &str) -> String {
        let loading = self
            .loader
            .as_ref()
            .is_none_or(|loader| loader.with(|state| matches!(state, LoadState::Loading)));
        if loading {
            return "Loading windows…".into();
        }
        let failure = self.loader.as_ref().and_then(|loader| {
            loader.with(|state| match state {
                LoadState::Failed(message) => Some(message.clone()),
                _ => None,
            })
        });
        match failure {
            Some(message) => message,
            None if query.trim().is_empty() => "No windows are open".into(),
            None => "No windows found".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    const WINDOWS: &str = r#"[
      {"id":1,"title":"Docs - Helium","app_id":"helium","pid":10,"workspace_id":1,"is_focused":false,"is_floating":false,"is_urgent":false,"layout":{"pos_in_scrolling_layout":[1,1],"tile_size":[1.0,1.0],"window_size":[1,1],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]},"focus_timestamp":{"secs":100,"nanos":0}},
      {"id":2,"title":"~/Dojo","app_id":"kitty","pid":11,"workspace_id":1,"is_focused":true,"is_floating":false,"is_urgent":false,"layout":{"pos_in_scrolling_layout":[2,1],"tile_size":[1.0,1.0],"window_size":[1,1],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]},"focus_timestamp":{"secs":200,"nanos":0}},
      {"id":3,"title":"Mail","app_id":"org.gnome.Geary","pid":12,"workspace_id":2,"is_focused":false,"is_floating":true,"is_urgent":true,"layout":{"pos_in_scrolling_layout":null,"tile_size":[1.0,1.0],"window_size":[1,1],"tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]},"focus_timestamp":{"secs":50,"nanos":0}}
    ]"#;
    const WORKSPACES: &str = r#"[
      {"id":1,"idx":1,"name":null,"output":"eDP-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":2},
      {"id":2,"idx":2,"name":"chat","output":"eDP-1","is_urgent":false,"is_active":false,"is_focused":false,"active_window_id":3}
    ]"#;

    fn items() -> Vec<WindowItem> {
        let windows: Vec<niri_ipc::Window> = serde_json::from_str(WINDOWS).unwrap();
        let workspaces: Vec<niri_ipc::Workspace> = serde_json::from_str(WORKSPACES).unwrap();
        windows::items(&windows, &workspaces)
    }

    static FOCUSED: Mutex<Vec<u64>> = Mutex::new(Vec::new());

    fn view() -> WindowsView {
        let mut view = WindowsView::with(
            || Ok(items()),
            |id| {
                if id == 3 {
                    anyhow::bail!("the window closed");
                }
                FOCUSED.lock().unwrap().push(id);
                Ok(())
            },
        );
        view.loader = Some(Loader::ready(items()));
        view
    }

    #[test]
    fn windows_list_most_recently_focused_first_with_app_and_workspace() {
        let mut view = view();
        let rows = view.rows("");
        let titles = rows
            .iter()
            .map(|row| row.title.as_str())
            .collect::<Vec<_>>();
        assert_eq!(titles, ["~/Dojo", "Docs - Helium", "Mail"]);
        assert_eq!(rows[0].subtitle, "kitty · 1");
        assert_eq!(rows[2].subtitle, "geary · chat · floating");
        assert_eq!(rows[0].icon, RowIcon::Glyph("▶".into()));
        assert_eq!(rows[2].icon, RowIcon::Glyph("❗".into()));
    }

    #[test]
    fn a_query_filters_by_title_and_application() {
        let mut view = view();
        assert_eq!(view.rows("hel")[0].title, "Docs - Helium");
        assert_eq!(view.rows("kitty")[0].title, "~/Dojo");
        assert!(view.rows("nothing-like-it").is_empty());
        assert_eq!(view.empty_text("nothing-like-it"), "No windows found");
        assert_eq!(view.empty_text(""), "No windows are open");
    }

    #[test]
    fn activating_a_row_focuses_that_window_and_reports_a_failure() {
        let mut view = view();
        let rows = view.rows("hel");
        assert_eq!(
            view.activate(&rows[0], "hel", &Host::default()),
            Outcome::Close
        );
        assert_eq!(FOCUSED.lock().unwrap().last(), Some(&1));
        let rows = view.rows("mail");
        let outcome = view.activate(&rows[0], "mail", &Host::default());
        assert_eq!(
            outcome,
            Outcome::Error("Could not focus the window: the window closed".into())
        );
    }

    #[test]
    fn a_view_that_never_loaded_says_so() {
        let view = WindowsView::with(|| Ok(Vec::new()), |_| Ok(()));
        assert_eq!(view.empty_text(""), "Loading windows…");
    }
}
