//! Server-owned isolated browser lifetime. Caller tool arguments cannot choose
//! the executable, profile, process, debugging endpoint, or launch flags.
use anyhow::{Context, Result, bail, ensure};
use async_trait::async_trait;
use dojo_cua_browser::browser_observation::host::{BrowserRefusal, BrowserRefusalCode};
use dojo_cua_browser::browser_observation::{ExactWindow, OwnedBrowserSource};
use niri_ipc::{Request, Response};
use std::{
    fs::{File, Permissions},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{
            fs::{MetadataExt, PermissionsExt},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn refused(reason: impl Into<String>) -> BrowserRefusal {
    BrowserRefusal::new(BrowserRefusalCode::BrowserRouteUnavailable, reason)
}
fn start(pid: u32) -> Result<u64> {
    super::screenshot::process_start_time(pid)
}
fn parent(pid: u32) -> Result<u32> {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))?
        .rsplit_once(')')
        .context("process stat")?
        .1
        .split_whitespace()
        .nth(1)
        .context("process parent")?
        .parse()
        .map_err(Into::into)
}
pub(super) fn descends(pid: u32, root: u32, root_start: u64) -> bool {
    if start(root).ok() != Some(root_start) {
        return false;
    }
    let mut current = pid;
    for _ in 0..64 {
        if current == root {
            return true;
        }
        let Ok(next) = parent(current) else {
            return false;
        };
        if next == 0 || next == current {
            return false;
        }
        current = next;
    }
    false
}
fn pidfd(pid: u32) -> Result<OwnedFd> {
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
    ensure!(
        descriptor >= 0,
        "owned process pidfd unavailable: {}",
        std::io::Error::last_os_error()
    );
    Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
}
fn owned_descendants(root: u32, root_start: u64) -> (Vec<OwnedFd>, bool) {
    let mut queue = std::collections::VecDeque::from([root]);
    let mut out = Vec::new();
    let mut complete = true;
    let mut seen = std::collections::HashSet::new();
    while let Some(pid) = queue.pop_front() {
        if seen.len() >= 256 {
            complete = false;
            break;
        }
        if !seen.insert(pid) {
            continue;
        }
        if !descends(pid, root, root_start) {
            if start(pid).is_ok() {
                complete = false;
            }
            continue;
        }
        if let Ok(children) = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")) {
            queue.extend(
                children
                    .split_whitespace()
                    .filter_map(|s| s.parse::<u32>().ok()),
            );
        } else if start(pid).is_ok() {
            complete = false;
        }
        if let Ok(generation) = start(pid) {
            if let Ok(fd) = pidfd(pid) {
                if start(pid).ok() == Some(generation) {
                    out.push(fd);
                }
            } else if start(pid).ok() == Some(generation) {
                complete = false;
            }
        }
    }
    (out, complete)
}
fn fd_exited(fd: &OwnedFd) -> bool {
    let mut event = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    (unsafe { libc::poll(&mut event, 1, 0) }) > 0
}
fn trusted(executable: &Path) -> Result<PathBuf> {
    let path = executable
        .canonicalize()
        .context("resolve operator browser executable")?;
    ensure!(path.is_absolute(), "browser executable is not absolute");
    for part in path.ancestors() {
        let meta = std::fs::symlink_metadata(part)?;
        ensure!(
            meta.uid() == 0
                && (meta.mode() & 0o022 == 0 || (meta.is_dir() && meta.mode() & 0o1000 != 0))
                && !meta.file_type().is_symlink(),
            "browser installation is not root-owned immutable source"
        );
    }
    let meta = std::fs::metadata(&path)?;
    ensure!(
        meta.is_file() && meta.mode() & 0o111 != 0,
        "browser executable unavailable"
    );
    let name = path.to_string_lossy().to_ascii_lowercase();
    ensure!(
        ["helium", "chromium", "chrome"]
            .iter()
            .any(|product| name.contains(product)),
        "operator executable is not a supported Chromium installation"
    );
    Ok(path)
}
fn executable() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("DOJO_BROWSER_EXECUTABLE") {
        return trusted(Path::new(&path));
    }
    for name in ["helium", "chromium", "chromium-browser", "google-chrome"] {
        if let Some(paths) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&paths) {
                let path = directory.join(name);
                if path.exists() {
                    if let Ok(path) = trusted(&path) {
                        return Ok(path);
                    }
                }
            }
        }
    }
    bail!("no trusted operator Chromium installation found")
}
fn display(socket: Option<&Path>) -> Result<String> {
    if let Some(path) = socket {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("Niri socket name unavailable")?;
        let body = name
            .strip_prefix("niri.")
            .and_then(|s| s.strip_suffix(".sock"))
            .context("explicit Niri socket lacks display identity")?;
        let (display, _pid) = body
            .rsplit_once('.')
            .context("Niri socket process suffix")?;
        ensure!(
            display.starts_with("wayland-")
                && display
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "invalid Niri display identity"
        );
        return Ok(display.into());
    }
    std::env::var("WAYLAND_DISPLAY").context("owned browser requires the operator Wayland display")
}

fn launch_arguments(profile: &Path) -> Vec<String> {
    let mut arguments = [
        "--ozone-platform=wayland",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-background-networking",
        "--disable-sync",
        "--disable-component-update",
        "--disable-extensions",
        "--remote-debugging-address=127.0.0.1",
        "--remote-debugging-port=0",
        "--class=dojo-owned-browser",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    arguments.push(format!("--user-data-dir={}", profile.display()));
    arguments.push(format!("file://{}/start.html", profile.display()));
    arguments
}
fn exact_launch_command(pid: u32, profile: &Path) -> bool {
    let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
        return false;
    };
    if trusted(&exe).is_err() {
        return false;
    }
    let Some(exe) = exe.to_str() else {
        return false;
    };
    let mut arguments = vec![exe.to_owned()];
    arguments.extend(launch_arguments(profile));
    let Ok(actual) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let mut argv = arguments.join("\0").into_bytes();
    argv.push(0);
    let mut display = arguments.join(" ").into_bytes();
    display.push(0);
    actual == argv || actual == display
}

static SUPERVISOR_STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn stop_supervisor(_signal: i32) {
    SUPERVISOR_STOP.store(true, Ordering::Release);
}
pub(super) fn supervise(
    executable: PathBuf,
    profile: PathBuf,
    parent_pid: u32,
    parent_start: u64,
) -> Result<()> {
    let executable = trusted(&executable)?;
    let metadata = std::fs::symlink_metadata(&profile)?;
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::getuid() }
            && metadata.mode() & 0o077 == 0,
        "supervisor requires private owned profile"
    );
    ensure!(
        profile
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("dojo-browser-"))
            && profile.join("start.html").is_file(),
        "supervisor requires generated starter profile"
    );
    let parent = unsafe { libc::getppid() };
    ensure!(
        parent > 1 && parent as u32 == parent_pid && start(parent_pid).ok() == Some(parent_start),
        "supervisor parent identity changed"
    );
    unsafe {
        ensure!(
            libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) == 0,
            "browser subreaper unavailable"
        );
        libc::signal(
            libc::SIGTERM,
            stop_supervisor as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            stop_supervisor as *const () as libc::sighandler_t,
        );
        ensure!(
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0) == 0,
            "browser parent-death guard unavailable"
        );
    }
    ensure!(
        unsafe { libc::getppid() } == parent,
        "browser parent exited during supervisor setup"
    );
    let launcher = Command::new(executable)
        .args(launch_arguments(&profile))
        .stdin(Stdio::null())
        .spawn()
        .context("supervisor launch browser")?;
    let launcher_pid = launcher.id();
    let mut stopping = false;
    let mut cleanup_deadline = None;
    loop {
        let mut status = 0;
        let reaped = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if reaped < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
            break;
        }
        stopping |= reaped == launcher_pid as i32
            || SUPERVISOR_STOP.load(Ordering::Acquire)
            || unsafe { libc::getppid() } != parent;
        if stopping {
            let deadline =
                *cleanup_deadline.get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
            let pid = std::process::id();
            let children = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))?;
            for child in children
                .split_whitespace()
                .filter_map(|id| id.parse::<u32>().ok())
            {
                if let Ok(generation) = start(child) {
                    let (handles, _) = owned_descendants(child, generation);
                    for fd in handles {
                        unsafe {
                            libc::syscall(
                                libc::SYS_pidfd_send_signal,
                                fd.as_raw_fd(),
                                libc::SIGKILL,
                                std::ptr::null::<libc::siginfo_t>(),
                                0,
                            )
                        };
                    }
                }
            }
            ensure!(
                Instant::now() < deadline,
                "owned supervisor cleanup did not settle; profile must remain"
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(launcher);
    Ok(())
}

struct Proof {
    alive: AtomicBool,
    root: u32,
    root_start: u64,
    browser: u32,
    browser_start: u64,
    cdp_pid: i32,
    cdp_start: u64,
    debugging_port: u16,
    window: u64,
    profile: PathBuf,
}
#[async_trait]
impl OwnedBrowserSource for Proof {
    fn browser_process(&self) -> (i32, u64) {
        (self.cdp_pid, self.cdp_start)
    }
    fn debugging_port(&self) -> u16 {
        self.debugging_port
    }
    async fn current_owned(&self, window: ExactWindow) -> std::result::Result<(), BrowserRefusal> {
        if start(self.cdp_pid as u32).ok() != Some(self.cdp_start)
            || !descends(self.browser, self.cdp_pid as u32, self.cdp_start)
            || !self.alive.load(Ordering::Acquire)
            || window.window_id != self.window
            || window.pid <= 0
            || window.pid as u32 != self.browser
            || window.process_start != self.browser_start
            || start(self.browser).ok() != Some(self.browser_start)
            || !descends(self.browser, self.root, self.root_start)
        {
            return Err(refused("owned browser lifetime or exact process changed"));
        }
        if !exact_launch_command(self.cdp_pid as u32, &self.profile) {
            return Err(refused(
                "owned browser exact launch/profile binding changed",
            ));
        }
        Ok(())
    }
}
pub(super) struct OwnedBrowser {
    child: Option<Child>,
    profile: Option<tempfile::TempDir>,
    root: u32,
    root_start: u64,
    proof: Option<Arc<Proof>>,
    window: u64,
    endpoint: String,
}
impl OwnedBrowser {
    pub(super) fn launch(socket: Option<&Path>, cancelled: &AtomicBool) -> Result<Self> {
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "browser creation cancelled"
        );
        let executable = executable()?;
        let profile = tempfile::Builder::new().prefix("dojo-browser-").tempdir()?;
        std::fs::set_permissions(profile.path(), Permissions::from_mode(0o700))?;
        let start_page = profile.path().join("start.html");
        std::fs::write(
            &start_page,
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Dojo Browser</title></head><body><h1>Dojo</h1><a href=\"about:blank\" aria-label=\"Open blank page\">Open blank page</a></body></html>",
        )?;
        let stderr = File::create(profile.path().join("browser.log"))?;
        let environment = [
            "PATH",
            "XDG_RUNTIME_DIR",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "TZ",
            "LD_LIBRARY_PATH",
            "LIBGL_DRIVERS_PATH",
            "GBM_BACKENDS_PATH",
            "__EGL_VENDOR_LIBRARY_FILENAMES",
            "VK_ICD_FILENAMES",
            "VK_DRIVER_FILES",
            "DRI_PRIME",
            "NIX_LD",
            "NIX_LD_LIBRARY_PATH",
            "DBUS_SESSION_BUS_ADDRESS",
            "FONTCONFIG_FILE",
            "FONTCONFIG_PATH",
        ]
        .into_iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (key, value)));
        let mut child = Command::new(std::env::current_exe()?)
            .env_clear()
            .envs(environment)
            .env("HOME", profile.path())
            .env("XDG_CONFIG_HOME", profile.path().join("config"))
            .env("XDG_CACHE_HOME", profile.path().join("cache"))
            .arg("--browser-supervisor")
            .arg(&executable)
            .arg(profile.path())
            .arg(std::process::id().to_string())
            .arg(start(std::process::id())?.to_string())
            .env("WAYLAND_DISPLAY", display(socket)?)
            .env("XDG_SESSION_TYPE", "wayland")
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .context("launch isolated operator browser")?;
        let root = child.id();
        let root_start = match start(root) {
            Ok(start) => start,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let mut owned = Self {
            child: Some(child),
            profile: Some(profile),
            root,
            root_start,
            proof: None,
            window: 0,
            endpoint: String::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            ensure!(
                !cancelled.load(Ordering::Acquire),
                "browser creation cancelled"
            );
            if let Ok(contents) = std::fs::read_to_string(
                owned
                    .profile
                    .as_ref()
                    .unwrap()
                    .path()
                    .join("DevToolsActivePort"),
            ) {
                let mut lines = contents.lines();
                if let (Some(port), Some(path)) = (
                    lines.next().and_then(|s| s.parse::<u16>().ok()),
                    lines.next(),
                ) {
                    if port > 0
                        && path.starts_with("/devtools/browser/")
                        && path
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-'))
                    {
                        let mut niri = super::desktop_socket(socket)?;
                        if let Response::Windows(windows) =
                            niri.send(Request::Windows)?.map_err(anyhow::Error::msg)?
                        {
                            let matches = windows
                                .into_iter()
                                .filter(|w| {
                                    w.pid
                                        .and_then(|pid| u32::try_from(pid).ok())
                                        .is_some_and(|pid| descends(pid, root, root_start))
                                })
                                .collect::<Vec<_>>();
                            ensure!(
                                matches.len() <= 1,
                                "owned browser native window is ambiguous"
                            );
                            if let Some(window) = matches.first() {
                                let browser = u32::try_from(window.pid.unwrap())?;
                                owned.endpoint = format!("ws://127.0.0.1:{port}{path}");
                                owned.window = window.id;
                                let mut cdp_pid = browser;
                                loop {
                                    if exact_launch_command(
                                        cdp_pid,
                                        owned.profile.as_ref().unwrap().path(),
                                    ) {
                                        break;
                                    }
                                    ensure!(
                                        cdp_pid != root,
                                        "owned native process has no exact browser ancestor"
                                    );
                                    cdp_pid = parent(cdp_pid)?;
                                    ensure!(
                                        descends(cdp_pid, root, root_start),
                                        "browser ancestor left owned launch"
                                    );
                                }
                                owned.proof = Some(Arc::new(Proof {
                                    alive: AtomicBool::new(true),
                                    root,
                                    root_start,
                                    browser,
                                    browser_start: start(browser)?,
                                    cdp_pid: i32::try_from(cdp_pid)?,
                                    cdp_start: start(cdp_pid)?,
                                    debugging_port: port,
                                    window: window.id,
                                    profile: owned.profile.as_ref().unwrap().path().to_path_buf(),
                                }));
                                return Ok(owned);
                            }
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let log =
            std::fs::read_to_string(owned.profile.as_ref().unwrap().path().join("browser.log"))
                .unwrap_or_default();
        bail!(
            "isolated browser did not publish a native window/endpoint: {}",
            log.chars().take(1500).collect::<String>()
        )
    }
    pub(super) fn window_id(&self) -> u64 {
        self.window
    }
    pub(super) fn endpoint(&self) -> String {
        self.endpoint.clone()
    }
    pub(super) fn proof(&self) -> Arc<dyn OwnedBrowserSource> {
        self.proof.as_ref().expect("launched proof").clone()
    }
    pub(super) fn close(&mut self) -> Result<()> {
        if let Some(proof) = &self.proof {
            proof.alive.store(false, Ordering::Release);
        }
        if let Some(mut child) = self.child.take() {
            let supervisor_exited = pidfd(self.root).map(|fd| fd_exited(&fd)).unwrap_or(true);
            let (mut descendants, mut complete) = owned_descendants(self.root, self.root_start);
            if let Some(proof) = &self.proof {
                for (pid, generation) in [
                    (proof.cdp_pid as u32, proof.cdp_start),
                    (proof.browser, proof.browser_start),
                ] {
                    if start(pid).ok() == Some(generation) {
                        let (extra, known) = owned_descendants(pid, generation);
                        descendants.extend(extra);
                        complete &= known;
                    }
                }
            }
            for fd in &descendants {
                unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        fd.as_raw_fd(),
                        libc::SIGKILL,
                        std::ptr::null::<libc::siginfo_t>(),
                        0,
                    )
                };
            }
            // The unreaped launcher pins this process-group ID. No PID from a
            // caller, process name, or personal-profile scan is ever signalled.
            if start(self.root).ok() == Some(self.root_start) {
                unsafe { libc::kill(-(self.root as i32), libc::SIGKILL) };
            }
            let _ = child.kill();
            let status = match child.wait() {
                Ok(status) => status,
                Err(error) => {
                    if let Some(profile) = self.profile.take() {
                        let _ = profile.keep();
                    }
                    return Err(error).context("reap owned browser launcher");
                }
            };
            if (supervisor_exited && !status.success())
                || status.code().is_some_and(|code| code != 0)
            {
                complete = false;
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while descendants.iter().any(|fd| !fd_exited(fd)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if !complete || descendants.iter().any(|fd| !fd_exited(fd)) {
                if let Some(profile) = self.profile.take() {
                    let _ = profile.keep();
                }
                bail!("owned descendant still exists; preserve its profile");
            }
        }
        self.profile.take();
        Ok(())
    }
}
impl Drop for OwnedBrowser {
    fn drop(&mut self) {
        if self.close().is_err() {
            if let Some(profile) = self.profile.take() {
                let _ = profile.keep();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_reaps_only_owned_child_and_removes_profile() {
        let profile = tempfile::Builder::new()
            .prefix("dojo-browser-cleanup-")
            .tempdir()
            .unwrap();
        let path = profile.path().to_path_buf();
        let child = Command::new("sleep")
            .arg("60")
            .process_group(0)
            .spawn()
            .unwrap();
        let root = child.id();
        let root_start = start(root).unwrap();
        let mut owned = OwnedBrowser {
            child: Some(child),
            profile: Some(profile),
            root,
            root_start,
            proof: None,
            window: 0,
            endpoint: String::new(),
        };
        owned.close().unwrap();
        assert!(!path.exists());
        assert!(start(root).is_err());
        owned.close().unwrap();
    }
    #[test]
    fn display_identity_requires_exact_niri_socket_shape() {
        assert_eq!(
            display(Some(Path::new("/run/user/1000/niri.wayland-9.123.sock"))).unwrap(),
            "wayland-9"
        );
        assert!(display(Some(Path::new("/tmp/arbitrary.sock"))).is_err());
    }
    #[test]
    fn caller_writable_executable_is_rejected() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(trusted(file.path()).is_err());
    }
    #[test]
    fn process_ancestry_is_generation_bound() {
        let pid = std::process::id();
        let generation = start(pid).unwrap();
        assert!(descends(pid, pid, generation));
        assert!(!descends(pid, pid, generation + 1));
    }
}
