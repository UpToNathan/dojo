//! The PAM calls a login makes, behind a trait. The helper's lifecycle runs against any
//! `Pam`, so its order is tested with a recording fake and a private service directory,
//! while the root-only calls (`pam_open_session` with a real stack) stay in the one
//! `Handle` implementation.

use std::ffi::{CString, c_int};

use crate::lock::worker::libpam::{self, Handle, PAM_DELETE_CRED, PAM_ESTABLISH_CRED, PAM_TTY};

pub trait Pam {
    /// `pam_authenticate`. Prompts and messages go to the greeter through the conversation.
    fn authenticate(&mut self) -> c_int;
    /// `pam_acct_mgmt`.
    fn account(&mut self) -> c_int;
    /// `pam_setcred(PAM_ESTABLISH_CRED)`.
    fn establish_credentials(&mut self) -> c_int;
    /// `pam_set_item(PAM_TTY)`.
    fn set_tty(&mut self, tty: &str) -> c_int;
    /// `pam_putenv("NAME=value")`.
    fn put_environment(&mut self, name: &str, value: &str) -> c_int;
    fn open_session(&mut self) -> c_int;
    /// `pam_getenvlist`, as `NAME=value` lines.
    fn environment(&mut self) -> Vec<Vec<u8>>;
    fn close_session(&mut self) -> c_int;
    /// `pam_setcred(PAM_DELETE_CRED)`.
    fn delete_credentials(&mut self) -> c_int;
    /// `pam_end`. Later calls do nothing.
    fn end(&mut self, status: c_int);
}

impl Pam for Handle<'_> {
    fn authenticate(&mut self) -> c_int {
        Handle::authenticate(self)
    }

    fn account(&mut self) -> c_int {
        self.acct_mgmt()
    }

    fn establish_credentials(&mut self) -> c_int {
        self.setcred(PAM_ESTABLISH_CRED)
    }

    fn set_tty(&mut self, tty: &str) -> c_int {
        match CString::new(tty) {
            Ok(tty) => self.set_string_item(PAM_TTY, &tty),
            Err(_) => libpam::PAM_BUF_ERR,
        }
    }

    fn put_environment(&mut self, name: &str, value: &str) -> c_int {
        match CString::new(format!("{name}={value}")) {
            Ok(entry) => self.put_env(&entry),
            Err(_) => libpam::PAM_BUF_ERR,
        }
    }

    fn open_session(&mut self) -> c_int {
        Handle::open_session(self)
    }

    fn environment(&mut self) -> Vec<Vec<u8>> {
        self.env_list()
    }

    fn close_session(&mut self) -> c_int {
        Handle::close_session(self)
    }

    fn delete_credentials(&mut self) -> c_int {
        self.setcred(PAM_DELETE_CRED)
    }

    fn end(&mut self, status: c_int) {
        Handle::end(self, status);
    }
}
