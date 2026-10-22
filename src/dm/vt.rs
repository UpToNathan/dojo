//! The virtual terminal the display manager owns. [`Vt`] is the number and what PAM and
//! the session are told about it; [`Console`] is the one thing the service does to it,
//! behind a trait so no test switches a real console.

use std::ffi::c_int;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

/// A Linux virtual terminal, 1 to 63.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Vt(u8);

impl Vt {
    pub fn new(number: u8) -> Option<Vt> {
        (1..=63).contains(&number).then_some(Vt(number))
    }

    /// `/dev/tty3` or `tty3`.
    pub fn from_tty_name(name: &str) -> Option<Vt> {
        let digits = name
            .strip_prefix("/dev/")
            .unwrap_or(name)
            .strip_prefix("tty")?;
        let plain = (1..=2).contains(&digits.len()) && digits.bytes().all(|c| c.is_ascii_digit());
        plain
            .then(|| digits.parse().ok().and_then(Vt::new))
            .flatten()
    }

    pub fn number(self) -> u8 {
        self.0
    }

    /// `PAM_TTY`.
    pub fn tty(self) -> String {
        format!("tty{}", self.0)
    }
}

/// Makes the terminal the visible one with nothing left over from the last session.
pub trait Console {
    fn vt(&self) -> Vt;
    fn acquire(&mut self) -> io::Result<()>;
    /// Puts the terminal in graphics mode, so the kernel's console does not paint over the
    /// screen while nothing draws on it. Meant for the gap between the greeter and the session;
    /// [`acquire`](Self::acquire) puts it back in text mode for the next greeter.
    fn graphics(&mut self) -> io::Result<()>;
}

const VT_ACTIVATE: u64 = 0x5606;
const VT_WAITACTIVE: u64 = 0x5607;
const KDSETMODE: u64 = 0x4B3A;
const KD_TEXT: c_int = 0;
const KD_GRAPHICS: c_int = 1;

/// The terminal systemd opened as standard input (`TTYPath=/dev/tty1`).
pub struct StdinConsole {
    vt: Vt,
}

impl StdinConsole {
    pub fn open() -> io::Result<StdinConsole> {
        let mut name = [0 as libc::c_char; 64];
        if unsafe { libc::ttyname_r(0, name.as_mut_ptr(), name.len()) } != 0 {
            return Err(io::Error::other("standard input is not a terminal"));
        }
        let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) }.to_string_lossy();
        Vt::from_tty_name(&name)
            .map(|vt| StdinConsole { vt })
            .ok_or_else(|| io::Error::other(format!("{name} is not a virtual terminal")))
    }
}

fn ioctl(request: u64, argument: c_int) -> io::Result<()> {
    let stdin = unsafe { BorrowedFd::borrow_raw(0) };
    if unsafe { libc::ioctl(stdin.as_raw_fd(), request as _, argument) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl Console for StdinConsole {
    fn vt(&self) -> Vt {
        self.vt
    }

    fn acquire(&mut self) -> io::Result<()> {
        let number = c_int::from(self.vt.number());
        ioctl(VT_ACTIVATE, number)?;
        ioctl(VT_WAITACTIVE, number)?;
        // A compositor that died leaves the terminal in graphics mode.
        ioctl(KDSETMODE, KD_TEXT)
    }

    fn graphics(&mut self) -> io::Result<()> {
        ioctl(KDSETMODE, KD_GRAPHICS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_are_numbered_one_to_sixty_three() {
        assert_eq!(Vt::from_tty_name("/dev/tty1").map(Vt::number), Some(1));
        assert_eq!(Vt::from_tty_name("tty12").map(Vt::number), Some(12));
        for bad in [
            "",
            "tty",
            "tty0",
            "tty64",
            "tty123",
            "ttyS0",
            "/dev/pts/3",
            "/dev/tty1/x",
            "/dev/ttyX",
            "../tty1",
            "tty-1",
            "tty+1",
        ] {
            assert_eq!(Vt::from_tty_name(bad), None, "{bad:?}");
        }
        assert_eq!(Vt::new(2).unwrap().tty(), "tty2");
        assert_eq!(Vt::new(0), None);
    }
}
