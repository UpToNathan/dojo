//! Files the shell and the locker pass each other under `$XDG_RUNTIME_DIR/dojo/`.
//!
//! `locked-at` is the locker's: written when the compositor confirms the lock
//! and removed when it ends. The shell watches it to drop its cover.
//! `lock-handoff` is the shell's: the farewell the locker plays after taking the
//! lock. `overlay-ready` is the shell's too: its pid, present while it holds a
//! parked closing over the lock, so the locker can leave the unlock opening to it.
//! All are small files renamed into place, so a reader never sees half.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::history;

pub const LOCKED_AT: &str = "locked-at";
pub const LOCK_HANDOFF: &str = "lock-handoff";
pub const OVERLAY_READY: &str = "overlay-ready";
/// What the shell's executable is called, whatever directory it was installed in.
const SHELL_EXE: &str = "dojo";

/// A handoff older than this was left by a lock that never started a locker.
const HANDOFF_FRESH: Duration = Duration::from_secs(10);
/// One short line. Anything longer is not a farewell.
const FAREWELL_MAX_CHARS: usize = 120;

/// `$XDG_RUNTIME_DIR/dojo`, for an absolute runtime directory only.
pub fn directory() -> Option<PathBuf> {
    directory_in(std::env::var_os("XDG_RUNTIME_DIR")?.into())
}

pub fn directory_in(runtime: PathBuf) -> Option<PathBuf> {
    runtime.is_absolute().then(|| runtime.join("dojo"))
}

/// The line in a handoff file, if it holds a usable one.
fn parse(contents: &str) -> Option<String> {
    history::parse(contents).filter(|line| line.chars().count() <= FAREWELL_MAX_CHARS)
}

/// Leaves `line` for the locker, through a temporary file renamed into place.
pub fn write_farewell(directory: &Path, line: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(history::format(line).as_bytes())?;
    file.persist(directory.join(LOCK_HANDOFF))
        .map(drop)
        .map_err(|error| error.error)
}

/// Reads and deletes the handoff. Always deletes, so a switched-off or stale
/// farewell cannot turn up at a later lock. `None` when the file is missing,
/// older than ten seconds at `now`, unusable, or `enabled` is false.
pub fn take_farewell(directory: &Path, enabled: bool, now: SystemTime) -> Option<String> {
    let path = directory.join(LOCK_HANDOFF);
    let modified = std::fs::metadata(&path).and_then(|meta| meta.modified()).ok();
    let contents = std::fs::read_to_string(&path).ok();
    if let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("dojo: could not remove {}: {error}", path.display());
    }
    let fresh = modified.is_some_and(|modified| {
        now.duration_since(modified)
            .map_or(true, |age| age <= HANDOFF_FRESH)
    });
    (enabled && fresh).then(|| parse(&contents?)).flatten()
}

/// Whether `locked-at` says the lock was taken at or after `since`.
/// Timestamps are coarse, so a few milliseconds of slack count as after.
pub fn lock_taken_since(directory: &Path, since: SystemTime) -> bool {
    const SLACK: Duration = Duration::from_millis(50);
    std::fs::metadata(directory.join(LOCKED_AT))
        .and_then(|meta| meta.modified())
        .is_ok_and(|modified| modified + SLACK >= since)
}

/// Records that the shell with `pid` holds a parked overlay.
pub fn write_overlay_ready(directory: &Path, pid: u32) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(format!("{pid}\n").as_bytes())?;
    file.persist(directory.join(OVERLAY_READY))
        .map(drop)
        .map_err(|error| error.error)
}

/// Idempotent: a missing file is already removed.
pub fn remove_overlay_ready(directory: &Path) {
    match std::fs::remove_file(directory.join(OVERLAY_READY)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("dojo: could not remove {OVERLAY_READY}: {error}"),
    }
}

/// Whether a live shell holds a parked overlay: the file names a pid whose
/// executable under `proc` is called `dojo`. A file left by a shell that has
/// crashed names a dead pid or one that has since been reused, and fails here.
/// A binary replaced on disk reads as `... (deleted)` and still counts.
pub fn overlay_live(directory: &Path, proc: &Path) -> bool {
    let Ok(contents) = std::fs::read_to_string(directory.join(OVERLAY_READY)) else {
        return false;
    };
    let Ok(pid) = contents.trim().parse::<u32>() else {
        return false;
    };
    let Ok(exe) = std::fs::read_link(proc.join(pid.to_string()).join("exe")) else {
        return false;
    };
    let exe = exe.to_string_lossy();
    let exe = exe.strip_suffix(" (deleted)").unwrap_or(&exe);
    Path::new(exe).file_name().is_some_and(|name| name == SHELL_EXE)
}

/// Whether the locker has taken the lock and not yet released it.
pub fn lock_marker_present(directory: &Path) -> bool {
    directory.join(LOCKED_AT).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> SystemTime {
        SystemTime::now()
    }

    #[test]
    fn a_farewell_survives_the_round_trip_once() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("dojo");
        write_farewell(&directory, "  Securing   the perimeter \n").unwrap();
        assert_eq!(
            take_farewell(&directory, true, now()).as_deref(),
            Some("Securing the perimeter")
        );
        assert_eq!(take_farewell(&directory, true, now()), None);
        assert!(!directory.join(LOCK_HANDOFF).exists());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_file_means_no_farewell() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(take_farewell(root.path(), true, now()), None);
        assert_eq!(take_farewell(&root.path().join("absent"), true, now()), None);
    }

    #[test]
    fn a_switched_off_farewell_is_still_consumed() {
        let root = tempfile::tempdir().unwrap();
        write_farewell(root.path(), "Goodbye").unwrap();
        assert_eq!(take_farewell(root.path(), false, now()), None);
        assert!(!root.path().join(LOCK_HANDOFF).exists());
    }

    #[test]
    fn a_stale_handoff_is_consumed_and_ignored() {
        let root = tempfile::tempdir().unwrap();
        write_farewell(root.path(), "Goodbye").unwrap();
        let later = now() + Duration::from_secs(11);
        assert_eq!(take_farewell(root.path(), true, later), None);
        assert!(!root.path().join(LOCK_HANDOFF).exists());
        write_farewell(root.path(), "Goodbye").unwrap();
        assert_eq!(
            take_farewell(root.path(), true, now() + Duration::from_secs(9)).as_deref(),
            Some("Goodbye")
        );
    }

    #[test]
    fn empty_and_overlong_lines_are_not_farewells() {
        let root = tempfile::tempdir().unwrap();
        for text in ["", "   \n", &"x".repeat(FAREWELL_MAX_CHARS + 1)] {
            std::fs::write(root.path().join(LOCK_HANDOFF), text).unwrap();
            assert_eq!(take_farewell(root.path(), true, now()), None, "{text:?}");
        }
        std::fs::write(root.path().join(LOCK_HANDOFF), "x".repeat(FAREWELL_MAX_CHARS)).unwrap();
        assert!(take_farewell(root.path(), true, now()).is_some());
    }

    #[test]
    fn only_a_marker_written_since_the_closing_began_counts() {
        let root = tempfile::tempdir().unwrap();
        let begun = now();
        assert!(!lock_taken_since(root.path(), begun));
        std::fs::write(root.path().join(LOCKED_AT), "1 x\n").unwrap();
        assert!(lock_taken_since(root.path(), begun));
        assert!(!lock_taken_since(root.path(), begun + Duration::from_secs(60)));
    }

    fn fake_proc(root: &Path, pid: u32, exe: &str) -> PathBuf {
        let proc = root.join("proc");
        std::fs::create_dir_all(proc.join(pid.to_string())).unwrap();
        std::os::unix::fs::symlink(exe, proc.join(pid.to_string()).join("exe")).unwrap();
        proc
    }

    #[test]
    fn a_live_shell_with_a_parked_overlay_is_found_through_its_pid() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("dojo");
        let proc = fake_proc(root.path(), 4242, "/home/n/.local/bin/dojo");
        assert!(!overlay_live(&directory, &proc), "no file");
        write_overlay_ready(&directory, 4242).unwrap();
        assert!(overlay_live(&directory, &proc));
        remove_overlay_ready(&directory);
        remove_overlay_ready(&directory);
        assert!(!overlay_live(&directory, &proc), "removed");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    }

    #[test]
    fn a_stale_overlay_file_is_not_live() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("dojo");
        let proc = fake_proc(root.path(), 7, "/usr/bin/bash");
        write_overlay_ready(&directory, 7).unwrap();
        assert!(!overlay_live(&directory, &proc), "pid reused by another program");
        write_overlay_ready(&directory, 8).unwrap();
        assert!(!overlay_live(&directory, &proc), "dead pid");
        std::fs::write(directory.join(OVERLAY_READY), "soon\n").unwrap();
        assert!(!overlay_live(&directory, &proc), "unparsable");
        let lookalike = fake_proc(root.path(), 9, "/opt/not-dojo");
        write_overlay_ready(&directory, 9).unwrap();
        assert!(!overlay_live(&directory, &lookalike), "a name that only ends in dojo");
    }

    #[test]
    fn a_shell_whose_binary_was_replaced_is_still_live() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("dojo");
        let proc = fake_proc(root.path(), 11, "/home/n/.local/bin/dojo (deleted)");
        write_overlay_ready(&directory, 11).unwrap();
        assert!(overlay_live(&directory, &proc));
    }

    #[test]
    fn the_directory_needs_an_absolute_runtime_dir() {
        assert_eq!(
            directory_in("/run/user/1000".into()),
            Some("/run/user/1000/dojo".into())
        );
        assert_eq!(directory_in("run/user".into()), None);
    }
}
