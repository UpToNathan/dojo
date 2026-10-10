use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use anyhow::Context as _;
use nucleo_matcher::Matcher;

use super::catalogue::Catalogue;

pub struct SearchResults {
    pub generation: u64,
    pub catalogue: Arc<Catalogue>,
    pub matches: Vec<usize>,
}

#[derive(Default)]
struct Work {
    generation: u64,
    request: Option<(u64, String)>,
    refresh_catalogue: bool,
    results: Option<SearchResults>,
    error: Option<String>,
    stopped: bool,
}

#[derive(Clone)]
pub struct SearchWorker {
    shared: Arc<(Mutex<Work>, Condvar)>,
    ready: async_channel::Receiver<()>,
}

impl SearchWorker {
    pub fn start() -> anyhow::Result<Self> {
        Self::start_with_discoverer(Catalogue::discover)
    }

    fn start_with_discoverer(
        discover: impl Fn() -> anyhow::Result<Catalogue> + Send + 'static,
    ) -> anyhow::Result<Self> {
        let shared = Arc::new((Mutex::new(Work::default()), Condvar::new()));
        let worker_shared = shared.clone();
        let (notify, ready) = async_channel::bounded(1);
        thread::Builder::new()
            .name("dojo-launcher-search".into())
            .spawn(move || {
                let mut catalogue = None;
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
                        match discover() {
                            Ok(discovered) => catalogue = Some(Arc::new(discovered)),
                            Err(error) => {
                                if let Ok(mut state) = worker_shared.0.lock() {
                                    state.error =
                                        Some(format!("Application discovery failed: {error:#}"));
                                }
                                notify_ready(&notify);
                                continue;
                            }
                        }
                    }
                    let Some(catalogue) = catalogue.as_ref().cloned() else {
                        continue;
                    };
                    let matches = catalogue.search(&query, &mut matcher);
                    let Ok(mut state) = worker_shared.0.lock() else {
                        return;
                    };
                    if state.request.is_none() && !state.stopped {
                        state.error = None;
                        state.results = Some(SearchResults {
                            generation,
                            catalogue,
                            matches,
                        });
                        notify_ready(&notify);
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

    pub fn take_results(&self) -> Result<Option<SearchResults>, String> {
        let mut state = self
            .shared
            .0
            .lock()
            .map_err(|_| "Launcher worker lock poisoned".to_string())?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        Ok(state.results.take())
    }

    pub fn stop(&self) {
        if let Ok(mut state) = self.shared.0.lock() {
            state.stopped = true;
            self.shared.1.notify_one();
        }
    }

    pub async fn wait(&self) -> bool {
        self.ready.recv().await.is_ok()
    }
}

fn notify_ready(sender: &async_channel::Sender<()>) {
    if let Err(error) = sender.try_send(()) {
        if error.is_closed() {
            eprintln!("Dojo launcher result receiver closed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

        assert_eq!(refreshed.matches.len(), 1);
        assert_eq!(
            refreshed.catalogue.applications[refreshed.matches[0]].name,
            "New App"
        );
        worker.stop();
        fs::remove_dir_all(root)?;
        Ok(())
    }

    fn wait_for_results(worker: &SearchWorker, generation: u64) -> anyhow::Result<SearchResults> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(results) = worker.take_results().map_err(anyhow::Error::msg)? {
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
