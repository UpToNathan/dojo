//! Look snapshots for the login screen: a child that has dropped to the user reads their
//! home, the root parent validates what the child sends and writes it under the state
//! directory. The parent never opens anything under a home directory, so a user cannot
//! point the display manager's own privilege at a file by way of a link or a FIFO.
//!
//! The pipe from child to parent is a list of records, one per snapshot file the child
//! has something to say about: the file's index, a status, and for a file that was
//! present, its bytes. The parent re-validates every piece with [`Look::decode`] and
//! takes the full name from passwd itself.

use std::ffi::c_int;
use std::io::{self, Read as _, Write as _};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::account::Account;
use crate::session::snapshot::{
    self, Collector, Decoded, File, Links, Login, Look, ReadError, TEXT_CAP,
};

/// Everything a well-behaved child can send: the five caps plus record headers.
pub const MAX_PIPE_BYTES: usize =
    (snapshot::PALETTE_CAP + snapshot::WALLPAPER_CAP + snapshot::SETTINGS_CAP + 2 * TEXT_CAP)
        as usize
        + 64;
const PRESENT: u8 = 1;
const REJECTED: u8 = 2;
/// A child that has not finished by now is stuck on a file system and is killed.
const CHILD_SECONDS: u32 = 15;
/// What the child may map beyond the address space it inherits at fork. The limit is on
/// virtual size, which already counts every thread stack and allocator arena of the parent,
/// so a fixed number would be too small in one process and no bound in another. The
/// wallpaper decoder is held to 16 MiB; this is for a decoder bug, and is still far below
/// what it takes to push the machine into swap.
const CHILD_HEADROOM: u64 = 1 << 30;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Piece {
    Present(Vec<u8>),
    /// The file was there and the child would not accept it.
    Rejected,
}

fn file_index(file: File) -> u8 {
    File::ALL.iter().position(|f| *f == file).unwrap_or(0) as u8
}

/// The child's side: the pieces of what it collected. Identity is not among them.
pub fn encode_pieces(decoded: &Decoded) -> Vec<u8> {
    let mut out = Vec::new();
    for (file, bytes) in decoded.look.encode() {
        if file == File::Identity {
            continue;
        }
        out.extend([file_index(file), PRESENT]);
        out.extend((bytes.len() as u32).to_le_bytes());
        out.extend(bytes);
    }
    for problem in &decoded.rejected {
        if problem.file != File::Identity {
            out.extend([file_index(problem.file), REJECTED]);
            out.extend(0u32.to_le_bytes());
        }
    }
    out
}

/// The parent's side. `None` for anything the child could not have meant: a repeated or
/// unknown file, an unknown status, a cut record, a piece over its file's cap.
pub fn decode_pieces(mut bytes: &[u8]) -> Option<Vec<(File, Piece)>> {
    let mut pieces: Vec<(File, Piece)> = Vec::new();
    while !bytes.is_empty() {
        let (header, rest) = bytes.split_at_checked(6)?;
        let file = *File::ALL.get(usize::from(header[0]))?;
        if file == File::Identity || pieces.iter().any(|(seen, _)| *seen == file) {
            return None;
        }
        let length = u32::from_le_bytes(header[2..6].try_into().ok()?) as usize;
        let (body, rest) = rest.split_at_checked(length)?;
        let piece = match (header[1], length) {
            (PRESENT, length) if length as u64 <= file.cap() => Piece::Present(body.to_vec()),
            (REJECTED, 0) => Piece::Rejected,
            _ => return None,
        };
        pieces.push((file, piece));
        bytes = rest;
    }
    Some(pieces)
}

/// The look to store: the pieces validated again, the full name from passwd, and the
/// previous snapshot's piece for any file the child rejected.
pub fn assemble(
    login: Login,
    gecos: Option<&str>,
    pieces: &[(File, Piece)],
    previous: Option<&Look>,
) -> Look {
    let mut decoded = Look::decode(login, |file| {
        match pieces.iter().find(|(seen, _)| *seen == file) {
            None => Ok(None),
            Some((_, Piece::Present(bytes))) => Ok(Some(bytes.clone())),
            Some((_, Piece::Rejected)) => Err(ReadError::NotRegular),
        }
    });
    decoded
        .rejected
        .retain(|problem| problem.file != File::Identity);
    let name = gecos.and_then(snapshot::full_name);
    let mut look = decoded.merged_over(previous);
    look.full_name = name;
    look
}

/// Where snapshots go and who may read them.
#[derive(Clone, Debug)]
pub struct Snapshots {
    root: PathBuf,
    group: Option<u32>,
}

impl Snapshots {
    pub fn new(root: PathBuf, group: Option<u32>) -> Snapshots {
        Snapshots { root, group }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory the greeter's group reads, created once with its group.
    pub fn prepare(&self) -> io::Result<()> {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        let users = self.root.join("users");
        if !users.is_dir() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o750)
                .create(&users)?;
            std::fs::set_permissions(&users, std::fs::Permissions::from_mode(0o750))?;
        }
        if let Some(group) = self.group {
            std::os::unix::fs::chown(&users, None, Some(group))?;
        }
        Ok(())
    }

    /// Reads the user's files as the user, then stores the result. On any failure the
    /// previous snapshot stays as it was.
    pub fn refresh(&self, account: &Account, gecos: Option<&str>) -> io::Result<()> {
        let bytes = collect_as(account, gecos)?;
        let pieces = decode_pieces(&bytes)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed snapshot"))?;
        let previous = snapshot::load(&self.root, account.login.as_str());
        let look = assemble(account.login.clone(), gecos, &pieces, previous.as_ref());
        snapshot::store_shared(&self.root, &look, self.group)
    }
}

/// Becomes the user. As root this is `initgroups`, `setgid`, `setuid` and a check that
/// root cannot come back; as the user themselves (test builds only) there is nothing to do.
fn drop_to(account: &Account) -> io::Result<()> {
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        let same = euid == account.uid && unsafe { libc::getegid() } == account.gid;
        return if cfg!(feature = "fake-pam-stack") && same {
            Ok(())
        } else {
            Err(io::ErrorKind::PermissionDenied.into())
        };
    }
    let failed = unsafe {
        libc::initgroups(account.name.as_ptr(), account.gid) < 0
            || libc::setgid(account.gid) < 0
            || libc::setuid(account.uid) < 0
            || (account.uid != 0 && libc::setuid(0) == 0)
    };
    if failed {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn collect_as(account: &Account, gecos: Option<&str>) -> io::Result<Vec<u8>> {
    let mut fds = [0 as c_int; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()),
        0 => {
            drop(read);
            collector_child(write, account, gecos)
        }
        child => read_child(child, read, write),
    }
}

/// The address-space limit for a process whose `/proc/self/statm` reads `statm`.
fn address_space_limit(statm: &str, page_size: u64) -> Option<u64> {
    let pages: u64 = statm.split_whitespace().next()?.parse().ok()?;
    pages.checked_mul(page_size)?.checked_add(CHILD_HEADROOM)
}

/// Caps this process's address space, soft and hard, so that a decoder bug in the collector
/// fails an allocation instead of eating the machine.
fn bound_address_space() -> io::Result<()> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let limit = std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|statm| address_space_limit(&statm, u64::try_from(page_size).ok()?))
        .ok_or_else(|| io::Error::other("cannot size the child's address space"))?;
    let limit = libc::rlimit {
        rlim_cur: limit,
        rlim_max: limit,
    };
    if unsafe { libc::setrlimit(libc::RLIMIT_AS, &limit) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The forked child. It never returns and never unwinds.
fn collector_child(write: OwnedFd, account: &Account, gecos: Option<&str>) -> ! {
    // Nothing the parent holds open, the greeter's socket above all, stays in a process
    // that is about to read a user's files.
    unsafe {
        libc::dup2(write.as_raw_fd(), 3);
        libc::syscall(libc::SYS_close_range, 4u32, u32::MAX, 0u32);
        libc::alarm(CHILD_SECONDS);
    }
    let result = (|| -> io::Result<()> {
        bound_address_space()?;
        drop_to(account)?;
        let collector = Collector::open(&account.home, Links::Follow)?;
        let decoded = collector.collect(account.login.clone(), gecos);
        let mut pipe = unsafe { std::fs::File::from_raw_fd(3) };
        pipe.write_all(&encode_pieces(&decoded))
    })();
    unsafe { libc::_exit(if result.is_ok() { 0 } else { 1 }) }
}

fn read_child(child: libc::pid_t, read: OwnedFd, write: OwnedFd) -> io::Result<Vec<u8>> {
    drop(write);
    let deadline = Instant::now() + Duration::from_secs(u64::from(CHILD_SECONDS) + 5);
    let mut file = std::fs::File::from(read);
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    let outcome = loop {
        let now = Instant::now();
        if now >= deadline {
            break Err(io::ErrorKind::TimedOut.into());
        }
        let mut poll = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let wait = (deadline - now).as_millis().min(1000) as c_int;
        match unsafe { libc::poll(&mut poll, 1, wait) } {
            0 => continue,
            n if n < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted => {
                break Err(io::Error::last_os_error());
            }
            n if n < 0 => continue,
            _ => {}
        }
        match file.read(&mut chunk) {
            Ok(0) => break Ok(()),
            Ok(count) => {
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.len() > MAX_PIPE_BYTES {
                    break Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "snapshot too large",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => break Err(error),
        }
    };
    if outcome.is_err() {
        unsafe { libc::kill(child, libc::SIGKILL) };
    }
    let mut status = 0;
    while unsafe { libc::waitpid(child, &mut status, 0) } < 0
        && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
    {}
    outcome?;
    if !libc::WIFEXITED(status) || libc::WEXITSTATUS(status) != 0 {
        return Err(io::Error::other("the snapshot child failed"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
