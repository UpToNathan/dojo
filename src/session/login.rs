//! Whether this shell start is a login, and whether the machine just booted.
//!
//! A login is the first shell start of a session. It leaves a marker file in
//! the runtime directory, which logind removes when the last session ends, so a
//! shell restarted mid-session finds the marker and stays quiet. Everything
//! here fails quiet: a missing or unwritable runtime directory means no
//! animation and never blocks startup.

use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The machine counts as freshly booted for the login greeting below this uptime.
pub const FRESH_BOOT: Duration = Duration::from_secs(300);

/// How a shell start relates to its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// First shell of the session: it claimed the marker.
    Login,
    /// The marker was already there: the shell restarted mid-session.
    Restart,
    /// No usable runtime directory. Treated as a restart.
    Unknown,
}

impl Start {
    pub fn is_login(self) -> bool {
        self == Start::Login
    }

    /// The decision, from the outcome of the exclusive marker creation.
    pub fn from_claim(claim: std::io::Result<()>) -> Start {
        match claim {
            Ok(()) => Start::Login,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Start::Restart,
            Err(_) => Start::Unknown,
        }
    }
}

/// `$XDG_RUNTIME_DIR/dojo/session-started`. A relative `XDG_RUNTIME_DIR` is ignored.
pub fn marker_path_from(xdg_runtime_dir: Option<OsString>) -> Option<PathBuf> {
    let runtime = PathBuf::from(xdg_runtime_dir?);
    runtime
        .is_absolute()
        .then(|| runtime.join("dojo/session-started"))
}

/// Creates the marker unless it exists, atomically: a temporary file in the
/// same directory is hard-linked into place, so two shells racing for the login
/// cannot both win and a reader never sees a half-written marker.
pub fn claim_marker(marker: &Path) -> std::io::Result<()> {
    let directory = marker
        .parent()
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidInput, "marker has no directory"))?;
    // Only the final link may report `AlreadyExists`, which means a restart.
    let not_a_claim = |error: std::io::Error| match error.kind() {
        ErrorKind::AlreadyExists => std::io::Error::other(error),
        _ => error,
    };
    std::fs::create_dir_all(directory).map_err(not_a_claim)?;
    let file = tempfile::NamedTempFile::new_in(directory).map_err(not_a_claim)?;
    match file.persist_noclobber(marker) {
        Ok(_) => Ok(()),
        Err(error) => Err(error.error),
    }
}

/// Decides this shell start and claims the marker when it is a login.
pub fn detect(xdg_runtime_dir: Option<OsString>) -> Start {
    match marker_path_from(xdg_runtime_dir) {
        Some(marker) => Start::from_claim(claim_marker(&marker)),
        None => Start::Unknown,
    }
}

/// Whether `/proc/uptime` text reports less than [`FRESH_BOOT`] since boot.
pub fn fresh_boot_from(uptime: &str) -> bool {
    uptime
        .split_whitespace()
        .next()
        .and_then(|seconds| seconds.parse::<f64>().ok())
        .is_some_and(|seconds| seconds >= 0. && seconds < FRESH_BOOT.as_secs_f64())
}

pub fn fresh_boot() -> bool {
    std::fs::read_to_string("/proc/uptime").is_ok_and(|text| fresh_boot_from(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_decision_follows_the_claim() {
        assert_eq!(Start::from_claim(Ok(())), Start::Login);
        for (kind, expected) in [
            (ErrorKind::AlreadyExists, Start::Restart),
            (ErrorKind::PermissionDenied, Start::Unknown),
            (ErrorKind::NotFound, Start::Unknown),
            (ErrorKind::ReadOnlyFilesystem, Start::Unknown),
        ] {
            assert_eq!(Start::from_claim(Err(kind.into())), expected, "{kind:?}");
        }
        assert!(Start::Login.is_login());
        assert!(!Start::Restart.is_login());
        assert!(!Start::Unknown.is_login());
    }

    #[test]
    fn the_marker_lives_under_an_absolute_runtime_directory() {
        let path = |dir: Option<&str>| marker_path_from(dir.map(Into::into));
        assert_eq!(
            path(Some("/run/user/1000")),
            Some("/run/user/1000/dojo/session-started".into())
        );
        assert_eq!(path(Some("run/user/1000")), None);
        assert_eq!(path(Some("")), None);
        assert_eq!(path(None), None);
    }

    #[test]
    fn the_first_start_is_a_login_and_a_restart_is_not() {
        let runtime = tempfile::tempdir().unwrap();
        let detect = || detect(Some(runtime.path().into()));
        assert_eq!(detect(), Start::Login);
        assert!(runtime.path().join("dojo/session-started").is_file());
        assert_eq!(detect(), Start::Restart);
        assert_eq!(detect(), Start::Restart);
        std::fs::remove_file(runtime.path().join("dojo/session-started")).unwrap();
        assert_eq!(detect(), Start::Login);
    }

    #[test]
    fn claiming_leaves_only_the_marker_behind() {
        let runtime = tempfile::tempdir().unwrap();
        let marker = runtime.path().join("dojo/session-started");
        claim_marker(&marker).unwrap();
        assert_eq!(
            claim_marker(&marker).unwrap_err().kind(),
            ErrorKind::AlreadyExists
        );
        let names: Vec<_> = std::fs::read_dir(marker.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["session-started"]);
    }

    #[test]
    fn a_missing_or_unwritable_runtime_directory_is_not_a_login() {
        assert_eq!(detect(None), Start::Unknown);
        assert_eq!(detect(Some("relative".into())), Start::Unknown);
        let runtime = tempfile::tempdir().unwrap();
        let file = runtime.path().join("not-a-directory");
        std::fs::write(&file, "").unwrap();
        assert_eq!(detect(Some(file.into())), Start::Unknown);
        std::fs::write(runtime.path().join("dojo"), "").unwrap();
        assert_eq!(detect(Some(runtime.path().into())), Start::Unknown);
        assert_eq!(
            detect(Some(runtime.path().join("missing/below/a/file").into())),
            Start::Login,
            "a missing directory is created"
        );
    }

    #[test]
    fn uptime_under_five_minutes_is_a_fresh_boot() {
        assert!(fresh_boot_from("12.34 45.67\n"));
        assert!(fresh_boot_from("299.99 1.0"));
        assert!(!fresh_boot_from("300.00 1.0"));
        assert!(!fresh_boot_from("86400.5 1000.0\n"));
        assert!(!fresh_boot_from(""));
        assert!(!fresh_boot_from("soon"));
        assert!(!fresh_boot_from("-1.0 0"));
    }
}
