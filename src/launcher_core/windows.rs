//! 05 Niri windows for the switcher. See .scratch/launcher-features/issues/ for the ticket.
//!
//! Pure logic lives here; the launcher wires it in. Keep I/O behind small functions so the rest is testable.
//!
//! [`list`] asks Niri for its windows and workspaces over the IPC socket. [`items`] turns the two
//! replies into [`WindowItem`]s and [`rank`] orders them for a query. Nothing is hidden: floating,
//! urgent, never-focused and untitled windows all appear, and the focused window is an ordinary
//! entry (the first one in the empty list, because it is the most recently focused).

use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use niri_ipc::socket::Socket;
use niri_ipc::{Action, Reply, Request, Response, Window, Workspace};
use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32String};

/// Same budget as `src/shell.rs` gives its Niri queries.
const NIRI_QUERY_TIMEOUT: Duration = Duration::from_millis(250);

/// One Niri window as the switcher shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowItem {
    /// Niri's window id, which `focus_request` takes. Stable while the window is open.
    pub id: u64,
    /// Window title, empty when the application set none.
    pub title: String,
    /// Wayland application id, empty when the application set none.
    pub app_id: String,
    /// Workspace name, or its index on its output when it has no name. `None` when Niri reports
    /// the window on no workspace.
    pub workspace: Option<String>,
    pub workspace_id: Option<u64>,
    /// Output the workspace is on.
    pub output: Option<String>,
    pub is_focused: bool,
    pub is_floating: bool,
    pub is_urgent: bool,
    pub pid: Option<i32>,
    /// When the window was last focused, on Niri's monotonic clock. Only comparable with other
    /// windows' values from the same reply.
    pub focused_at: Option<Duration>,
    /// The last dot-separated part of the application id, lowercased (`firefox` for
    /// `org.mozilla.firefox`), which is what people type.
    app_name: String,
}

impl WindowItem {
    /// The title, or the application id, or a placeholder; never empty.
    pub fn display_title(&self) -> &str {
        if !self.title.is_empty() {
            &self.title
        } else if !self.app_id.is_empty() {
            &self.app_id
        } else {
            "Untitled window"
        }
    }

    /// Short name of the application for the row's subtitle: the last part of its id.
    pub fn app_name(&self) -> &str {
        &self.app_name
    }
}

fn app_name(app_id: &str) -> String {
    app_id
        .rsplit('.')
        .find(|part| !part.is_empty())
        .unwrap_or(app_id)
        .to_lowercase()
}

fn workspace_label(workspace: &Workspace) -> String {
    workspace
        .name
        .as_ref()
        .filter(|name| !name.is_empty())
        .cloned()
        .unwrap_or_else(|| workspace.idx.to_string())
}

/// Builds the switcher's items from Niri's windows and workspaces. The order is Niri's; use
/// [`rank`] to order them.
pub fn items(windows: &[Window], workspaces: &[Workspace]) -> Vec<WindowItem> {
    windows
        .iter()
        .map(|window| {
            let workspace = window
                .workspace_id
                .and_then(|id| workspaces.iter().find(|workspace| workspace.id == id));
            let app_id = window.app_id.clone().unwrap_or_default();
            WindowItem {
                id: window.id,
                title: window.title.clone().unwrap_or_default(),
                app_name: app_name(&app_id),
                app_id,
                workspace: workspace.map(workspace_label),
                workspace_id: window.workspace_id,
                output: workspace.and_then(|workspace| workspace.output.clone()),
                is_focused: window.is_focused,
                is_floating: window.is_floating,
                is_urgent: window.is_urgent,
                pid: window.pid,
                focused_at: window
                    .focus_timestamp
                    .map(|stamp| Duration::new(stamp.secs, stamp.nanos.min(999_999_999))),
            }
        })
        .collect()
}

/// Most recently focused first. A window Niri has never marked focused comes after every window
/// that has been, and the currently focused window counts as the newest even if its timestamp has
/// not caught up with Niri's debounce. Remaining ties fall back to the window id so the order is
/// deterministic.
fn recency_order(left: &WindowItem, right: &WindowItem) -> std::cmp::Ordering {
    right
        .is_focused
        .cmp(&left.is_focused)
        .then_with(|| right.focused_at.cmp(&left.focused_at))
        .then_with(|| left.id.cmp(&right.id))
}

/// Windows matching `query`, best first, as indices into `items`.
///
/// An empty query lists every window, most recently focused first. Otherwise matches fall into
/// tiers: the query is exactly the title, application id or application name; then the start of
/// one of them; then a fuzzy match on one of them; then a fuzzy match across all of them together
/// (so `firefox github` finds a Firefox window titled GitHub). Within a tier the more recently
/// focused window comes first, then the better fuzzy score.
pub fn rank(items: &[WindowItem], query: &str, matcher: &mut Matcher) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        let mut order = (0..items.len()).collect::<Vec<_>>();
        order.sort_by(|&left, &right| recency_order(&items[left], &items[right]));
        return order;
    }
    let pattern = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let atom = |kind| {
        Atom::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            kind,
            false,
        )
    };
    let exact = atom(AtomKind::Exact);
    let prefix = atom(AtomKind::Prefix);

    let mut scored = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let fields = [
                Utf32String::from(item.title.as_str()),
                Utf32String::from(item.app_id.as_str()),
                Utf32String::from(item.app_name.as_str()),
            ];
            let present = |field: &&Utf32String| field.len() > 0;
            if fields
                .iter()
                .filter(present)
                .any(|field| exact.score(field.slice(..), matcher).is_some())
            {
                return Some((index, 0, u32::MAX));
            }
            if fields
                .iter()
                .filter(present)
                .any(|field| prefix.score(field.slice(..), matcher).is_some())
            {
                return Some((index, 1, u32::MAX));
            }
            let best_field = fields
                .iter()
                .filter(present)
                .filter_map(|field| pattern.score(field.slice(..), matcher))
                .max();
            if let Some(score) = best_field {
                return Some((index, 2, score));
            }
            let combined = Utf32String::from(
                format!("{} {} {}", item.app_name, item.app_id, item.title).as_str(),
            );
            pattern
                .score(combined.slice(..), matcher)
                .map(|score| (index, 3, score))
        })
        .collect::<Vec<_>>();
    scored.sort_by(
        |(left, left_tier, left_score), (right, right_tier, right_score)| {
            left_tier
                .cmp(right_tier)
                .then_with(|| recency_order(&items[*left], &items[*right]))
                .then_with(|| right_score.cmp(left_score))
        },
    );
    scored.into_iter().map(|(index, ..)| index).collect()
}

/// The Niri request that focuses a window: `Action::FocusWindow`.
pub fn focus_request(id: u64) -> Request {
    Request::Action(Action::FocusWindow { id })
}

fn connect(path: Option<&Path>) -> Result<Socket> {
    let socket = match path {
        Some(path) => Socket::connect_to(path),
        None => Socket::connect(),
    }
    .context("connecting to the Niri socket")?;
    socket
        .set_read_timeout(Some(NIRI_QUERY_TIMEOUT))
        .context("setting the Niri read timeout")?;
    socket
        .set_write_timeout(Some(NIRI_QUERY_TIMEOUT))
        .context("setting the Niri write timeout")?;
    Ok(socket)
}

fn send(socket: &mut Socket, request: Request) -> Result<Response> {
    let reply: Reply = socket.send(request).context("talking to Niri")?;
    reply.map_err(|message| anyhow::anyhow!("Niri refused the request: {message}"))
}

/// The windows on the Niri this process runs under (`NIRI_SOCKET`), unordered.
pub fn list() -> Result<Vec<WindowItem>> {
    list_on(None)
}

fn list_on(path: Option<&Path>) -> Result<Vec<WindowItem>> {
    let mut socket = connect(path)?;
    let Response::Windows(windows) = send(&mut socket, Request::Windows)? else {
        bail!("Niri answered the window query with something else");
    };
    // Workspace names are a nicety: a failed lookup leaves the workspace column empty rather
    // than losing the switcher.
    let workspaces = match send(&mut socket, Request::Workspaces) {
        Ok(Response::Workspaces(workspaces)) => workspaces,
        _ => Vec::new(),
    };
    Ok(items(&windows, &workspaces))
}

/// Focuses a window through Niri. The id may have closed since [`list`]; Niri then refuses.
pub fn focus(id: u64) -> Result<()> {
    focus_on(None, id)
}

fn focus_on(path: Option<&Path>, id: u64) -> Result<()> {
    let mut socket = connect(path)?;
    match send(&mut socket, focus_request(id))? {
        Response::Handled => Ok(()),
        _ => bail!("Niri answered the focus request with something else"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use niri_ipc::{Timestamp, WindowLayout};
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::os::unix::net::UnixListener;

    fn layout() -> WindowLayout {
        WindowLayout {
            pos_in_scrolling_layout: None,
            tile_size: (0.0, 0.0),
            window_size: (0, 0),
            tile_pos_in_workspace_view: None,
            window_offset_in_tile: (0.0, 0.0),
        }
    }

    fn window(id: u64, app_id: &str, title: &str, focus_secs: Option<u64>) -> Window {
        Window {
            id,
            title: (!title.is_empty()).then(|| title.to_owned()),
            app_id: (!app_id.is_empty()).then(|| app_id.to_owned()),
            pid: Some(1000 + id as i32),
            workspace_id: Some(1),
            is_focused: false,
            is_floating: false,
            is_urgent: false,
            layout: layout(),
            focus_timestamp: focus_secs.map(|secs| Timestamp { secs, nanos: 0 }),
        }
    }

    fn workspace(id: u64, idx: u8, name: Option<&str>, output: &str) -> Workspace {
        Workspace {
            id,
            idx,
            name: name.map(str::to_owned),
            output: Some(output.to_owned()),
            is_urgent: false,
            is_active: true,
            is_focused: false,
            active_window_id: None,
        }
    }

    fn matcher() -> Matcher {
        Matcher::new(nucleo_matcher::Config::DEFAULT)
    }

    fn ranked_ids(windows: &[Window], query: &str) -> Vec<u64> {
        let built = items(windows, &[workspace(1, 1, None, "DP-1")]);
        rank(&built, query, &mut matcher())
            .into_iter()
            .map(|index| built[index].id)
            .collect()
    }

    #[test]
    fn items_carry_titles_workspaces_and_flags() {
        let mut focused = window(7, "org.mozilla.firefox", "GitHub", Some(50));
        focused.is_focused = true;
        focused.is_floating = true;
        focused.is_urgent = true;
        let mut elsewhere = window(8, "", "", None);
        elsewhere.workspace_id = Some(2);
        let mut orphan = window(9, "kitty", "", None);
        orphan.workspace_id = None;
        let built = items(
            &[focused, elsewhere, orphan],
            &[
                workspace(1, 3, None, "DP-1"),
                workspace(2, 1, Some("chat"), "HDMI-A-1"),
            ],
        );

        let first = &built[0];
        assert_eq!(first.id, 7);
        assert_eq!(first.title, "GitHub");
        assert_eq!(first.app_id, "org.mozilla.firefox");
        assert_eq!(first.app_name(), "firefox");
        assert_eq!(first.workspace.as_deref(), Some("3"));
        assert_eq!(first.output.as_deref(), Some("DP-1"));
        assert!(first.is_focused && first.is_floating && first.is_urgent);
        assert_eq!(first.focused_at, Some(Duration::from_secs(50)));

        let second = &built[1];
        assert_eq!(second.workspace.as_deref(), Some("chat"));
        assert_eq!(second.output.as_deref(), Some("HDMI-A-1"));
        assert_eq!(second.display_title(), "Untitled window");
        assert_eq!(second.focused_at, None);

        let third = &built[2];
        assert_eq!(third.workspace, None);
        assert_eq!(third.workspace_id, None);
        assert_eq!(third.display_title(), "kitty");
    }

    #[test]
    fn an_unknown_workspace_id_leaves_the_label_empty_but_keeps_the_window() {
        let built = items(&[window(1, "a", "b", None)], &[]);
        assert_eq!(built.len(), 1);
        assert_eq!(built[0].workspace, None);
        assert_eq!(built[0].workspace_id, Some(1));
    }

    #[test]
    fn an_empty_query_lists_every_window_most_recently_focused_first() {
        let mut current = window(3, "kitty", "shell", Some(10));
        current.is_focused = true;
        let windows = [
            window(1, "firefox", "tab", Some(40)),
            window(2, "code", "main.rs", Some(90)),
            current,
            window(4, "mpv", "movie", None),
            window(5, "foot", "", None),
        ];
        // The focused window leads even though its timestamp is older (the debounce), then by
        // timestamp, then the never-focused windows by id.
        assert_eq!(ranked_ids(&windows, ""), vec![3, 2, 1, 4, 5]);
        assert_eq!(ranked_ids(&windows, "   "), vec![3, 2, 1, 4, 5]);
    }

    #[test]
    fn fractional_timestamps_order_windows_within_one_second() {
        let mut older = window(1, "a", "x", Some(5));
        let mut newer = window(2, "b", "y", Some(5));
        older.focus_timestamp = Some(Timestamp {
            secs: 5,
            nanos: 100,
        });
        newer.focus_timestamp = Some(Timestamp {
            secs: 5,
            nanos: 200,
        });
        assert_eq!(ranked_ids(&[older, newer], ""), vec![2, 1]);
    }

    #[test]
    fn search_matches_titles_app_ids_and_the_short_app_name() {
        let windows = [
            window(1, "org.mozilla.firefox", "Dojo issues", Some(10)),
            window(2, "kitty", "nvim launcher.rs", Some(20)),
            window(3, "mpv", "Movie night", Some(30)),
        ];
        assert_eq!(ranked_ids(&windows, "movie"), vec![3]);
        assert_eq!(ranked_ids(&windows, "firefox"), vec![1]);
        assert_eq!(ranked_ids(&windows, "mozilla"), vec![1]);
        assert_eq!(ranked_ids(&windows, "launcher"), vec![2]);
        assert_eq!(ranked_ids(&windows, "zzzz"), Vec::<u64>::new());
    }

    #[test]
    fn exact_beats_prefix_beats_fuzzy_regardless_of_recency() {
        let windows = [
            window(1, "x", "my kitty notes", Some(300)),
            window(2, "x", "kitten tools", Some(200)),
            window(3, "kitty", "terminal", Some(100)),
            window(4, "x", "k i t t y", Some(400)),
        ];
        // Window 3 is the exact application id; the other fuzzy matches follow it even though
        // two of them were focused more recently.
        let order = ranked_ids(&windows, "kitty");
        assert_eq!(order[0], 3);
        assert!(!order.contains(&2));
        let prefix_windows = [
            window(1, "x", "notes on kitty", Some(300)),
            window(2, "x", "kitty config", Some(100)),
        ];
        // Prefix wins even though the fuzzy match is more recent.
        assert_eq!(ranked_ids(&prefix_windows, "kitty"), vec![2, 1]);
    }

    #[test]
    fn ties_in_a_tier_go_to_the_more_recently_focused_window() {
        let windows = [
            window(1, "firefox", "Docs", Some(10)),
            window(2, "firefox", "Mail", Some(30)),
            window(3, "firefox", "News", Some(20)),
        ];
        assert_eq!(ranked_ids(&windows, "firefox"), vec![2, 3, 1]);
    }

    #[test]
    fn a_query_can_span_app_and_title() {
        let windows = [
            window(1, "org.mozilla.firefox", "GitHub", Some(10)),
            window(2, "org.mozilla.firefox", "Inbox", Some(20)),
            window(3, "kitty", "GitHub CLI", Some(30)),
        ];
        assert_eq!(ranked_ids(&windows, "firefox github"), vec![1]);
    }

    #[test]
    fn matching_is_case_insensitive_for_lowercase_queries() {
        let windows = [window(1, "Slack", "General | Team", Some(1))];
        assert_eq!(ranked_ids(&windows, "slack"), vec![1]);
        assert_eq!(ranked_ids(&windows, "general"), vec![1]);
    }

    #[test]
    fn windows_without_a_title_or_app_id_are_still_listed_and_do_not_break_search() {
        let windows = [window(1, "", "", None), window(2, "foot", "", Some(5))];
        assert_eq!(ranked_ids(&windows, ""), vec![2, 1]);
        assert_eq!(ranked_ids(&windows, "foot"), vec![2]);
    }

    #[test]
    fn focus_request_is_the_focus_window_action() {
        assert!(matches!(
            focus_request(42),
            Request::Action(Action::FocusWindow { id: 42 })
        ));
        assert_eq!(
            serde_json::to_string(&focus_request(42)).unwrap(),
            r#"{"Action":{"FocusWindow":{"id":42}}}"#
        );
    }

    /// Serves Niri's protocol for one connection: one reply per request line.
    fn fake_niri(
        directory: &Path,
        replies: impl Fn(Request) -> Reply + Send + 'static,
    ) -> (std::path::PathBuf, std::thread::JoinHandle<()>) {
        let path = directory.join("niri.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let request: Request = serde_json::from_str(&line).unwrap();
                let mut reply = serde_json::to_vec(&replies(request)).unwrap();
                reply.push(b'\n');
                reader.get_mut().write_all(&reply).unwrap();
            }
        });
        (path, server)
    }

    #[test]
    fn list_reads_windows_and_workspaces_over_the_socket() {
        let directory = tempfile::tempdir().unwrap();
        let (path, server) = fake_niri(directory.path(), |request| match request {
            Request::Windows => Ok(Response::Windows(vec![window(
                5,
                "kitty",
                "shell",
                Some(1),
            )])),
            Request::Workspaces => Ok(Response::Workspaces(vec![workspace(1, 2, None, "DP-1")])),
            other => panic!("unexpected request {other:?}"),
        });
        let listed = list_on(Some(&path)).unwrap();
        server.join().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, 5);
        assert_eq!(listed[0].workspace.as_deref(), Some("2"));
    }

    #[test]
    fn list_survives_a_failed_workspace_query() {
        let directory = tempfile::tempdir().unwrap();
        let (path, server) = fake_niri(directory.path(), |request| match request {
            Request::Windows => Ok(Response::Windows(vec![window(5, "kitty", "shell", None)])),
            _ => Err("no workspaces today".to_owned()),
        });
        let listed = list_on(Some(&path)).unwrap();
        server.join().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].workspace, None);
    }

    #[test]
    fn focus_sends_the_action_and_reports_a_refusal() {
        let directory = tempfile::tempdir().unwrap();
        let (path, server) = fake_niri(directory.path(), |request| match request {
            Request::Action(Action::FocusWindow { id: 1 }) => Ok(Response::Handled),
            Request::Action(Action::FocusWindow { .. }) => Err("no such window".to_owned()),
            other => panic!("unexpected request {other:?}"),
        });
        // Each call opens its own connection, but the fake serves one, so check both replies on
        // one socket through the shared helper.
        let mut socket = connect(Some(&path)).unwrap();
        assert!(matches!(
            send(&mut socket, focus_request(1)).unwrap(),
            Response::Handled
        ));
        let error = send(&mut socket, focus_request(2)).unwrap_err();
        assert!(error.to_string().contains("no such window"));
        drop(socket);
        server.join().unwrap();
    }

    #[test]
    fn focus_on_a_socket_maps_handled_to_ok() {
        let directory = tempfile::tempdir().unwrap();
        let (path, server) = fake_niri(directory.path(), |_| Ok(Response::Handled));
        focus_on(Some(&path), 9).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn a_missing_socket_is_an_error_not_a_panic() {
        let directory = tempfile::tempdir().unwrap();
        assert!(list_on(Some(&directory.path().join("absent.sock"))).is_err());
    }

    /// Read-only: lists the real session's windows when running under Niri.
    #[test]
    fn live_list_smoke() {
        if std::env::var_os(niri_ipc::socket::SOCKET_PATH_ENV).is_none() {
            eprintln!("skipped: not running under Niri");
            return;
        }
        let Ok(listed) = list() else {
            eprintln!("skipped: the Niri socket is not reachable");
            return;
        };
        let mut ids = listed.iter().map(|item| item.id).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), listed.len());
        assert!(listed.iter().filter(|item| item.is_focused).count() <= 1);
        let order = rank(&listed, "", &mut matcher());
        assert_eq!(order.len(), listed.len());
    }
}
