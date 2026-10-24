//! The Services command: systemd user units, with start, stop and restart.

use std::sync::{Arc, Mutex};

use dojo::launcher_core::rows::{Row, RowAction, RowIcon};
use dojo::launcher_core::services::{self, Action, ServiceUnit};
use nucleo_matcher::Matcher;

use super::loader::{LoadState, Loader};
use super::{Host, Notify, Outcome, Secondary, View};

type List = fn() -> anyhow::Result<Vec<ServiceUnit>>;
type Run = fn(Action, &str) -> anyhow::Result<()>;
type Log = fn(&str) -> anyhow::Result<Vec<String>>;

/// Units the launcher will not stop or restart: the shell that is running it, the session bus and
/// the compositor. Starting them again would not be reached.
fn protected(name: &str) -> bool {
    name.starts_with("app-dojo@")
        || name.starts_with("dbus")
        || name.contains("niri")
        || name.starts_with("graphical-session")
}

#[derive(Clone, Copy)]
struct Backend {
    list: List,
    run: Run,
    log: Log,
}

pub struct ServicesView {
    backend: Backend,
    loader: Option<Loader<Vec<ServiceUnit>>>,
    notify: Option<Notify>,
    /// The units behind the rows last returned, by the rows' `Item` position.
    shown: Vec<ServiceUnit>,
    /// What a background action reported, and whether the list must be read again.
    pending: Arc<Mutex<Option<(String, bool)>>>,
    matcher: Matcher,
}

impl ServicesView {
    pub fn new() -> Self {
        Self::with(Backend {
            list: services::list,
            run: services::run,
            log: services::recent_log,
        })
    }

    fn with(backend: Backend) -> Self {
        Self {
            backend,
            loader: None,
            notify: None,
            shown: Vec::new(),
            pending: Arc::new(Mutex::new(None)),
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
        }
    }

    fn load(&mut self) {
        let Some(notify) = self.notify.clone() else {
            return;
        };
        let list = self.backend.list;
        self.loader = Some(Loader::start(notify, move || {
            list().map_err(|error| format!("Could not list services: {error:#}"))
        }));
    }

    /// Runs `work` on another thread and leaves its message (and whether it changed state) for the
    /// next status refresh.
    fn in_background(&self, work: impl FnOnce() -> (String, bool) + Send + 'static) {
        let pending = self.pending.clone();
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let result = work();
            if let Ok(mut slot) = pending.lock() {
                *slot = Some(result);
            }
            if let Some(notify) = notify {
                notify();
            }
        });
    }

    fn unit_for(&self, row: &Row) -> Option<ServiceUnit> {
        match row.action {
            RowAction::Item(index) => self.shown.get(index).cloned(),
            _ => None,
        }
    }

    fn act(&self, action: Action, unit: &ServiceUnit, doing: &str, done: &str) -> Outcome {
        if matches!(action, Action::Stop | Action::Restart) && protected(&unit.name) {
            return Outcome::Error(format!(
                "{} is part of the desktop session; the launcher will not {} it",
                unit.short_name(),
                if action == Action::Stop {
                    "stop"
                } else {
                    "restart"
                }
            ));
        }
        let (run, name, short) = (self.backend.run, unit.name.clone(), unit.short_name());
        let done = done.to_owned();
        self.in_background(move || match run(action, &name) {
            Ok(()) => (format!("{short} {done}"), true),
            Err(error) => (format!("Could not change {short}: {error:#}"), false),
        });
        Outcome::Notice(format!("{doing} {}…", unit.short_name()))
    }
}

fn glyph(unit: &ServiceUnit) -> &'static str {
    if unit.is_failed() {
        "✖"
    } else if unit.is_active() {
        "●"
    } else {
        "○"
    }
}

impl View for ServicesView {
    fn placeholder(&self) -> &str {
        "Search user services…"
    }

    fn open(&mut self, notify: Notify) {
        self.notify = Some(notify);
        self.load();
    }

    fn rows(&mut self, query: &str) -> Vec<Row> {
        self.shown.clear();
        let Some(loader) = &self.loader else {
            return Vec::new();
        };
        let (matcher, shown) = (&mut self.matcher, &mut self.shown);
        loader.with(|state| match state {
            LoadState::Ready(units) => {
                let order = if query.trim().is_empty() {
                    services::order(units)
                } else {
                    services::search(units, query, matcher)
                };
                order
                    .into_iter()
                    .map(|index| {
                        let unit = &units[index];
                        shown.push(unit.clone());
                        let state = if unit.is_failed() {
                            "failed".to_owned()
                        } else {
                            format!("{} ({})", unit.active, unit.sub)
                        };
                        Row {
                            id: String::new(),
                            title: unit.short_name(),
                            subtitle: if unit.description.is_empty() {
                                state
                            } else {
                                format!("{state} · {}", unit.description)
                            },
                            icon: RowIcon::Glyph(glyph(unit).into()),
                            action: RowAction::Item(shown.len() - 1),
                        }
                    })
                    .collect()
            }
            _ => Vec::new(),
        })
    }

    /// Enter starts a stopped unit and restarts a running or failed one.
    fn activate(&mut self, row: &Row, _query: &str, _host: &Host) -> Outcome {
        let Some(unit) = self.unit_for(row) else {
            return Outcome::Stay;
        };
        if unit.is_active() || unit.is_failed() {
            self.act(Action::Restart, &unit, "Restarting", "restarted")
        } else {
            self.act(Action::Start, &unit, "Starting", "started")
        }
    }

    fn secondary(&mut self, key: Secondary, row: &Row, _host: &Host) -> Option<Outcome> {
        let unit = self.unit_for(row)?;
        match key {
            Secondary::Delete => Some(if unit.is_failed() && !unit.is_active() {
                self.act(Action::ResetFailed, &unit, "Clearing", "cleared")
            } else {
                self.act(Action::Stop, &unit, "Stopping", "stopped")
            }),
            Secondary::Alternate => {
                let (log, name, short) = (self.backend.log, unit.name.clone(), unit.short_name());
                self.in_background(move || {
                    let message = match log(&name) {
                        Ok(lines) => lines.last().map_or_else(
                            || format!("{short}: no log lines"),
                            |line| format!("{short}: {line}"),
                        ),
                        Err(error) => format!("Could not read the log of {short}: {error:#}"),
                    };
                    (message, false)
                });
                Some(Outcome::Notice(format!(
                    "Reading the log of {}…",
                    unit.short_name()
                )))
            }
            Secondary::Pin => None,
        }
    }

    fn hint(&self) -> &str {
        "Shift+Enter log · Ctrl+D stop"
    }

    fn status(&mut self) -> Option<String> {
        let (message, reload) = self.pending.lock().ok()?.take()?;
        if reload {
            self.load();
        }
        Some(message)
    }

    fn empty_text(&self, query: &str) -> String {
        let state = self.loader.as_ref().map(|loader| {
            loader.with(|state| match state {
                LoadState::Loading => Some("Loading services…".to_owned()),
                LoadState::Failed(message) => Some(message.clone()),
                LoadState::Ready(_) => None,
            })
        });
        match state {
            Some(Some(message)) => message,
            _ if query.trim().is_empty() => "No user services found".into(),
            _ => "No matching services".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn unit(name: &str, active: &str, sub: &str, description: &str) -> ServiceUnit {
        ServiceUnit {
            name: name.into(),
            description: description.into(),
            load: "loaded".into(),
            active: active.into(),
            sub: sub.into(),
            file_state: None,
        }
    }

    fn units() -> Vec<ServiceUnit> {
        vec![
            unit(
                "dojo-voice.service",
                "active",
                "running",
                "Dojo agent runtime",
            ),
            unit(
                "clipvault-history.service",
                "failed",
                "failed",
                "Clipboard history",
            ),
            unit("old-thing.service", "inactive", "dead", ""),
            unit(
                "app-dojo@autostart.service",
                "active",
                "running",
                "The shell",
            ),
        ]
    }

    static CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn backend() -> Backend {
        Backend {
            list: || Ok(units()),
            run: |action, name| {
                CALLS.lock().unwrap().push(format!("{action:?} {name}"));
                if name == "broken.service" {
                    anyhow::bail!("Unit not found");
                }
                Ok(())
            },
            log: |name| {
                if name == "dojo-voice.service" {
                    Ok(vec!["first".into(), "model ready".into()])
                } else {
                    Ok(Vec::new())
                }
            },
        }
    }

    fn view() -> ServicesView {
        let mut view = ServicesView::with(backend());
        view.loader = Some(Loader::ready(units()));
        view
    }

    fn wait_status(view: &mut ServicesView) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = view.status() {
                return status;
            }
            assert!(Instant::now() < deadline, "no status arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn units_list_failed_first_with_state_and_description() {
        let mut view = view();
        let rows = view.rows("");
        assert_eq!(rows[0].title, "clipvault-history");
        assert_eq!(rows[0].subtitle, "failed · Clipboard history");
        assert_eq!(rows[0].icon, RowIcon::Glyph("✖".into()));
        let voice = rows.iter().find(|row| row.title == "dojo-voice").unwrap();
        assert_eq!(voice.subtitle, "active (running) · Dojo agent runtime");
        assert_eq!(voice.icon, RowIcon::Glyph("●".into()));
        let idle = rows.iter().find(|row| row.title == "old-thing").unwrap();
        assert_eq!(idle.subtitle, "inactive (dead)");
        assert_eq!(idle.icon, RowIcon::Glyph("○".into()));
        assert_eq!(view.rows("voice")[0].title, "dojo-voice");
        assert!(view.rows("zzzz").is_empty());
    }

    #[test]
    fn enter_restarts_running_and_failed_units_and_starts_stopped_ones() {
        let mut view = view();
        let rows = view.rows("");
        let find = |title: &str| rows.iter().find(|row| row.title == title).unwrap().clone();
        let outcome = view.activate(&find("dojo-voice"), "", &Host::default());
        assert_eq!(outcome, Outcome::Notice("Restarting dojo-voice…".into()));
        assert_eq!(wait_status(&mut view), "dojo-voice restarted");
        view.activate(&find("old-thing"), "", &Host::default());
        assert_eq!(wait_status(&mut view), "old-thing started");
        view.activate(&find("clipvault-history"), "", &Host::default());
        assert_eq!(wait_status(&mut view), "clipvault-history restarted");
        let calls = CALLS.lock().unwrap().clone();
        assert!(calls.contains(&"Restart dojo-voice.service".into()));
        assert!(calls.contains(&"Start old-thing.service".into()));
        assert!(calls.contains(&"Restart clipvault-history.service".into()));
    }

    #[test]
    fn ctrl_d_stops_or_clears_and_shift_enter_shows_the_last_log_line() {
        let mut view = view();
        let rows = view.rows("");
        let find = |title: &str| rows.iter().find(|row| row.title == title).unwrap().clone();
        assert_eq!(
            view.secondary(Secondary::Delete, &find("dojo-voice"), &Host::default()),
            Some(Outcome::Notice("Stopping dojo-voice…".into()))
        );
        assert_eq!(wait_status(&mut view), "dojo-voice stopped");
        assert_eq!(
            view.secondary(
                Secondary::Delete,
                &find("clipvault-history"),
                &Host::default()
            ),
            Some(Outcome::Notice("Clearing clipvault-history…".into()))
        );
        assert_eq!(wait_status(&mut view), "clipvault-history cleared");
        view.secondary(Secondary::Alternate, &find("dojo-voice"), &Host::default());
        assert_eq!(wait_status(&mut view), "dojo-voice: model ready");
        view.secondary(Secondary::Alternate, &find("old-thing"), &Host::default());
        assert_eq!(wait_status(&mut view), "old-thing: no log lines");
        assert_eq!(
            view.secondary(Secondary::Pin, &find("dojo-voice"), &Host::default()),
            None
        );
    }

    #[test]
    fn the_desktop_session_cannot_be_stopped_or_restarted_from_here() {
        let mut view = view();
        let rows = view.rows("autostart");
        let outcome = view.activate(&rows[0], "autostart", &Host::default());
        assert_eq!(
            outcome,
            Outcome::Error("app-dojo@autostart is part of the desktop session; the launcher will not restart it".into())
        );
        let outcome = view.secondary(Secondary::Delete, &rows[0], &Host::default());
        assert!(
            matches!(outcome, Some(Outcome::Error(message)) if message.contains("will not stop"))
        );
        for name in [
            "dbus.service",
            "dbus-broker.service",
            "niri.service",
            "graphical-session.target",
        ] {
            assert!(protected(name), "{name}");
        }
        assert!(!protected("dojo-voice.service"));
    }

    #[test]
    fn a_failing_action_is_reported_in_the_status_line() {
        let mut view = ServicesView::with(backend());
        let broken = unit("broken.service", "inactive", "dead", "");
        view.loader = Some(Loader::ready(vec![broken]));
        let rows = view.rows("");
        view.activate(&rows[0], "", &Host::default());
        assert_eq!(
            wait_status(&mut view),
            "Could not change broken: Unit not found"
        );
    }

    #[test]
    fn an_unloaded_view_says_so() {
        let view = ServicesView::with(backend());
        assert_eq!(view.empty_text(""), "No user services found");
    }
}
