//! Ephemeral child enrollment by an explicitly selected launch attester.
//! Process identity and enrollment never authorize an individual UI effect.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{
    cell::Cell,
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::net::UnixStream,
    },
    path::PathBuf,
    rc::Rc,
};

pub(super) struct LauncherEnrollment {
    pid: u32,
    start: u64,
    recipe: String,
    handle: Rc<OwnedFd>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChildClaim {
    pub version: u32,
    pub child_pid: u32,
    pub child_start: u64,
    pub recipe_sha256: String,
    pub compositor_pid: u32,
    pub compositor_start: u64,
}
pub(super) struct RegisteredChild {
    pid: u32,
    start: u64,
    pidfd: OwnedFd,
    control: UnixStream,
    revoked: Cell<bool>,
    launcher: Rc<OwnedFd>,
}
impl LauncherEnrollment {
    pub(super) fn open(pid: u32, start: u64, recipe: String) -> Result<Self> {
        ensure!(
            pid > 0 && process(pid)?.1 == start,
            "launch attester generation mismatch"
        );
        ensure!(
            recipe.len() == 64
                && recipe
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "invalid enrolled recipe identity"
        );
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        ensure!(raw >= 0, "cannot monitor enrolled launch attester");
        let handle = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        ensure!(
            pidfd_live(&handle) && process(pid)?.1 == start,
            "launch attester retired during enrollment"
        );
        Ok(Self {
            pid,
            start,
            recipe,
            handle: Rc::new(handle),
        })
    }

    pub(super) fn accept(
        &self,
        sender: libc::ucred,
        claim: ChildClaim,
        pidfd: OwnedFd,
        control: OwnedFd,
    ) -> Result<RegisteredChild> {
        ensure!(
            sender.pid > 0
                && sender.pid as u32 == self.pid
                && sender.uid == unsafe { libc::geteuid() }
                && pidfd_live(&self.handle)
                && process(self.pid)?.1 == self.start,
            "child enrollment sender is not the selected attester"
        );
        ensure!(
            claim.version == 1
                && claim.recipe_sha256 == self.recipe
                && claim.compositor_pid == std::process::id()
                && claim.compositor_start == process(std::process::id())?.1,
            "child enrollment recipe or compositor differs"
        );
        ensure!(
            claim.child_pid > 0 && pidfd_live(&pidfd),
            "enrolled child is not alive"
        );
        ensure!(
            std::fs::read_link(format!("/proc/self/fd/{}", pidfd.as_raw_fd()))?
                == PathBuf::from("anon_inode:[pidfd]"),
            "enrollment requires a process handle"
        );
        let info = bounded(&format!("/proc/self/fdinfo/{}", pidfd.as_raw_fd()))?;
        let handle_pid = info
            .lines()
            .find_map(|line| line.strip_prefix("Pid:"))
            .and_then(|pid| pid.trim().parse::<u32>().ok());
        ensure!(
            handle_pid == Some(claim.child_pid),
            "process handle names another child"
        );
        let (parent, start) = process(claim.child_pid)?;
        ensure!(
            parent == self.pid && start == claim.child_start && pidfd_live(&pidfd),
            "child was not spawned by this attester generation"
        );
        let control = UnixStream::from(control);
        let mut domain = 0i32;
        let mut length = std::mem::size_of_val(&domain) as libc::socklen_t;
        ensure!(
            unsafe {
                libc::getsockopt(
                    control.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_DOMAIN,
                    (&mut domain as *mut i32).cast(),
                    &mut length,
                )
            } == 0
                && domain == libc::AF_UNIX,
            "child control is not a Unix stream"
        );
        let mut kind = 0i32;
        ensure!(
            unsafe {
                libc::getsockopt(
                    control.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_TYPE,
                    (&mut kind as *mut i32).cast(),
                    &mut length,
                )
            } == 0
                && kind == libc::SOCK_STREAM,
            "child control has the wrong socket type"
        );
        control.set_nonblocking(true)?;
        Ok(RegisteredChild {
            pid: claim.child_pid,
            start,
            pidfd,
            control,
            revoked: Cell::new(false),
            launcher: self.handle.clone(),
        })
    }
}
impl RegisteredChild {
    pub(super) fn matches(&self, sender: libc::ucred) -> bool {
        sender.pid > 0
            && sender.pid as u32 == self.pid
            && sender.uid == unsafe { libc::geteuid() }
            && self.live()
            && process(self.pid).is_ok_and(|(_, start)| start == self.start)
    }
    pub(super) fn process_identity(&self) -> (u32, u64) {
        (self.pid, self.start)
    }
    pub(super) fn live(&self) -> bool {
        if self.revoked.get() || !pidfd_live(&self.launcher) || !pidfd_live(&self.pidfd) {
            return false;
        }
        let mut byte = 0u8;
        let size = unsafe {
            libc::recv(
                self.control.as_raw_fd(),
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        size > 0
            || (size < 0
                && std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock)
    }
    pub(super) fn revoke(&self) {
        self.revoked.set(true);
        let _ = self.control.shutdown(std::net::Shutdown::Both);
    }
}
fn pidfd_live(handle: &OwnedFd) -> bool {
    let mut descriptor = libc::pollfd {
        fd: handle.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut descriptor, 1, 0) == 0 }
}
fn bounded(path: &str) -> Result<String> {
    let mut bytes = Vec::new();
    File::open(path)?.take(8193).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 8192,
        "enrollment process metadata exceeds limit"
    );
    String::from_utf8(bytes).map_err(Into::into)
}
fn process(pid: u32) -> Result<(u32, u64)> {
    let text = bounded(&format!("/proc/{pid}/stat"))?;
    let fields = text
        .rsplit_once(')')
        .context("invalid enrolled process metadata")?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    Ok((
        fields
            .get(1)
            .context("process parent unavailable")?
            .parse()?,
        fields
            .get(19)
            .context("process generation unavailable")?
            .parse()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        os::fd::FromRawFd,
        process::{Command, Stdio},
    };
    struct ChildFixture {
        child: std::process::Child,
        control: Option<UnixStream>,
    }
    impl Drop for ChildFixture {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    fn child() -> ChildFixture {
        let (mut control, inherited) = UnixStream::pair().unwrap();
        let inherited: OwnedFd = inherited.into();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "external_admission::registration::tests::child_control_probe",
                "--ignored",
            ])
            .stdin(Stdio::from(inherited))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        control
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        control.read_exact(&mut [0]).unwrap();
        ChildFixture {
            child,
            control: Some(control),
        }
    }
    fn pidfd(pid: u32) -> OwnedFd {
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        assert!(raw >= 0);
        unsafe { OwnedFd::from_raw_fd(raw as i32) }
    }
    fn sender() -> libc::ucred {
        libc::ucred {
            pid: std::process::id() as i32,
            uid: unsafe { libc::geteuid() },
            gid: unsafe { libc::getegid() },
        }
    }
    fn claim(fixture: &ChildFixture) -> ChildClaim {
        ChildClaim {
            version: 1,
            child_pid: fixture.child.id(),
            child_start: process(fixture.child.id()).unwrap().1,
            recipe_sha256: "a".repeat(64),
            compositor_pid: std::process::id(),
            compositor_start: process(std::process::id()).unwrap().1,
        }
    }
    fn enrollment() -> LauncherEnrollment {
        LauncherEnrollment::open(
            std::process::id(),
            process(std::process::id()).unwrap().1,
            "a".repeat(64),
        )
        .unwrap()
    }
    #[test]
    #[ignore = "child control fixture, launched only by enrollment tests"]
    fn child_control_probe() {
        let mut control = unsafe { UnixStream::from_raw_fd(0) };
        control.write_all(b"r").unwrap();
        let _ = control.read(&mut [0]);
    }
    #[test]
    fn selected_attester_enrolls_only_its_live_child_and_revocation_is_final() {
        let mut fixture = child();
        let claim = claim(&fixture);
        let owner = (claim.child_pid, claim.child_start);
        let registered = enrollment()
            .accept(
                sender(),
                claim,
                pidfd(fixture.child.id()),
                fixture.control.take().unwrap().into(),
            )
            .unwrap();
        assert!(registered.live());
        assert_eq!(registered.process_identity(), owner);
        assert!(registered.matches(libc::ucred {
            pid: owner.0 as i32,
            ..sender()
        }));
        assert!(!registered.matches(sender()));
        registered.revoke();
        assert!(!registered.live());
        registered.revoke();
        assert!(!registered.live());
    }
    #[test]
    fn forged_sender_recipe_and_process_handle_cannot_enroll() {
        let mut fixture = child();
        let raw = claim(&fixture);
        assert!(enrollment()
            .accept(
                libc::ucred {
                    pid: sender().pid + 1,
                    ..sender()
                },
                raw,
                pidfd(fixture.child.id()),
                fixture.control.take().unwrap().into()
            )
            .is_err());
        let mut fixture = child();
        let mut raw = claim(&fixture);
        raw.recipe_sha256 = "b".repeat(64);
        assert!(enrollment()
            .accept(
                sender(),
                raw,
                pidfd(fixture.child.id()),
                fixture.control.take().unwrap().into()
            )
            .is_err());
        let mut fixture = child();
        let raw = claim(&fixture);
        assert!(enrollment()
            .accept(
                sender(),
                raw,
                pidfd(std::process::id()),
                fixture.control.take().unwrap().into()
            )
            .is_err());
    }
    #[test]
    fn child_death_invalidates_registration_even_before_control_event_dispatch() {
        let mut fixture = child();
        let raw = claim(&fixture);
        let registered = enrollment()
            .accept(
                sender(),
                raw,
                pidfd(fixture.child.id()),
                fixture.control.take().unwrap().into(),
            )
            .unwrap();
        fixture.child.kill().unwrap();
        fixture.child.wait().unwrap();
        assert!(!registered.live());
    }
    #[test]
    fn enrollment_receives_kernel_credentials_and_both_handles_over_the_wire() {
        use std::os::unix::net::UnixDatagram;
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        let enabled: i32 = 1;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    receiver.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PASSCRED,
                    (&enabled as *const i32).cast(),
                    std::mem::size_of_val(&enabled) as _,
                )
            },
            0
        );
        let mut fixture = child();
        let identity = claim(&fixture);
        let payload = serde_json::to_vec(&serde_json::json!({
            "version":identity.version,"child_pid":identity.child_pid,"child_start":identity.child_start,
            "recipe_sha256":identity.recipe_sha256,"compositor_pid":identity.compositor_pid,
            "compositor_start":identity.compositor_start
        })).unwrap();
        let handle = pidfd(fixture.child.id());
        let control = fixture.control.take().unwrap();
        let descriptors = [handle.as_raw_fd(), control.as_raw_fd()];
        let mut iov = libc::iovec {
            iov_base: payload.as_ptr().cast_mut().cast(),
            iov_len: payload.len(),
        };
        let mut ancillary = [0usize; 8];
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = ancillary.as_mut_ptr().cast();
        message.msg_controllen =
            unsafe { libc::CMSG_SPACE(std::mem::size_of_val(&descriptors) as _) } as usize;
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&message);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(std::mem::size_of_val(&descriptors) as _) as usize;
            std::ptr::copy_nonoverlapping(descriptors.as_ptr(), libc::CMSG_DATA(header).cast(), 2);
        }
        assert_eq!(
            unsafe { libc::sendmsg(sender.as_raw_fd(), &message, libc::MSG_NOSIGNAL) },
            payload.len() as isize
        );
        // Transfer the only control endpoint. The receiver owns both references.
        drop(control);
        drop(handle);
        let (raw, mut handles, credentials, _) =
            super::super::receive_descriptors(receiver.as_raw_fd(), 2)
                .unwrap()
                .unwrap();
        assert_eq!(credentials.pid, std::process::id() as i32);
        assert_eq!(handles.len(), 2);
        for descriptor in &handles {
            assert_ne!(
                unsafe { libc::fcntl(descriptor.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
                0
            );
        }
        let control = handles.pop().unwrap();
        let handle = handles.pop().unwrap();
        let registered = enrollment()
            .accept(
                credentials,
                serde_json::from_slice(&raw).unwrap(),
                handle,
                control,
            )
            .unwrap();
        assert_eq!(
            registered.process_identity(),
            (identity.child_pid, identity.child_start)
        );
        assert!(registered.live());
    }
    #[test]
    fn registered_child_cannot_use_another_process_native_peer() {
        use std::io::Read;
        let mut child = child();
        let identity = claim(&child);
        let registered = enrollment()
            .accept(
                sender(),
                identity,
                pidfd(child.child.id()),
                child.control.take().unwrap().into(),
            )
            .unwrap();
        let mut fixture = crate::tests::fixture::Fixture::with_agent_input();
        fixture.add_output(1, (800, 600));
        let (_, _, target) = super::super::tests::fixture_window(&mut fixture);
        let (mut writer, mut browser) = super::super::tests::fixture_writer(&mut fixture, target);
        writer.owner = Some(Rc::new(registered));
        assert!(matches!(
            writer.ready(fixture.niri_state()),
            calloop::PostAction::Remove
        ));
        assert_eq!(
            browser.read(&mut [0]).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(!fixture.niri_state().external_target_unresolved(target));
    }
}
