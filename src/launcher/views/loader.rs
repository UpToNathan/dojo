//! Data a view loads off the UI thread.

use std::sync::{Arc, Mutex};

use super::Notify;

pub enum LoadState<T> {
    Loading,
    Ready(T),
    Failed(String),
}

/// A value being produced on another thread. The view reads it when asked for rows; the launcher
/// is told through `notify` when it arrives.
pub struct Loader<T> {
    shared: Arc<Mutex<LoadState<T>>>,
}

impl<T: Send + 'static> Loader<T> {
    pub fn start(
        notify: Notify,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Self {
        let shared = Arc::new(Mutex::new(LoadState::Loading));
        let target = shared.clone();
        std::thread::spawn(move || {
            let result = work();
            if let Ok(mut state) = target.lock() {
                *state = match result {
                    Ok(value) => LoadState::Ready(value),
                    Err(message) => LoadState::Failed(message),
                };
            }
            notify();
        });
        Self { shared }
    }

    /// A loader that already has its value.
    #[cfg(test)]
    pub fn ready(value: T) -> Self {
        Self {
            shared: Arc::new(Mutex::new(LoadState::Ready(value))),
        }
    }

    pub fn with<R>(&self, read: impl FnOnce(&mut LoadState<T>) -> R) -> R {
        let mut state = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        read(&mut state)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    fn wait_for(loader: &Loader<u32>) -> Option<u32> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match loader.with(|state| match state {
                LoadState::Ready(value) => Some(Some(*value)),
                LoadState::Failed(_) => Some(None),
                LoadState::Loading => None,
            }) {
                Some(done) => return done,
                None => {
                    assert!(Instant::now() < deadline, "the loader never finished");
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }

    #[test]
    fn a_loader_notifies_once_when_its_value_arrives() {
        let notified = Arc::new(AtomicUsize::new(0));
        let counter = notified.clone();
        let loader = Loader::start(
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
            || Ok(7),
        );
        assert_eq!(wait_for(&loader), Some(7));
        let deadline = Instant::now() + Duration::from_secs(5);
        while notified.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(notified.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failure_is_kept_and_a_ready_loader_has_its_value() {
        let loader: Loader<u32> = Loader::start(Arc::new(|| {}), || Err("no socket".into()));
        assert_eq!(wait_for(&loader), None);
        loader.with(|state| match state {
            LoadState::Failed(message) => assert_eq!(message, "no socket"),
            _ => panic!("expected a failure"),
        });
        let ready = Loader::ready(3u32);
        assert_eq!(wait_for(&ready), Some(3));
    }
}
