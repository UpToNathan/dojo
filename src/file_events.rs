use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Write as _;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::Context as _;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::inotify;

pub struct Watcher {
    subscribers: Arc<Mutex<Vec<async_channel::Sender<()>>>>,
    stop: UnixStream,
    worker: Option<JoinHandle<()>>,
}

impl Watcher {
    pub fn start(paths: Vec<PathBuf>) -> anyhow::Result<Self> {
        anyhow::ensure!(
            paths.iter().all(|path| path.is_absolute()),
            "watched paths must be absolute"
        );
        let descriptor =
            inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
        let (stop, worker_stop) = UnixStream::pair()?;
        let mut watches = WatchSet::default();
        watches.reconcile(&descriptor, &paths)?;
        let subscribers = Arc::new(Mutex::new(Vec::<async_channel::Sender<()>>::new()));
        let worker_subscribers = Arc::clone(&subscribers);
        let worker = std::thread::Builder::new().name("dojo-file-events".into()).spawn(move || {
            let mut buffer = [MaybeUninit::uninit(); 16 * 1024];
            let mut degraded = false;
            loop {
                let mut descriptors = [PollFd::new(&descriptor, PollFlags::IN), PollFd::new(&worker_stop, PollFlags::IN)];
                let retry = Timespec { tv_sec: 30, tv_nsec: 0 };
                match poll(&mut descriptors, degraded.then_some(&retry)) {
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(error) => {
                        eprintln!("Dojo file event poll failed: {error}");
                        break;
                    }
                }
                if !descriptors[1].revents().is_empty() {
                    break;
                }
                let mut changed = descriptors[0].revents().is_empty() && degraded;
                if descriptors[0].revents().contains(PollFlags::IN) {
                    let mut reader = inotify::Reader::new(&descriptor, &mut buffer);
                    loop {
                        match reader.next() {
                            Ok(event) => {
                                changed |= watches.changed(&event);
                                if reader.is_buffer_empty() {
                                    break;
                                }
                            }
                            Err(rustix::io::Errno::AGAIN) => break,
                            Err(rustix::io::Errno::INTR) => continue,
                            Err(error) => {
                                eprintln!("Dojo file event read failed: {error}");
                                degraded = true;
                                changed = true;
                                break;
                            }
                        }
                    }
                }
                if changed {
                    degraded = match watches.reconcile(&descriptor, &paths) {
                        Ok(()) => false,
                        Err(error) => {
                            eprintln!("Dojo file watches need repair; retrying in 30 seconds: {error:#}");
                            true
                        }
                    };
                    if let Ok(subscribers) = worker_subscribers.lock() {
                        for subscriber in subscribers.iter() {
                            match subscriber.try_send(()) {
                                Ok(()) | Err(async_channel::TrySendError::Full(())) | Err(async_channel::TrySendError::Closed(())) => {}
                            }
                        }
                    }
                }
            }
        }).context("starting file event watcher")?;
        Ok(Self {
            subscribers,
            stop,
            worker: Some(worker),
        })
    }

    pub fn subscribe(&self) -> (async_channel::Sender<()>, async_channel::Receiver<()>) {
        let (sender, receiver) = async_channel::bounded(1);
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(sender.clone());
        }
        match sender.try_send(()) {
            Ok(())
            | Err(async_channel::TrySendError::Full(()))
            | Err(async_channel::TrySendError::Closed(())) => {}
        }
        (sender, receiver)
    }

    pub fn shutdown(&mut self) {
        if let Some(worker) = self.worker.take() {
            if let Err(error) = self.stop.write_all(&[1]) {
                eprintln!("Dojo could not wake its file watcher: {error}");
            }
            if worker.join().is_err() {
                eprintln!("Dojo file watcher panicked during shutdown");
            }
            if let Ok(mut subscribers) = self.subscribers.lock() {
                subscribers.clear();
            }
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Default)]
struct WatchSet {
    directories: BTreeMap<PathBuf, (i32, BTreeSet<OsString>)>,
    invalidated: BTreeSet<i32>,
}

impl WatchSet {
    fn reconcile(
        &mut self,
        descriptor: &rustix::fd::OwnedFd,
        paths: &[PathBuf],
    ) -> anyhow::Result<()> {
        for _ in 0..8 {
            let plan = watch_plan(paths)?;
            self.install(descriptor, plan)?;
            let latest = watch_plan(paths)?;
            if latest.len() == self.directories.len()
                && latest.iter().all(|(directory, names)| {
                    self.directories
                        .get(directory)
                        .is_some_and(|(_, installed)| names == installed)
                })
            {
                return Ok(());
            }
        }
        anyhow::bail!("watched directories kept changing during reconciliation")
    }

    fn install(
        &mut self,
        descriptor: &rustix::fd::OwnedFd,
        plan: BTreeMap<PathBuf, BTreeSet<OsString>>,
    ) -> anyhow::Result<()> {
        let obsolete: Vec<_> = self
            .directories
            .keys()
            .filter(|directory| {
                !plan.contains_key(*directory)
                    || self
                        .directories
                        .get(*directory)
                        .is_some_and(|(watch, _)| self.invalidated.contains(watch))
            })
            .cloned()
            .collect();
        for directory in obsolete {
            if let Some((watch, _)) = self.directories.remove(&directory) {
                match inotify::remove_watch(descriptor, watch) {
                    Ok(()) | Err(rustix::io::Errno::INVAL) => {}
                    Err(error) => return Err(error).context("removing obsolete file watch"),
                }
            }
        }
        for (directory, names) in plan {
            if let Some((_, previous_names)) = self.directories.get_mut(&directory) {
                *previous_names = names;
            } else {
                let flags = inotify::WatchFlags::CLOSE_WRITE
                    | inotify::WatchFlags::MODIFY
                    | inotify::WatchFlags::MOVED_TO
                    | inotify::WatchFlags::MOVED_FROM
                    | inotify::WatchFlags::CREATE
                    | inotify::WatchFlags::DELETE
                    | inotify::WatchFlags::ATTRIB
                    | inotify::WatchFlags::DELETE_SELF
                    | inotify::WatchFlags::MOVE_SELF
                    | inotify::WatchFlags::ONLYDIR;
                let watch = inotify::add_watch(descriptor, &directory, flags)
                    .with_context(|| format!("watching {}", directory.display()))?;
                self.directories.insert(directory, (watch, names));
            }
        }
        self.invalidated.clear();
        Ok(())
    }

    fn changed(&mut self, event: &inotify::Event<'_>) -> bool {
        let flags = event.events();
        if flags.contains(inotify::ReadFlags::QUEUE_OVERFLOW) {
            return true;
        }
        let changed = self.directories.values().any(|(descriptor, names)| {
            *descriptor == event.wd()
                && (flags.intersects(
                    inotify::ReadFlags::DELETE_SELF
                        | inotify::ReadFlags::MOVE_SELF
                        | inotify::ReadFlags::IGNORED,
                ) || event.file_name().is_some_and(|name| {
                    names.contains(std::ffi::OsStr::from_bytes(name.to_bytes()))
                }))
        });
        if flags.contains(inotify::ReadFlags::IGNORED) {
            self.directories
                .retain(|_, (descriptor, _)| *descriptor != event.wd());
        } else if flags.intersects(inotify::ReadFlags::DELETE_SELF | inotify::ReadFlags::MOVE_SELF)
        {
            self.invalidated.insert(event.wd());
        }
        changed
    }
}

fn watch_plan(paths: &[PathBuf]) -> anyhow::Result<BTreeMap<PathBuf, BTreeSet<OsString>>> {
    let mut plan = BTreeMap::new();
    for path in paths {
        plan_path(path, &mut plan)?;
        let mut source = path.clone();
        for _ in 0..16 {
            let target = if let Ok(target) = source.canonicalize() {
                target
            } else if let Some((link, target)) = source.ancestors().find_map(|ancestor| {
                std::fs::read_link(ancestor)
                    .ok()
                    .map(|target| (ancestor, target))
            }) {
                let target = if target.is_absolute() {
                    target
                } else {
                    link.parent()
                        .context("relative symlink needs a parent")?
                        .join(target)
                };
                target.join(source.strip_prefix(link)?)
            } else {
                break;
            };
            if target == source {
                break;
            }
            plan_path(&target, &mut plan)?;
            source = target;
        }
    }
    Ok(plan)
}

fn plan_path(path: &Path, plan: &mut BTreeMap<PathBuf, BTreeSet<OsString>>) -> anyhow::Result<()> {
    let mut directory = path.parent().context("watched path needs a parent")?;
    while !directory.is_dir() {
        directory = directory
            .parent()
            .context("watched path has no existing ancestor")?;
    }
    let name = path
        .strip_prefix(directory)?
        .components()
        .next()
        .context("watched path needs a filename")?
        .as_os_str()
        .to_owned();
    plan.entry(directory.canonicalize()?)
        .or_default()
        .insert(name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait(receiver: &async_channel::Receiver<()>) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if receiver.try_recv().is_ok() {
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "file change notification timed out"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn native_events_follow_writes_and_atomic_renames() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "initial")?;
        let mut watcher = Watcher::start(vec![path.clone()])?;
        let (_, receiver) = watcher.subscribe();
        wait(&receiver)?;
        std::fs::write(&path, "changed")?;
        wait(&receiver)?;
        assert_eq!(std::fs::read_to_string(&path)?, "changed");
        let replacement = directory.path().join("new-config");
        std::fs::write(&replacement, "replacement")?;
        std::fs::rename(replacement, &path)?;
        wait(&receiver)?;
        assert_eq!(std::fs::read_to_string(&path)?, "replacement");
        watcher.shutdown();
        while receiver.try_recv().is_ok() {}
        assert!(receiver.is_closed());
        Ok(())
    }

    #[test]
    fn native_events_ignore_apply_lock_and_follow_config_symlink_targets() -> anyhow::Result<()> {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir()?;
        let target_directory = directory.path().join("tracked");
        std::fs::create_dir(&target_directory)?;
        let target = target_directory.join("config.toml");
        std::fs::write(&target, "initial")?;
        let path = directory.path().join("config.toml");
        symlink(&target, &path)?;
        let mut watcher = Watcher::start(vec![path.clone()])?;
        let (_, receiver) = watcher.subscribe();
        wait(&receiver)?;
        std::fs::write(directory.path().join("apply.lock"), "ignored")?;
        std::fs::write(target_directory.join("unrelated"), "ignored")?;
        std::thread::sleep(Duration::from_millis(50));
        assert!(receiver.is_empty());
        std::fs::write(&target, "target changed")?;
        wait(&receiver)?;
        assert_eq!(std::fs::read_to_string(&path)?, "target changed");
        watcher.shutdown();
        Ok(())
    }

    #[test]
    fn native_events_follow_dangling_symlinks_and_replaced_directories() -> anyhow::Result<()> {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir()?;
        let target_directory = directory.path().join("tracked");
        let target = target_directory.join("config.toml");
        let path = directory.path().join("config.toml");
        symlink(&target, &path)?;
        let mut watcher = Watcher::start(vec![path.clone()])?;
        let (_, receiver) = watcher.subscribe();
        wait(&receiver)?;
        std::fs::create_dir(&target_directory)?;
        std::fs::write(&target, "created")?;
        wait(&receiver)?;
        assert_eq!(std::fs::read_to_string(&path)?, "created");
        std::thread::sleep(Duration::from_millis(20));
        while receiver.try_recv().is_ok() {}
        std::fs::rename(&target_directory, directory.path().join("old"))?;
        std::fs::create_dir(&target_directory)?;
        std::fs::write(&target, "replacement directory")?;
        wait(&receiver)?;
        std::thread::sleep(Duration::from_millis(20));
        while receiver.try_recv().is_ok() {}
        std::fs::write(&target, "updated replacement")?;
        wait(&receiver)?;
        assert_eq!(std::fs::read_to_string(&path)?, "updated replacement");
        watcher.shutdown();
        Ok(())
    }

    #[test]
    fn native_events_rearm_missing_parents_and_symlink_generations() -> anyhow::Result<()> {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("missing/theme");
        let current = root.join("current");
        let path = current.join("dojo.json");
        let mut watcher = Watcher::start(vec![current.clone(), path.clone()])?;
        let (_, receiver) = watcher.subscribe();
        wait(&receiver)?;
        std::fs::create_dir_all(&root)?;
        let first = root.join("one");
        std::fs::create_dir(&first)?;
        std::fs::write(first.join("dojo.json"), "first")?;
        symlink(&first, &current)?;
        wait(&receiver)?;
        std::thread::sleep(Duration::from_millis(20));
        while receiver.try_recv().is_ok() {}
        std::fs::write(first.join("dojo.json"), "updated")?;
        wait(&receiver)?;
        assert_eq!(std::fs::read_to_string(&path)?, "updated");
        let second = root.join("two");
        std::fs::create_dir(&second)?;
        std::fs::write(second.join("dojo.json"), "second")?;
        symlink(&second, root.join("next"))?;
        std::fs::rename(root.join("next"), &current)?;
        wait(&receiver)?;
        std::thread::sleep(Duration::from_millis(20));
        while receiver.try_recv().is_ok() {}
        std::fs::write(second.join("dojo.json"), "new update")?;
        wait(&receiver)?;
        assert_eq!(std::fs::read_to_string(&path)?, "new update");
        std::thread::sleep(Duration::from_millis(20));
        while receiver.try_recv().is_ok() {}
        std::fs::write(first.join("dojo.json"), "obsolete generation")?;
        std::thread::sleep(Duration::from_millis(50));
        assert!(receiver.is_empty());
        std::fs::write(second.join("dojo.json"), "current generation")?;
        wait(&receiver)?;
        assert_eq!(std::fs::read_to_string(&path)?, "current generation");
        watcher.shutdown();
        Ok(())
    }
}
