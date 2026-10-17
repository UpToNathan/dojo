//! libpam loaded at run time, so no Dojo binary gains a link-time PAM dependency.
//! Every raw PAM constant Dojo uses is defined here, once.

use std::ffi::{CStr, c_char, c_int, c_void};

pub(crate) const PAM_SUCCESS: c_int = 0;
pub(crate) const PAM_BUF_ERR: c_int = 5;
pub(crate) const PAM_AUTH_ERR: c_int = 7;
pub(crate) const PAM_AUTHINFO_UNAVAIL: c_int = 9;
pub(crate) const PAM_MAXTRIES: c_int = 11;
pub(crate) const PAM_CONV_ERR: c_int = 19;

pub(super) const PAM_PROMPT_ECHO_OFF: c_int = 1;
pub(super) const PAM_PROMPT_ECHO_ON: c_int = 2;
pub(super) const PAM_ERROR_MSG: c_int = 3;
pub(super) const PAM_TEXT_INFO: c_int = 4;

pub(super) const PAM_DISALLOW_NULL_AUTHTOK: c_int = 0x1;
pub(super) const PAM_MAX_NUM_MSG: c_int = 32;

#[repr(C)]
pub(super) struct PamMessage {
    pub msg_style: c_int,
    pub msg: *const c_char,
}

#[repr(C)]
pub(super) struct PamResponse {
    pub resp: *mut c_char,
    pub resp_retcode: c_int,
}

pub(super) type Converse = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int;

#[repr(C)]
pub(super) struct PamConv {
    pub conv: Converse,
    pub appdata_ptr: *mut c_void,
}

type Start = unsafe extern "C" fn(
    service: *const c_char,
    user: *const c_char,
    conv: *const PamConv,
    handle: *mut *mut c_void,
) -> c_int;
#[cfg(feature = "fake-pam-stack")]
type StartConfdir = unsafe extern "C" fn(
    service: *const c_char,
    user: *const c_char,
    conv: *const PamConv,
    confdir: *const c_char,
    handle: *mut *mut c_void,
) -> c_int;
type Authenticate = unsafe extern "C" fn(handle: *mut c_void, flags: c_int) -> c_int;
type End = unsafe extern "C" fn(handle: *mut c_void, status: c_int) -> c_int;

pub(super) struct LibPam {
    start: Start,
    #[cfg(feature = "fake-pam-stack")]
    start_confdir: StartConfdir,
    authenticate: Authenticate,
    end: End,
}

impl LibPam {
    /// Never closed: the helper exits once its one conversation is over.
    pub fn open() -> Result<LibPam, String> {
        unsafe {
            let library = libc::dlopen(c"libpam.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if library.is_null() {
                return Err(dlerror());
            }
            Ok(LibPam {
                start: std::mem::transmute::<*mut c_void, Start>(symbol(library, c"pam_start")?),
                #[cfg(feature = "fake-pam-stack")]
                start_confdir: std::mem::transmute::<*mut c_void, StartConfdir>(symbol(
                    library,
                    c"pam_start_confdir",
                )?),
                authenticate: std::mem::transmute::<*mut c_void, Authenticate>(symbol(
                    library,
                    c"pam_authenticate",
                )?),
                end: std::mem::transmute::<*mut c_void, End>(symbol(library, c"pam_end")?),
            })
        }
    }

    /// Runs one authentication and ends the handle. Returns the PAM code.
    pub fn authenticate(
        &self,
        service: &CStr,
        user: &CStr,
        conv: &PamConv,
        #[cfg(feature = "fake-pam-stack")] confdir: Option<&CStr>,
    ) -> c_int {
        let mut handle = std::ptr::null_mut();
        #[cfg(feature = "fake-pam-stack")]
        let started = match confdir {
            Some(confdir) => unsafe {
                (self.start_confdir)(
                    service.as_ptr(),
                    user.as_ptr(),
                    conv,
                    confdir.as_ptr(),
                    &mut handle,
                )
            },
            None => unsafe { (self.start)(service.as_ptr(), user.as_ptr(), conv, &mut handle) },
        };
        #[cfg(not(feature = "fake-pam-stack"))]
        let started = unsafe { (self.start)(service.as_ptr(), user.as_ptr(), conv, &mut handle) };
        if started != PAM_SUCCESS {
            return started;
        }
        unsafe {
            let code = (self.authenticate)(handle, PAM_DISALLOW_NULL_AUTHTOK);
            (self.end)(handle, code);
            code
        }
    }
}

unsafe fn symbol(library: *mut c_void, name: &CStr) -> Result<*mut c_void, String> {
    let address = unsafe { libc::dlsym(library, name.as_ptr()) };
    if address.is_null() {
        Err(dlerror())
    } else {
        Ok(address)
    }
}

fn dlerror() -> String {
    let message = unsafe { libc::dlerror() };
    if message.is_null() {
        "unknown dlopen error".into()
    } else {
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    }
}
