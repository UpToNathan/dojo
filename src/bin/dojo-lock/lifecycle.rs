//! Exit status and the `locked-at` takeover marker.
//!
//! The marker lets a locker restarted after a crash keep counting the lock age
//! from the first lock. It names the compositor instance, so a marker left by a
//! crashed locker under a compositor that has since restarted is ignored.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

/// `EX_TEMPFAIL`: the event loop ended without the model deciding to exit, so
/// the session may still be locked. `Restart=on-failure` restarts on it.
pub const ENDED_UNEXPECTEDLY: u8 = 75;

static EXIT: AtomicU8 = AtomicU8::new(ENDED_UNEXPECTEDLY);

/// Called for the model's `Exit`: unlocked, refused and ended all exit 0.
pub fn exit_cleanly() {
    EXIT.store(0, Ordering::SeqCst);
}

pub fn exit_status() -> u8 {
    EXIT.load(Ordering::SeqCst)
}

pub struct Marker {
    path: PathBuf,
    compositor: String,
}

impl Marker {
    /// `$XDG_RUNTIME_DIR/dojo/locked-at`, for the compositor named by
    /// `NIRI_SOCKET` (unique per instance), else `WAYLAND_DISPLAY`.
    pub fn from_env() -> Option<Marker> {
        let directory = dojo::session::handoff::directory()?;
        let compositor = std::env::var("NIRI_SOCKET")
            .or_else(|_| std::env::var("WAYLAND_DISPLAY"))
            .ok()?;
        Some(Marker {
            path: directory.join(dojo::session::handoff::LOCKED_AT),
            compositor,
        })
    }

    /// When the session was first locked under this compositor, if a marker says so.
    pub fn read(&self) -> Option<i64> {
        parse(&std::fs::read_to_string(&self.path).ok()?, &self.compositor)
    }

    /// Written through a temporary file and renamed into place, so a reader never sees half of it.
    pub fn write(&self, since_unix: i64) -> std::io::Result<()> {
        let directory = self.path.parent().unwrap_or(Path::new("/"));
        std::fs::create_dir_all(directory)?;
        let temporary = directory.join(".locked-at.tmp");
        std::fs::write(&temporary, format(since_unix, &self.compositor))?;
        std::fs::rename(&temporary, &self.path)
    }

    /// Idempotent: a missing marker is already removed.
    pub fn remove(&self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!(
                "dojo-lock: could not remove {}: {error}",
                self.path.display()
            ),
        }
    }
}

fn format(since_unix: i64, compositor: &str) -> String {
    format!("{since_unix} {compositor}\n")
}

fn parse(contents: &str, compositor: &str) -> Option<i64> {
    let (since, owner) = contents.trim_end().split_once(' ')?;
    (owner == compositor).then(|| since.parse().ok())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marker_round_trips_only_for_its_own_compositor() {
        let text = format(1_791_983_100, "/run/user/1000/niri.wayland-1.42.sock");
        assert_eq!(
            parse(&text, "/run/user/1000/niri.wayland-1.42.sock"),
            Some(1_791_983_100)
        );
        assert_eq!(parse(&text, "/run/user/1000/niri.wayland-1.43.sock"), None);
        assert_eq!(parse("", "x"), None);
        assert_eq!(parse("soon x\n", "x"), None);
    }

    #[test]
    fn write_read_and_remove_are_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let marker = Marker {
            path: root.path().join("dojo/locked-at"),
            compositor: "wayland-9".into(),
        };
        assert_eq!(marker.read(), None);
        marker.write(100).unwrap();
        marker.write(200).unwrap();
        assert_eq!(marker.read(), Some(200));
        marker.remove();
        marker.remove();
        assert_eq!(marker.read(), None);
        let names: Vec<_> = std::fs::read_dir(root.path().join("dojo"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(names.is_empty(), "{names:?}");
    }
}
