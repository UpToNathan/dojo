//! The marker that tells the shell its session follows the login screen:
//! `$XDG_RUNTIME_DIR/dojo/after-greeter`. The session helper creates it as the user in the
//! forked child, right before the login shell is exec'd, and the shell deletes it as it starts
//! (`session::login::consume_after_greeter`). It carries no data, so nothing about it is
//! trusted beyond its existence in a directory only the user can write.
//!
//! Everything here runs between `fork` and `exec`, so it allocates nothing and calls only
//! libc. Nothing is followed through a symlink: every directory is opened by descriptor with
//! `O_NOFOLLOW` and must belong to the user.

use std::ffi::CStr;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};

pub const DIRECTORY: &CStr = c"dojo";
pub const FILE: &CStr = c"after-greeter";

fn checked(fd: libc::c_int) -> io::Result<OwnedFd> {
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

/// Opens `name` below `parent` (or `name` itself when `parent` is `AT_FDCWD`) as a directory the
/// calling user owns, never through a symlink.
fn owned_directory(parent: libc::c_int, name: &CStr) -> io::Result<OwnedFd> {
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let directory = checked(unsafe { libc::openat(parent, name.as_ptr(), flags) })?;
    let mut status: libc::stat = unsafe { std::mem::zeroed() };
    use std::os::fd::AsRawFd;
    if unsafe { libc::fstat(directory.as_raw_fd(), &mut status) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if status.st_uid != unsafe { libc::geteuid() } {
        return Err(io::Error::from_raw_os_error(libc::EPERM));
    }
    Ok(directory)
}

/// Creates `<runtime>/dojo/after-greeter` as the calling user, mode 0600. `runtime` is the
/// session's `XDG_RUNTIME_DIR`, which must exist and be the user's. A marker already there, a
/// symlink included, is replaced, never followed. The directory `dojo` is created (0700) when
/// missing and refused when it is a symlink or belongs to someone else.
pub fn create(runtime: &CStr) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let runtime = owned_directory(libc::AT_FDCWD, runtime)?;
    if unsafe { libc::mkdirat(runtime.as_raw_fd(), DIRECTORY.as_ptr(), 0o700) } < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EEXIST) {
            return Err(error);
        }
    }
    let directory = owned_directory(runtime.as_raw_fd(), DIRECTORY)?;
    if unsafe { libc::unlinkat(directory.as_raw_fd(), FILE.as_ptr(), 0) } < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENOENT) {
            return Err(error);
        }
    }
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    checked(unsafe { libc::openat(directory.as_raw_fd(), FILE.as_ptr(), flags, 0o600) }).map(drop)
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::path::Path;

    use super::*;

    fn make(runtime: &Path) -> io::Result<()> {
        create(&CString::new(runtime.as_os_str().as_bytes()).unwrap())
    }

    fn me() -> u32 {
        unsafe { libc::geteuid() }
    }

    #[test]
    fn the_marker_is_an_empty_private_file_of_the_user_in_a_private_directory() {
        let runtime = tempfile::tempdir().unwrap();
        make(runtime.path()).unwrap();
        let marker = runtime.path().join("dojo/after-greeter");
        let file = std::fs::symlink_metadata(&marker).unwrap();
        assert!(file.file_type().is_file());
        assert_eq!((file.uid(), file.mode() & 0o7777, file.len()), (me(), 0o600, 0));
        let directory = std::fs::metadata(runtime.path().join("dojo")).unwrap();
        assert_eq!((directory.uid(), directory.mode() & 0o7777), (me(), 0o700));
    }

    #[test]
    fn making_it_twice_leaves_one_marker() {
        let runtime = tempfile::tempdir().unwrap();
        make(runtime.path()).unwrap();
        make(runtime.path()).unwrap();
        let names: Vec<_> = std::fs::read_dir(runtime.path().join("dojo"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["after-greeter"]);
    }

    #[test]
    fn an_existing_directory_keeps_its_other_files() {
        let runtime = tempfile::tempdir().unwrap();
        std::fs::create_dir(runtime.path().join("dojo")).unwrap();
        std::fs::write(runtime.path().join("dojo/session-started"), "").unwrap();
        make(runtime.path()).unwrap();
        assert!(runtime.path().join("dojo/session-started").exists());
        assert!(runtime.path().join("dojo/after-greeter").exists());
    }

    #[test]
    fn a_marker_that_is_a_symlink_is_replaced_and_its_target_left_alone() {
        let runtime = tempfile::tempdir().unwrap();
        let victim = runtime.path().join("victim");
        std::fs::write(&victim, "keep").unwrap();
        std::fs::set_permissions(&victim, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::create_dir(runtime.path().join("dojo")).unwrap();
        symlink(&victim, runtime.path().join("dojo/after-greeter")).unwrap();
        make(runtime.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(std::fs::metadata(&victim).unwrap().mode() & 0o7777, 0o644);
        let marker = std::fs::symlink_metadata(runtime.path().join("dojo/after-greeter")).unwrap();
        assert!(marker.file_type().is_file());
    }

    #[test]
    fn a_directory_that_is_a_symlink_is_refused() {
        let runtime = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        symlink(elsewhere.path(), runtime.path().join("dojo")).unwrap();
        assert!(make(runtime.path()).is_err());
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_runtime_directory_that_is_a_symlink_or_missing_is_refused() {
        let real = tempfile::tempdir().unwrap();
        let parent = tempfile::tempdir().unwrap();
        let link = parent.path().join("run");
        symlink(real.path(), &link).unwrap();
        assert!(make(&link).is_err());
        assert_eq!(std::fs::read_dir(real.path()).unwrap().count(), 0);
        assert!(make(&parent.path().join("missing")).is_err());
    }

    #[test]
    fn a_runtime_directory_of_someone_else_is_refused() {
        if me() != 0 {
            assert_eq!(
                make(Path::new("/")).unwrap_err().raw_os_error(),
                Some(libc::EPERM)
            );
        }
    }

    #[test]
    fn a_runtime_directory_that_is_a_file_is_refused() {
        let runtime = tempfile::tempdir().unwrap();
        let file = runtime.path().join("file");
        std::fs::write(&file, "").unwrap();
        assert!(make(&file).is_err());
    }

    #[test]
    fn a_marker_that_is_a_directory_is_an_error_not_a_followed_path() {
        let runtime = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(runtime.path().join("dojo/after-greeter")).unwrap();
        assert!(make(runtime.path()).is_err());
    }
}
