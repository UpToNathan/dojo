//! Executable identity for the operator-selected native semantic preparer.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Captured once at compositor startup. A pathname supplied by a request never
/// changes this identity. Native peer ownership must also match at admission.
pub struct TrustedPreparer {
    path: PathBuf,
    device: u64,
    inode: u64,
    _executable: File,
}
impl TrustedPreparer {
    pub fn open(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path).context("resolve pinned semantic preparer")?;
        let relative = path
            .strip_prefix("/nix/store")
            .context("semantic preparer must be in the Nix store")?;
        let components: Vec<_> = relative.components().collect();
        let named_agent = match components.as_slice() {
            [entry] => entry
                .as_os_str()
                .to_str()
                .is_some_and(|name| name.ends_with("-dojo-agent")),
            [_, bin, executable] => {
                bin.as_os_str() == "bin" && executable.as_os_str() == "dojo-agent"
            }
            _ => false,
        };
        ensure!(named_agent, "pin a Nix-store dojo-agent executable");
        let mut file = File::open(&path)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == 0
                && metadata.mode() & 0o222 == 0
                && metadata.mode() & 0o111 != 0,
            "semantic preparer must be an immutable root-owned executable"
        );
        let mut magic = [0; 4];
        file.read_exact(&mut magic)?;
        ensure!(
            magic == *b"\x7fELF",
            "semantic preparer must be a native ELF executable"
        );
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
            _executable: file,
        })
    }

    pub fn matches(&self, pid: u32, start: u64) -> bool {
        self.matches_inner(pid, start).unwrap_or(false)
    }

    fn matches_inner(&self, pid: u32, start: u64) -> Result<bool> {
        if pid == 0 || start == 0 || process_start(pid)? != start {
            return Ok(false);
        }
        let process = PathBuf::from(format!("/proc/{pid}"));
        if fs::metadata(&process)?.uid() != unsafe { libc::geteuid() } {
            return Ok(false);
        }
        let exe = process.join("exe");
        if fs::read_link(&exe)? != self.path {
            return Ok(false);
        }
        let actual = fs::metadata(&exe)?;
        Ok(actual.is_file()
            && actual.dev() == self.device
            && actual.ino() == self.inode
            && actual.uid() == 0
            && actual.mode() & 0o222 == 0
            && process_start(pid)? == start)
    }
}
fn process_start(pid: u32) -> Result<u64> {
    let mut value = String::new();
    File::open(format!("/proc/{pid}/stat"))?
        .take(8193)
        .read_to_string(&mut value)?;
    ensure!(value.len() <= 8192, "process identity exceeds limit");
    value
        .rsplit_once(')')
        .context("invalid process identity")?
        .1
        .split_whitespace()
        .nth(19)
        .context("missing process generation")?
        .parse()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trusted_store_child() {
        if std::env::var_os("DOJO_PREPARER_TEST_CHILD").is_none() {
            return;
        }
        let executable = fs::read_link("/proc/self/exe").unwrap();
        let identity = TrustedPreparer::open(&executable).unwrap();
        let pid = std::process::id();
        let start = process_start(pid).unwrap();
        assert!(identity.matches(pid, start));
        assert!(!identity.matches(pid, start + 1));
        let parent = unsafe { libc::getppid() } as u32;
        assert!(!identity.matches(parent, process_start(parent).unwrap()));
    }
    #[test]
    fn real_nix_store_native_process_is_the_only_accepted_generation() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("dojo-agent");
        fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o555)).unwrap();
        let script = Command::new("nix-store")
            .arg("--add")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(script.status.success());
        let script = String::from_utf8(script.stdout).unwrap();
        assert!(TrustedPreparer::open(Path::new(script.trim())).is_err());
        fs::remove_file(&executable).unwrap();
        fs::copy("/proc/self/exe", &executable).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o555)).unwrap();
        let added = Command::new("nix-store")
            .arg("--add")
            .arg(executable)
            .output()
            .unwrap();
        assert!(
            added.status.success(),
            "{}",
            String::from_utf8_lossy(&added.stderr)
        );
        let path = String::from_utf8(added.stdout).unwrap();
        let result = Command::new(path.trim())
            .args([
                "--exact",
                "semantic_preparer::tests::trusted_store_child",
                "--nocapture",
            ])
            .env("DOJO_PREPARER_TEST_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    #[test]
    fn mutable_or_non_agent_paths_cannot_be_pinned() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("dojo-agent");
        fs::write(&file, b"\x7fELFfixture").unwrap();
        assert!(TrustedPreparer::open(&file).is_err());
        assert!(TrustedPreparer::open(Path::new("/proc/self/exe")).is_err());
    }
    #[test]
    fn exact_process_generation_and_executable_identity_are_required() {
        let path = fs::read_link("/proc/self/exe").unwrap();
        let file = File::open(&path).unwrap();
        let metadata = file.metadata().unwrap();
        let identity = TrustedPreparer {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
            _executable: file,
        };
        let pid = std::process::id();
        let start = process_start(pid).unwrap();
        // Test binaries are mutable user files, so production admission rejects
        // them even when inode and process generation match.
        assert!(!identity.matches(pid, start));
        assert!(!identity.matches(pid, start + 1));
        assert!(!identity.matches(0, start));
        let other = File::open("/proc/self/exe").unwrap();
        let different = TrustedPreparer {
            inode: metadata.ino().wrapping_add(1),
            _executable: other,
            ..identity
        };
        assert!(!different.matches(pid, start));
    }
}
