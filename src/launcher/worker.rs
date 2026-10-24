use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use anyhow::Context as _;
use nucleo_matcher::Matcher;

use dojo::launch_history::{self, LaunchHistory};
use dojo::launcher_core::commands::{self, HiddenCommands};
use dojo::launcher_core::inline::{InlineState, fallback_rows, inline_rows};
use dojo::launcher_core::root::{RootInput, root_rows};
use dojo::launcher_core::rows::{Row, RowAction};

use super::catalogue::Catalogue;

#[derive(Clone)]
pub struct SearchResults {
    pub generation: u64,
    pub catalogue: Arc<Catalogue>,
    pub rows: Vec<Row>,
}

#[derive(Default)]
struct Work {
    generation: u64,
    request: Option<(u64, String)>,
    refresh_catalogue: bool,
    results: Option<SearchResults>,
    error: Option<String>,
    stopped: bool,
    listeners: Vec<async_channel::Sender<()>>,
}

pub struct SearchWorker {
    shared: Arc<(Mutex<Work>, Condvar)>,
    ready: async_channel::Receiver<()>,
}

impl Clone for SearchWorker {
    fn clone(&self) -> Self {
        let (notify, ready) = async_channel::bounded(1);
        if let Ok(mut state) = self.shared.0.lock() {
            if !state.stopped {
                if state.results.is_some() || state.error.is_some() {
                    let _ = notify.try_send(());
                }
                state.listeners.push(notify);
            }
        }
        Self {
            shared: self.shared.clone(),
            ready,
        }
    }
}

impl SearchWorker {
    pub fn start() -> anyhow::Result<Self> {
        Self::start_with_discoverer(Catalogue::discover)
    }

    fn start_with_discoverer(
        discover: impl Fn() -> anyhow::Result<Catalogue> + Send + 'static,
    ) -> anyhow::Result<Self> {
        let (notify, ready) = async_channel::bounded(1);
        let shared = Arc::new((
            Mutex::new(Work {
                listeners: vec![notify],
                ..Work::default()
            }),
            Condvar::new(),
        ));
        let worker_shared = shared.clone();
        thread::Builder::new()
            .name("dojo-launcher-search".into())
            .spawn(move || {
                let mut catalogue = None;
                let mut history = LaunchHistory::default();
                let mut hidden = HiddenCommands::default();
                let mut inline = InlineState::tracked();
                let available = commands::available();
                let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
                loop {
                    let (generation, query, refresh_catalogue) = {
                        let Ok(mut state) = worker_shared.0.lock() else {
                            return;
                        };
                        while state.request.is_none() && !state.stopped {
                            state = match worker_shared.1.wait(state) {
                                Ok(state) => state,
                                Err(_) => return,
                            };
                        }
                        if state.stopped {
                            return;
                        }
                        let Some(request) = state.request.take() else {
                            continue;
                        };
                        let refresh_catalogue = std::mem::take(&mut state.refresh_catalogue);
                        (request.0, request.1, refresh_catalogue)
                    };

                    if refresh_catalogue || catalogue.is_none() {
                        // Each reopen refreshes, so launches since the last open count.
                        history = launch_history::default_path().map_or_else(
                            |_| LaunchHistory::default(),
                            |path| LaunchHistory::load(&path),
                        );
                        inline = InlineState::load();
                        hidden = commands::hidden_path().map_or_else(
                            |_| HiddenCommands::default(),
                            |path| HiddenCommands::load(&path),
                        );
                        match discover() {
                            Ok(discovered) => catalogue = Some(Arc::new(discovered)),
                            Err(error) => {
                                if let Ok(mut state) = worker_shared.0.lock() {
                                    state.error =
                                        Some(format!("Application discovery failed: {error:#}"));
                                    notify_ready(&mut state.listeners);
                                }
                                continue;
                            }
                        }
                    }
                    let Some(catalogue) = catalogue.as_ref().cloned() else {
                        continue;
                    };
                    let extras = inline_rows(&query, &inline);
                    let rows = root_rows(
                        RootInput {
                            catalogue: &catalogue,
                            query: &query,
                            history: &history,
                            now_ms: launch_history::now_ms(),
                            commands: &available,
                            hidden: &hidden,
                            inline: extras.leading,
                            trailing: extras.trailing,
                        },
                        &mut matcher,
                    );
                    // With nothing else to offer, a typed query can still be searched on the web.
                    let rows = if rows.is_empty() {
                        fallback_rows(&query, &inline)
                    } else {
                        rows
                    };
                    let Ok(mut state) = worker_shared.0.lock() else {
                        return;
                    };
                    if state.request.is_none() && !state.stopped {
                        state.error = None;
                        state.results = Some(SearchResults {
                            generation,
                            catalogue,
                            rows,
                        });
                        notify_ready(&mut state.listeners);
                    }
                }
            })
            .context("starting launcher search worker")?;
        Ok(Self { shared, ready })
    }

    pub fn query(&self, query: String) -> Result<u64, String> {
        self.submit(query, false)
    }

    pub fn refresh(&self, query: String) -> Result<u64, String> {
        self.submit(query, true)
    }

    fn submit(&self, query: String, refresh_catalogue: bool) -> Result<u64, String> {
        let mut state = self
            .shared
            .0
            .lock()
            .map_err(|_| "Launcher worker lock poisoned".to_string())?;
        if state.stopped {
            return Err("Launcher search stopped".into());
        }
        state.error = None;
        state.generation += 1;
        let generation = state.generation;
        // A single latest-query slot bounds work during fast typing.
        state.request = Some((generation, query));
        state.refresh_catalogue |= refresh_catalogue;
        state.results = None;
        self.shared.1.notify_one();
        Ok(generation)
    }

    pub fn results(&self) -> Result<Option<SearchResults>, String> {
        let state = self
            .shared
            .0
            .lock()
            .map_err(|_| "Launcher worker lock poisoned".to_string())?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        Ok(state.results.clone())
    }

    pub fn stop(&self) {
        if let Ok(mut state) = self.shared.0.lock() {
            state.stopped = true;
            state.listeners.clear();
            self.shared.1.notify_one();
        }
    }

    pub async fn wait(&self) -> bool {
        self.ready.recv().await.is_ok()
    }
}

fn notify_ready(listeners: &mut Vec<async_channel::Sender<()>>) {
    listeners.retain(|sender| {
        !matches!(
            sender.try_send(()),
            Err(async_channel::TrySendError::Closed(_))
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[test]
    fn reopening_delivers_results_to_the_current_launcher() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("app.desktop"),
            "[Desktop Entry]\nType=Application\nName=Visible App\nExec=/bin/true\n",
        )?;
        let root = directory.path().to_path_buf();
        let worker = SearchWorker::start_with_discoverer(move || {
            Catalogue::from_roots(&[root.clone()], &[], &[])
        })?;
        let closing_window = worker.clone();
        let current_window = worker.clone();
        let generation = current_window
            .refresh(String::new())
            .map_err(anyhow::Error::msg)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if closing_window.ready.try_recv().is_ok() {
                break;
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "closing launcher did not receive notification"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let closing_results = closing_window
            .results()
            .map_err(anyhow::Error::msg)?
            .context("closing launcher has no results")?;
        assert_eq!(closing_results.generation, generation);
        // Reading takes the state lock, ensuring the broadcast has completed.
        assert!(
            current_window.ready.try_recv().is_ok(),
            "reopened launcher remains Searching because the closing launcher consumed its wakeup"
        );
        let results = current_window
            .results()
            .map_err(anyhow::Error::msg)?
            .context("current launcher has no results")?;
        assert_eq!(results.generation, generation);
        // The empty search leads with command suggestions; the first application follows.
        assert_eq!(
            results
                .rows
                .iter()
                .find(|row| matches!(row.action, RowAction::Launch(_)))
                .map(|row| row.title.as_str()),
            Some("Visible App")
        );
        worker.stop();
        Ok(())
    }

    #[test]
    fn refresh_picks_up_applications_added_after_worker_start() -> anyhow::Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!(
            "dojo-launcher-refresh-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root)?;
        let discovery_root = root.clone();
        let worker = SearchWorker::start_with_discoverer(move || {
            Catalogue::from_roots(&[discovery_root.clone()], &[], &[])
        })?;

        let initial_generation = worker.refresh(String::new()).map_err(anyhow::Error::msg)?;
        let initial = wait_for_results(&worker, initial_generation)?;
        assert!(initial.catalogue.applications.is_empty());

        fs::write(
            root.join("new-app.desktop"),
            "[Desktop Entry]\nType=Application\nName=New App\nExec=/bin/true\n",
        )?;
        let refreshed_generation = worker
            .refresh("New App".into())
            .map_err(anyhow::Error::msg)?;
        let refreshed = wait_for_results(&worker, refreshed_generation)?;

        assert_eq!(refreshed.rows.len(), 1);
        assert_eq!(refreshed.rows[0].title, "New App");
        worker.stop();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn wait_for_results(worker: &SearchWorker, generation: u64) -> anyhow::Result<SearchResults> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(results) = worker.results().map_err(anyhow::Error::msg)? {
                if results.generation == generation {
                    return Ok(results);
                }
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "search worker did not return results"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn queued_queries_are_bounded_and_generations_survive_reopening() -> anyhow::Result<()> {
        let (_sender, ready) = async_channel::bounded(1);
        let worker = SearchWorker {
            shared: Arc::new((Mutex::new(Work::default()), Condvar::new())),
            ready,
        };
        let first = worker.query("fire".into()).map_err(anyhow::Error::msg)?;
        let second = worker.query("firefox".into()).map_err(anyhow::Error::msg)?;
        assert!(second > first);
        let state = worker
            .shared
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("poisoned worker"))?;
        assert_eq!(state.request, Some((second, "firefox".into())));
        drop(state);
        let reopened = worker.query(String::new()).map_err(anyhow::Error::msg)?;
        assert!(reopened > second);
        worker.stop();
        assert!(worker.query("late".into()).is_err());
        Ok(())
    }
}
