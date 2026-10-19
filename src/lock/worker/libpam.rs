//! libpam loaded at run time, so no Dojo binary gains a link-time PAM dependency.
//! Every raw PAM constant Dojo uses is defined here, once.

use std::ffi::{CStr, c_char, c_int, c_void};

pub(crate) const PAM_SUCCESS: c_int = 0;
pub(crate) const PAM_BUF_ERR: c_int = 5;
pub(crate) const PAM_AUTH_ERR: c_int = 7;
pub(crate) const PAM_AUTHINFO_UNAVAIL: c_int = 9;
pub(crate) const PAM_MAXTRIES: c_int = 11;
pub(crate) const PAM_CONV_ERR: c_int = 19;

pub(crate) const PAM_PROMPT_ECHO_OFF: c_int = 1;
pub(crate) const PAM_PROMPT_ECHO_ON: c_int = 2;
pub(crate) const PAM_ERROR_MSG: c_int = 3;
pub(crate) const PAM_TEXT_INFO: c_int = 4;

pub(crate) const PAM_DISALLOW_NULL_AUTHTOK: c_int = 0x1;
pub(crate) const PAM_ESTABLISH_CRED: c_int = 0x2;
pub(crate) const PAM_DELETE_CRED: c_int = 0x4;
pub(crate) const PAM_TTY: c_int = 3;
pub(crate) const PAM_MAX_NUM_MSG: c_int = 32;

#[repr(C)]
pub(crate) struct PamMessage {
    pub msg_style: c_int,
    pub msg: *const c_char,
}

#[repr(C)]
pub(crate) struct PamResponse {
    pub resp: *mut c_char,
    pub resp_retcode: c_int,
}

pub(crate) type Converse = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int;

#[repr(C)]
pub(crate) struct PamConv {
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
type Flagged = unsafe extern "C" fn(handle: *mut c_void, flags: c_int) -> c_int;
type End = unsafe extern "C" fn(handle: *mut c_void, status: c_int) -> c_int;
type SetItem =
    unsafe extern "C" fn(handle: *mut c_void, item: c_int, value: *const c_void) -> c_int;
type PutEnv = unsafe extern "C" fn(handle: *mut c_void, entry: *const c_char) -> c_int;
type GetEnvList = unsafe extern "C" fn(handle: *mut c_void) -> *mut *mut c_char;

pub(crate) struct LibPam {
    start: Start,
    #[cfg(feature = "fake-pam-stack")]
    start_confdir: StartConfdir,
    authenticate: Flagged,
    acct_mgmt: Flagged,
    setcred: Flagged,
    open_session: Flagged,
    close_session: Flagged,
    set_item: SetItem,
    put_env: PutEnv,
    get_env_list: GetEnvList,
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
                authenticate: flagged(library, c"pam_authenticate")?,
                acct_mgmt: flagged(library, c"pam_acct_mgmt")?,
                setcred: flagged(library, c"pam_setcred")?,
                open_session: flagged(library, c"pam_open_session")?,
                close_session: flagged(library, c"pam_close_session")?,
                set_item: std::mem::transmute::<*mut c_void, SetItem>(symbol(
                    library,
                    c"pam_set_item",
                )?),
                put_env: std::mem::transmute::<*mut c_void, PutEnv>(symbol(
                    library,
                    c"pam_putenv",
                )?),
                get_env_list: std::mem::transmute::<*mut c_void, GetEnvList>(symbol(
                    library,
                    c"pam_getenvlist",
                )?),
                end: std::mem::transmute::<*mut c_void, End>(symbol(library, c"pam_end")?),
            })
        }
    }

    /// Starts a transaction. The caller keeps `conv` and what its `appdata_ptr` points at alive
    /// until the handle ends. The error is the PAM code from `pam_start`.
    pub fn start(
        &self,
        service: &CStr,
        user: &CStr,
        conv: &PamConv,
        #[cfg(feature = "fake-pam-stack")] confdir: Option<&CStr>,
    ) -> Result<Handle<'_>, c_int> {
        let mut raw = std::ptr::null_mut();
        #[cfg(feature = "fake-pam-stack")]
        let started = match confdir {
            Some(confdir) => unsafe {
                (self.start_confdir)(
                    service.as_ptr(),
                    user.as_ptr(),
                    conv,
                    confdir.as_ptr(),
                    &mut raw,
                )
            },
            None => unsafe { (self.start)(service.as_ptr(), user.as_ptr(), conv, &mut raw) },
        };
        #[cfg(not(feature = "fake-pam-stack"))]
        let started = unsafe { (self.start)(service.as_ptr(), user.as_ptr(), conv, &mut raw) };
        if started == PAM_SUCCESS {
            Ok(Handle { pam: self, raw })
        } else {
            Err(started)
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
        let started = self.start(
            service,
            user,
            conv,
            #[cfg(feature = "fake-pam-stack")]
            confdir,
        );
        match started {
            Ok(mut handle) => {
                let code = handle.authenticate();
                handle.end(code);
                code
            }
            Err(code) => code,
        }
    }
}

/// One PAM transaction. Dropping it ends the transaction as a success.
pub(crate) struct Handle<'a> {
    pam: &'a LibPam,
    raw: *mut c_void,
}

impl Handle<'_> {
    pub fn authenticate(&mut self) -> c_int {
        unsafe { (self.pam.authenticate)(self.raw, PAM_DISALLOW_NULL_AUTHTOK) }
    }

    pub fn acct_mgmt(&mut self) -> c_int {
        unsafe { (self.pam.acct_mgmt)(self.raw, PAM_DISALLOW_NULL_AUTHTOK) }
    }

    pub fn setcred(&mut self, flags: c_int) -> c_int {
        unsafe { (self.pam.setcred)(self.raw, flags) }
    }

    pub fn open_session(&mut self) -> c_int {
        unsafe { (self.pam.open_session)(self.raw, 0) }
    }

    pub fn close_session(&mut self) -> c_int {
        unsafe { (self.pam.close_session)(self.raw, 0) }
    }

    /// Items that are C strings, such as `PAM_TTY`. PAM copies the value.
    pub fn set_string_item(&mut self, item: c_int, value: &CStr) -> c_int {
        unsafe { (self.pam.set_item)(self.raw, item, value.as_ptr().cast()) }
    }

    /// `NAME=value`. PAM copies the entry.
    pub fn put_env(&mut self, entry: &CStr) -> c_int {
        unsafe { (self.pam.put_env)(self.raw, entry.as_ptr()) }
    }

    /// The PAM environment as `NAME=value` byte strings, copied out and freed.
    pub fn env_list(&mut self) -> Vec<Vec<u8>> {
        let list = unsafe { (self.pam.get_env_list)(self.raw) };
        if list.is_null() {
            return Vec::new();
        }
        let mut entries = Vec::new();
        let mut index = 0;
        loop {
            let entry = unsafe { *list.add(index) };
            if entry.is_null() {
                break;
            }
            entries.push(unsafe { CStr::from_ptr(entry) }.to_bytes().to_vec());
            unsafe { libc::free(entry.cast()) };
            index += 1;
        }
        unsafe { libc::free(list.cast()) };
        entries
    }

    /// Ends the transaction once; later calls do nothing.
    pub fn end(&mut self, status: c_int) {
        if !self.raw.is_null() {
            unsafe { (self.pam.end)(self.raw, status) };
            self.raw = std::ptr::null_mut();
        }
    }
}

impl Drop for Handle<'_> {
    fn drop(&mut self) {
        self.end(PAM_SUCCESS);
    }
}

unsafe fn flagged(library: *mut c_void, name: &CStr) -> Result<Flagged, String> {
    Ok(unsafe { std::mem::transmute::<*mut c_void, Flagged>(symbol(library, name)?) })
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
