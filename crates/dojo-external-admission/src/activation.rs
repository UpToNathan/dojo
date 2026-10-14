//! Fixed-recipe child provenance for an explicitly enrolled launch attester.
//! This module does not enroll brokers or grant permission to mutate a desktop.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
        unix::{fs::MetadataExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const MAX_CONFIG: u64 = 32 * 1024;
const MAX_EXECUTABLE: u64 = 1024 * 1024 * 1024;
const MAX_HELLO: usize = 4096;
const RECIPE_ENV: &str = "DOJO_LAUNCH_RECIPE_SHA256";
static STDIN_CONTROL_TAKEN: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeRecord {
    version: u32,
    executable: PathBuf,
    executable_sha256: String,
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
}

/// Values come only from the immutable operator-selected record. Starting a
/// child accepts no executable, arguments, environment or purported PID.
pub struct FixedLaunchRecipe {
    executable: File,
    record: RecipeRecord,
    digest: String,
}
impl FixedLaunchRecipe {
    pub fn open(path: &Path) -> Result<Self> {
        let path = immutable_file(path)?;
        let bytes = bounded_file(&path, MAX_CONFIG)?;
        let record: RecipeRecord = serde_json::from_slice(&bytes)?;
        validate_record(&record)?;
        let executable_path = immutable_file(&record.executable)?;
        let mut executable = File::open(executable_path)?;
        ensure!(
            executable.metadata()?.len() <= MAX_EXECUTABLE,
            "service ELF exceeds limit"
        );
        let mut header = [0; 6];
        executable.read_exact(&mut header)?;
        ensure!(
            &header == b"\x7fELF\x02\x01",
            "service must be a Linux ELF64 image"
        );
        executable.seek(SeekFrom::Start(0))?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        loop {
            let size = executable.read(&mut buffer)?;
            if size == 0 {
                break;
            }
            hash.update(&buffer[..size]);
        }
        ensure!(
            format!("{:x}", hash.finalize()) == record.executable_sha256,
            "service ELF differs from enrolled recipe"
        );
        executable.seek(SeekFrom::Start(0))?;
        Ok(Self {
            executable,
            record,
            digest: format!("{:x}", Sha256::digest(bytes)),
        })
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn spawn(&self) -> Result<AttestedChild> {
        let (control, inherited) = UnixStream::pair()?;
        enable_credentials(&control)?;
        // Executing the opened ELF preserves its identity through the fork.
        // Stdin is the single deliberate inherited capability; all ambient
        // environment is cleared and the other standard descriptors are null.
        let inherited: OwnedFd = inherited.into();
        let child = Command::new(format!("/proc/self/fd/{}", self.executable.as_raw_fd()))
            .args(&self.record.arguments)
            .env_clear()
            .envs(&self.record.environment)
            .env(RECIPE_ENV, &self.digest)
            .stdin(Stdio::from(inherited))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let child = ReapedChild(child);
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, child.0.id(), 0) };
        ensure!(
            raw >= 0,
            "cannot monitor spawned service generation: {}",
            std::io::Error::last_os_error()
        );
        let pidfd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        let start = process_start(child.0.id())?;
        Ok(AttestedChild {
            child,
            control,
            start,
            pidfd,
            recipe: self.digest.clone(),
        })
    }
}

/// Constructed only for a child actually spawned by the selected recipe.
/// The owning attester must still be explicitly authenticated by Niri.
pub struct AttestedChild {
    child: ReapedChild,
    control: UnixStream,
    start: u64,
    pidfd: OwnedFd,
    recipe: String,
}
impl AttestedChild {
    pub fn process_id(&self) -> u32 {
        self.child.0.id()
    }
    pub fn process_start(&self) -> u64 {
        self.start
    }
    pub fn process_handle(&self) -> BorrowedFd<'_> {
        self.pidfd.as_fd()
    }
    pub fn recipe_digest(&self) -> &str {
        &self.recipe
    }

    pub fn receive_ready(&mut self, deadline: Instant) -> Result<()> {
        let mut header = [0; 4];
        self.receive_exact(&mut header, deadline)?;
        let size = u32::from_be_bytes(header) as usize;
        ensure!(size <= MAX_HELLO, "child hello exceeds limit");
        let mut bytes = vec![0; size];
        self.receive_exact(&mut bytes, deadline)?;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Hello {
            version: u32,
            recipe_sha256: String,
        }
        let hello: Hello = serde_json::from_slice(&bytes)?;
        ensure!(
            hello.version == 1 && hello.recipe_sha256 == self.recipe,
            "child hello differs from selected recipe"
        );
        ensure!(
            process_start(self.child.0.id())? == self.start && self.child.0.try_wait()?.is_none(),
            "child generation ended during registration"
        );
        Ok(())
    }

    fn receive_exact(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<()> {
        let mut offset = 0;
        while offset < bytes.len() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            ensure!(!remaining.is_zero(), "child registration deadline exceeded");
            self.control.set_read_timeout(Some(remaining))?;
            let size = receive_credentials(&self.control, &mut bytes[offset..], self.child.0.id())?;
            ensure!(size > 0, "child control closed before registration");
            offset += size;
        }
        Ok(())
    }
}
impl Drop for AttestedChild {
    fn drop(&mut self) {
        let _ = self.control.shutdown(std::net::Shutdown::Both);
    }
}
struct ReapedChild(Child);
impl Drop for ReapedChild {
    fn drop(&mut self) {
        // The unreaped Child owns this PID; it cannot have been reused.
        // This belongs to the launch attester, never to Niri's event thread.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Service-side bootstrap. The descriptor must be an inherited Unix stream,
/// and is marked close-on-exec before any descendants may be launched.
pub struct InheritedChildControl(UnixStream);
impl InheritedChildControl {
    pub fn from_stdin() -> Result<Self> {
        ensure!(
            STDIN_CONTROL_TAKEN
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "service bootstrap already took stdin"
        );
        let mut kind = 0i32;
        let mut length = std::mem::size_of_val(&kind) as libc::socklen_t;
        ensure!(
            unsafe {
                libc::getsockopt(
                    0,
                    libc::SOL_SOCKET,
                    libc::SO_TYPE,
                    (&mut kind as *mut i32).cast(),
                    &mut length,
                )
            } == 0
                && kind == libc::SOCK_STREAM,
            "service bootstrap requires an inherited stream"
        );
        let mut domain = 0i32;
        ensure!(
            unsafe {
                libc::getsockopt(
                    0,
                    libc::SOL_SOCKET,
                    libc::SO_DOMAIN,
                    (&mut domain as *mut i32).cast(),
                    &mut length,
                )
            } == 0
                && domain == libc::AF_UNIX,
            "service bootstrap requires an inherited Unix stream"
        );
        ensure!(
            unsafe { libc::fcntl(0, libc::F_SETFD, libc::FD_CLOEXEC) } == 0,
            "cannot protect inherited service capability"
        );
        // The service bootstrap takes stdin exactly once.
        Ok(Self(unsafe { UnixStream::from_raw_fd(0) }))
    }
    pub fn ready(&mut self) -> Result<()> {
        let recipe = std::env::var(RECIPE_ENV).context("missing selected recipe identity")?;
        ensure!(valid_digest(&recipe), "invalid selected recipe identity");
        let bytes = serde_json::to_vec(&serde_json::json!({"version":1,"recipe_sha256":recipe}))?;
        self.0.set_write_timeout(Some(Duration::from_secs(3)))?;
        self.0.write_all(&(bytes.len() as u32).to_be_bytes())?;
        self.0.write_all(&bytes)?;
        Ok(())
    }
}

fn validate_record(record: &RecipeRecord) -> Result<()> {
    ensure!(
        record.version == 1 && valid_digest(&record.executable_sha256),
        "invalid launch recipe"
    );
    ensure!(
        record.arguments.len() <= 32
            && record
                .arguments
                .iter()
                .all(|value| value.len() <= 4096 && !value.contains('\0')),
        "invalid fixed arguments"
    );
    ensure!(
        record.environment.len() <= 32,
        "fixed environment exceeds limit"
    );
    for (key, value) in &record.environment {
        ensure!(
            allowed_environment(key) && value.len() <= 8192 && !value.contains('\0'),
            "unsupported fixed environment entry"
        );
        if matches!(key.as_str(), "LD_LIBRARY_PATH" | "PATH" | "FONTCONFIG_PATH") {
            for path in std::env::split_paths(value) {
                ensure!(
                    immutable_path(&path)?.is_dir(),
                    "fixed search path is not an immutable directory"
                );
            }
        }
        if matches!(
            key.as_str(),
            "FONTCONFIG_FILE" | "DOJO_DESKTOP_POLICY" | "DOJO_PERCEPTION_INSTALLATION"
        ) {
            immutable_file(Path::new(value))?;
        }
    }
    Ok(())
}
fn allowed_environment(key: &str) -> bool {
    #[cfg(test)]
    if key == "DOJO_ACTIVATION_TEST_COOKIE" {
        return true;
    }
    matches!(
        key,
        "PATH"
            | "HOME"
            | "XDG_DATA_HOME"
            | "XDG_DATA_DIRS"
            | "XDG_RUNTIME_DIR"
            | "DBUS_SESSION_BUS_ADDRESS"
            | "FONTCONFIG_FILE"
            | "FONTCONFIG_PATH"
            | "LD_LIBRARY_PATH"
            | "DOJO_DESKTOP_POLICY"
            | "DOJO_PERCEPTION_INSTALLATION"
    )
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn immutable_path(path: &Path) -> Result<PathBuf> {
    let canonical = path.canonicalize()?;
    let relative = canonical
        .strip_prefix("/nix/store")
        .context("recipe inputs must be immutable Nix members")?;
    let mut components = relative.components();
    let member = components
        .next()
        .context("Nix store itself is not an enrolled member")?;
    let mut current = PathBuf::from("/nix/store").join(member.as_os_str());
    loop {
        let metadata = std::fs::symlink_metadata(&current)?;
        ensure!(
            metadata.uid() == 0
                && metadata.mode() & 0o222 == 0
                && !metadata.file_type().is_symlink(),
            "launch recipe member is mutable"
        );
        let Some(component) = components.next() else {
            break;
        };
        current.push(component.as_os_str());
    }
    Ok(canonical)
}
fn immutable_file(path: &Path) -> Result<PathBuf> {
    let path = immutable_path(path)?;
    ensure!(path.is_file(), "launch recipe member is not a regular file");
    Ok(path)
}
fn bounded_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "launch recipe input exceeds limit"
    );
    Ok(bytes)
}
fn process_start(pid: u32) -> Result<u64> {
    let bytes = bounded_file(&PathBuf::from(format!("/proc/{pid}/stat")), 8192)?;
    let text = std::str::from_utf8(&bytes)?;
    text.rsplit_once(')')
        .and_then(|(_, tail)| tail.split_whitespace().nth(19))
        .context("process generation unavailable")?
        .parse()
        .map_err(Into::into)
}
fn enable_credentials(stream: &UnixStream) -> Result<()> {
    let enabled = 1i32;
    ensure!(
        unsafe {
            libc::setsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const i32).cast(),
                std::mem::size_of_val(&enabled) as _,
            )
        } == 0,
        "cannot authenticate child control messages"
    );
    Ok(())
}
fn receive_credentials(stream: &UnixStream, bytes: &mut [u8], child_pid: u32) -> Result<usize> {
    let mut ancillary = [0usize; 16];
    let mut iovec = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut iovec;
    message.msg_iovlen = 1;
    message.msg_control = ancillary.as_mut_ptr().cast();
    message.msg_controllen = std::mem::size_of_val(&ancillary);
    let size = unsafe { libc::recvmsg(stream.as_raw_fd(), &mut message, libc::MSG_CMSG_CLOEXEC) };
    ensure!(
        size >= 0,
        "child control receive failed: {}",
        std::io::Error::last_os_error()
    );
    if size == 0 {
        return Ok(0);
    }
    let mut credentials = None;
    let mut invalid = false;
    unsafe {
        let mut header = libc::CMSG_FIRSTHDR(&message);
        while !header.is_null() {
            let item = &*header;
            let base = libc::CMSG_LEN(0) as usize;
            if item.cmsg_level == libc::SOL_SOCKET && item.cmsg_type == libc::SCM_RIGHTS {
                let count = item.cmsg_len.saturating_sub(base) / std::mem::size_of::<i32>();
                for index in 0..count {
                    libc::close(std::ptr::read_unaligned(
                        libc::CMSG_DATA(header).cast::<i32>().add(index),
                    ));
                }
                invalid = true;
            } else if item.cmsg_level == libc::SOL_SOCKET
                && item.cmsg_type == libc::SCM_CREDENTIALS
                && item.cmsg_len == libc::CMSG_LEN(std::mem::size_of::<libc::ucred>() as _) as usize
                && credentials.is_none()
            {
                credentials = Some(std::ptr::read_unaligned(
                    libc::CMSG_DATA(header).cast::<libc::ucred>(),
                ));
            } else {
                invalid = true;
            }
            header = libc::CMSG_NXTHDR(&message, header);
        }
    }
    ensure!(
        !invalid && message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) == 0,
        "unexpected child control ancillary data"
    );
    let owner = credentials.context("child control lacks kernel credentials")?;
    ensure!(
        owner.pid > 0 && owner.pid as u32 == child_pid && owner.uid == unsafe { libc::geteuid() },
        "child control sender is not the spawned process"
    );
    Ok(size as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    struct FixtureDirectory(PathBuf);
    impl FixtureDirectory {
        fn new() -> Self {
            use std::os::unix::fs::DirBuilderExt;
            let mut nonce = [0; 16];
            getrandom::getrandom(&mut nonce).unwrap();
            let id = nonce
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let path = std::env::temp_dir().join(format!("dojo-activation-{id}"));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .unwrap();
            Self(path)
        }
    }
    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn store(path: &Path) -> PathBuf {
        let result = Command::new("nix-store")
            .arg("--add")
            .arg(path)
            .output()
            .unwrap();
        assert!(result.status.success(), "Nix fixture staging failed");
        PathBuf::from(String::from_utf8(result.stdout).unwrap().trim())
    }
    fn image() -> &'static PathBuf {
        static IMAGE: OnceLock<PathBuf> = OnceLock::new();
        IMAGE.get_or_init(|| store(&std::env::current_exe().unwrap()))
    }
    fn record(cookie: &str) -> serde_json::Value {
        serde_json::json!({"version":1,"executable":image(),
            "executable_sha256":format!("{:x}",Sha256::digest(std::fs::read(image()).unwrap())),
            "arguments":["--exact","activation::tests::inherited_channel_probe","--ignored","--nocapture"],
            "environment":{"DOJO_ACTIVATION_TEST_COOKIE":cookie}})
    }
    fn recipe(value: &serde_json::Value) -> PathBuf {
        let directory = FixtureDirectory::new();
        let path = directory.0.join("recipe.json");
        std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
        store(&path)
    }

    #[test]
    #[ignore = "child fixture, invoked only by a fixed-recipe parent"]
    fn inherited_channel_probe() {
        let cookie = std::env::var("DOJO_ACTIVATION_TEST_COOKIE").unwrap();
        let mut keys = std::env::vars().map(|(key, _)| key).collect::<Vec<_>>();
        keys.sort();
        assert_eq!(keys, vec!["DOJO_ACTIVATION_TEST_COOKIE", RECIPE_ENV]);
        let mut control = InheritedChildControl::from_stdin().unwrap();
        assert!(InheritedChildControl::from_stdin().is_err());
        assert_ne!(
            unsafe { libc::fcntl(control.0.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        if cookie == "wrong-recipe" {
            let bytes = serde_json::to_vec(
                &serde_json::json!({"version":1,"recipe_sha256":"0".repeat(64)}),
            )
            .unwrap();
            control
                .0
                .write_all(&(bytes.len() as u32).to_be_bytes())
                .unwrap();
            control.0.write_all(&bytes).unwrap();
        } else {
            assert_eq!(cookie, "selected");
            control.ready().unwrap();
        }
        let mut byte = [0];
        let _ = control.0.read(&mut byte);
    }

    #[test]
    fn fixed_recipe_executes_owned_image_with_only_selected_environment() {
        let recipe = FixedLaunchRecipe::open(&recipe(&record("selected"))).unwrap();
        let mut child = recipe.spawn().unwrap();
        let pid = child.process_id();
        assert_eq!(
            std::fs::read_link(format!("/proc/{pid}/exe")).unwrap(),
            *image()
        );
        child
            .receive_ready(Instant::now() + Duration::from_secs(3))
            .unwrap();
        assert_eq!(child.recipe_digest(), recipe.digest());
        assert_eq!(process_start(pid).unwrap(), child.process_start());
        drop(child);
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }

    #[test]
    fn same_image_arbitrary_invocation_has_no_inherited_capability() {
        let result = Command::new(image())
            .args([
                "--exact",
                "activation::tests::inherited_channel_probe",
                "--ignored",
            ])
            .env_clear()
            .env("DOJO_ACTIVATION_TEST_COOKIE", "selected")
            .env(RECIPE_ENV, "0".repeat(64))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!result.success());
    }

    #[test]
    fn mutable_recipe_wrong_digest_and_environment_injection_are_refused() {
        let directory = FixtureDirectory::new();
        let mutable = directory.0.join("recipe.json");
        std::fs::write(&mutable, serde_json::to_vec(&record("selected")).unwrap()).unwrap();
        assert!(FixedLaunchRecipe::open(&mutable).is_err());
        let mut wrong = record("selected");
        wrong["executable_sha256"] = "0".repeat(64).into();
        assert!(FixedLaunchRecipe::open(&recipe(&wrong)).is_err());
        let mut injected = record("selected");
        injected["environment"]["LD_PRELOAD"] = "/tmp/injected.so".into();
        assert!(FixedLaunchRecipe::open(&recipe(&injected)).is_err());
        injected["environment"]
            .as_object_mut()
            .unwrap()
            .remove("LD_PRELOAD");
        injected["environment"][RECIPE_ENV] = "0".repeat(64).into();
        assert!(FixedLaunchRecipe::open(&recipe(&injected)).is_err());
    }

    #[test]
    fn child_cannot_claim_another_recipe() {
        let recipe = FixedLaunchRecipe::open(&recipe(&record("wrong-recipe"))).unwrap();
        let mut child = recipe.spawn().unwrap();
        assert!(
            child
                .receive_ready(Instant::now() + Duration::from_secs(3))
                .unwrap_err()
                .to_string()
                .contains("differs from selected recipe")
        );
    }

    #[test]
    fn kernel_credentials_refuse_a_different_sender() {
        let (mut sender, receiver) = UnixStream::pair().unwrap();
        enable_credentials(&receiver).unwrap();
        sender.write_all(b"x").unwrap();
        let error = receive_credentials(&receiver, &mut [0], std::process::id() + 1).unwrap_err();
        assert!(error.to_string().contains("not the spawned process"));
    }
}
