//! The `dojo-lock-pam` process: one PAM conversation for the current user,
//! spoken over the socket on fd 3.

use std::ffi::{CStr, CString, c_int, c_void};
use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd};

use super::libpam::{self, LibPam, PamConv, PamMessage, PamResponse};
use super::{WORKER_FD, wire};
use crate::lock::pam::{MessageStyle, Method};
use crate::lock::secret::protect_process;

const EXIT_USAGE: i32 = 64;
const EXIT_UNAVAILABLE: i32 = 69;
const EXIT_SOFTWARE: i32 = 70;

struct Invocation {
    method: Method,
    #[cfg(feature = "fake-pam-stack")]
    confdir: Option<CString>,
}

enum Command {
    /// `--check`: load libpam and resolve its symbols, nothing else.
    Check,
    Authenticate(Invocation),
}

/// The body of `src/bin/dojo-lock-pam.rs`.
pub fn run_helper() -> ! {
    let command = parse_command(std::env::args_os().skip(1).collect());
    if let Some(Command::Check) = command {
        let (report, code) = check_report(LibPam::open().map(drop));
        match code {
            0 => println!("{report}"),
            _ => eprintln!("{report}"),
        }
        std::process::exit(code);
    }
    protect_process();
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL, 0, 0, 0);
        // Orphans of PAM children reparent here, so the parent's tree snapshot still sees them.
        libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
        if libc::getppid() == 1 {
            libc::_exit(EXIT_UNAVAILABLE);
        }
    }
    let Some(Command::Authenticate(invocation)) = command else {
        std::process::exit(EXIT_USAGE);
    };
    let Some(socket) = take_socket() else {
        std::process::exit(EXIT_USAGE);
    };
    let Some(user) = current_user() else {
        std::process::exit(EXIT_UNAVAILABLE);
    };
    let Ok(pam) = LibPam::open() else {
        std::process::exit(EXIT_UNAVAILABLE);
    };

    let service = CString::new(invocation.method.service()).expect("service names have no NUL");
    let conversation = Conversation {
        socket: socket.as_fd(),
    };
    let conv = PamConv {
        conv: converse,
        appdata_ptr: (&conversation as *const Conversation).cast_mut().cast(),
    };
    let code = pam.authenticate(
        &service,
        &user,
        &conv,
        #[cfg(feature = "fake-pam-stack")]
        invocation.confdir.as_deref(),
    );
    match wire::send(socket.as_fd(), &wire::done(code)) {
        Ok(()) => std::process::exit(0),
        Err(_) => std::process::exit(EXIT_SOFTWARE),
    }
}

/// The `--check` report and exit status. A failure names the dlopen or dlsym step libc reported.
fn check_report(loaded: Result<(), String>) -> (String, i32) {
    match loaded {
        Ok(()) => ("ok".into(), 0),
        Err(step) => (format!("libpam unavailable: {step}"), EXIT_UNAVAILABLE),
    }
}

/// `--check` is accepted only as the sole argument; everything else is the service-name form.
fn parse_command(arguments: Vec<std::ffi::OsString>) -> Option<Command> {
    if arguments.len() == 1 && arguments[0] == "--check" {
        return Some(Command::Check);
    }
    parse_arguments(arguments).map(Command::Authenticate)
}

/// argv is the service name and nothing else; test builds also accept `--pam-confdir DIR` first.
fn parse_arguments(arguments: Vec<std::ffi::OsString>) -> Option<Invocation> {
    let arguments: Vec<&str> = arguments
        .iter()
        .map(|argument| argument.to_str())
        .collect::<Option<_>>()?;
    match arguments.as_slice() {
        [service] => Some(Invocation {
            method: Method::from_service(service)?,
            #[cfg(feature = "fake-pam-stack")]
            confdir: None,
        }),
        #[cfg(feature = "fake-pam-stack")]
        ["--pam-confdir", confdir, service] => Some(Invocation {
            method: Method::from_service(service)?,
            confdir: Some(CString::new(*confdir).ok()?),
        }),
        _ => None,
    }
}

/// fd 3 must be the parent's SEQPACKET socket. It is made close-on-exec so PAM children never see it.
fn take_socket() -> Option<OwnedFd> {
    let mut kind: c_int = 0;
    let mut length = size_of::<c_int>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            WORKER_FD,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut kind as *mut c_int).cast(),
            &mut length,
        )
    };
    if rc != 0 || kind != libc::SOCK_SEQPACKET {
        return None;
    }
    if unsafe { libc::fcntl(WORKER_FD, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return None;
    }
    Some(unsafe { OwnedFd::from_raw_fd(WORKER_FD) })
}

/// The PAM user is whoever runs the helper. It never comes from argv.
fn current_user() -> Option<CString> {
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    let rc = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if rc != 0 || found.is_null() || entry.pw_name.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(entry.pw_name) }.to_owned())
}

struct Conversation<'a> {
    socket: BorrowedFd<'a>,
}

impl Conversation<'_> {
    /// Asks the parent, then hands PAM a malloc'd NUL-terminated copy, which PAM owns.
    /// The received buffer is zeroed when `answer` drops at the end of this call.
    fn prompt(&self, echo: bool) -> Option<*mut libc::c_char> {
        wire::send(self.socket, &wire::prompt(echo)).ok()?;
        let answer = wire::Answer::receive(self.socket).ok()?;
        let secret = answer.secret();
        let copy = unsafe { libc::malloc(secret.len() + 1) }.cast::<u8>();
        if copy.is_null() {
            return None;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(secret.as_ptr(), copy, secret.len());
            *copy.add(secret.len()) = 0;
        }
        Some(copy.cast())
    }

    fn tell(&self, style: MessageStyle, text: &[u8]) -> Option<()> {
        wire::send(self.socket, &wire::text(style, text)).ok()
    }
}

unsafe extern "C" fn converse(
    count: c_int,
    messages: *mut *const PamMessage,
    out: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if count <= 0 || count > libpam::PAM_MAX_NUM_MSG || messages.is_null() || out.is_null() {
        return libpam::PAM_CONV_ERR;
    }
    let count = count as usize;
    let conversation = unsafe { &*appdata.cast::<Conversation>() };
    let replies = unsafe { libc::calloc(count, size_of::<PamResponse>()) }.cast::<PamResponse>();
    if replies.is_null() {
        return libpam::PAM_BUF_ERR;
    }
    for index in 0..count {
        let message = unsafe { &**messages.add(index) };
        let text = if message.msg.is_null() {
            &b""[..]
        } else {
            unsafe { CStr::from_ptr(message.msg) }.to_bytes()
        };
        let handled = match message.msg_style {
            libpam::PAM_PROMPT_ECHO_OFF | libpam::PAM_PROMPT_ECHO_ON => conversation
                .prompt(message.msg_style == libpam::PAM_PROMPT_ECHO_ON)
                .map(|reply| unsafe { (*replies.add(index)).resp = reply }),
            libpam::PAM_ERROR_MSG => conversation.tell(MessageStyle::Error, text),
            libpam::PAM_TEXT_INFO => conversation.tell(MessageStyle::Info, text),
            _ => None,
        };
        if handled.is_none() {
            unsafe { scrub_replies(replies, count) };
            return libpam::PAM_CONV_ERR;
        }
    }
    unsafe { *out = replies };
    libpam::PAM_SUCCESS
}

unsafe fn scrub_replies(replies: *mut PamResponse, count: usize) {
    for index in 0..count {
        let reply = unsafe { (*replies.add(index)).resp };
        if !reply.is_null() {
            let length = unsafe { libc::strlen(reply) };
            zeroize::Zeroize::zeroize(unsafe {
                std::slice::from_raw_parts_mut(reply.cast::<u8>(), length)
            });
            unsafe { libc::free(reply.cast()) };
        }
    }
    unsafe { libc::free(replies.cast()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation(arguments: &[&str]) -> Option<Method> {
        parse_arguments(arguments.iter().map(Into::into).collect())
            .map(|invocation| invocation.method)
    }

    #[test]
    fn argv_is_exactly_one_known_service() {
        assert_eq!(invocation(&["dojo-lock-password"]), Some(Method::Password));
        assert_eq!(
            invocation(&["dojo-lock-fingerprint"]),
            Some(Method::Fingerprint)
        );
        assert_eq!(invocation(&[]), None);
        assert_eq!(invocation(&["sudo"]), None);
        assert_eq!(invocation(&["dojo-lock-password", "nathan"]), None);
    }

    #[test]
    fn check_is_only_accepted_alone() {
        let parse = |arguments: &[&str]| parse_command(arguments.iter().map(Into::into).collect());
        assert!(matches!(parse(&["--check"]), Some(Command::Check)));
        assert!(parse(&["--check", "dojo-lock-password"]).is_none());
        assert!(parse(&["dojo-lock-password", "--check"]).is_none());
        assert!(parse(&["--check", "--check"]).is_none());
        assert!(parse(&["--Check"]).is_none());
        assert!(parse(&[]).is_none());
        assert!(matches!(
            parse(&["dojo-lock-password"]),
            Some(Command::Authenticate(_))
        ));
    }

    #[test]
    fn check_report_names_the_failed_step_and_exits_nonzero() {
        assert_eq!(check_report(Ok(())), ("ok".into(), 0));
        assert_eq!(
            check_report(Err("libpam.so.0: cannot open shared object file".into())),
            (
                "libpam unavailable: libpam.so.0: cannot open shared object file".into(),
                EXIT_UNAVAILABLE
            )
        );
    }

    #[cfg(not(feature = "fake-pam-stack"))]
    #[test]
    fn release_builds_refuse_a_private_confdir() {
        assert_eq!(
            invocation(&["--pam-confdir", "/tmp/x", "dojo-lock-password"]),
            None
        );
    }

    #[cfg(feature = "fake-pam-stack")]
    #[test]
    fn test_builds_accept_a_private_confdir() {
        assert_eq!(
            invocation(&["--pam-confdir", "/tmp/x", "dojo-lock-password"]),
            Some(Method::Password)
        );
        assert_eq!(
            invocation(&["dojo-lock-password", "--pam-confdir", "/tmp/x"]),
            None
        );
    }
}
