//! Dojo's **notices**: one `Notify` call on the session bus, best effort (see ADR-0012).
//!
//! A notice says that something happened, never that something is so: the sources that send them
//! report changes. Sending never fails an operation: a missing notification service is an error
//! the caller may log and ignore. [`post`] hands a notice to a worker thread and returns at once,
//! for the shell; [`send`] makes the call itself, for commands.

use std::collections::HashMap;
use std::fmt;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

use zbus::blocking::{Connection, connection};
use zbus::zvariant::Value;

pub mod level;

/// The name notices are sent under. The notification service lists and mutes apps by it.
pub const APP: &str = "Dojo";

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// The freedesktop urgency hint: low is a confirmation the person caused, normal a change they
/// would want to know about, critical what needs them now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Urgency {
    Low = 0,
    Normal = 1,
    Critical = 2,
}

impl std::str::FromStr for Urgency {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        match s {
            "low" => Ok(Urgency::Low),
            "normal" => Ok(Urgency::Normal),
            "critical" => Ok(Urgency::Critical),
            other => Err(Error(format!("urgency is low, normal or critical, not `{other}`"))),
        }
    }
}

/// One notification Dojo sends.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    /// What the notice reports (`volume`, `battery`). A new notice replaces the earlier one of the
    /// same kind that is still showing.
    pub kind: String,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    /// A theme icon name, or empty.
    pub icon: String,
    /// A progress value out of 100, drawn as a bar.
    pub value: Option<u8>,
    /// Milliseconds on screen: 0 never expires, -1 is the service's default.
    pub timeout_ms: i32,
}

impl Notice {
    pub fn new(kind: impl Into<String>, summary: impl Into<String>) -> Notice {
        Notice {
            kind: kind.into(),
            summary: summary.into(),
            body: String::new(),
            urgency: Urgency::Normal,
            icon: String::new(),
            value: None,
            timeout_ms: -1,
        }
    }

    pub fn body(mut self, body: impl Into<String>) -> Notice {
        self.body = body.into();
        self
    }

    pub fn urgency(mut self, urgency: Urgency) -> Notice {
        self.urgency = urgency;
        self
    }

    pub fn icon(mut self, icon: impl Into<String>) -> Notice {
        self.icon = icon.into();
        self
    }

    pub fn value(mut self, percent: u8) -> Notice {
        self.value = Some(percent.min(100));
        self
    }

    pub fn timeout_ms(mut self, ms: i32) -> Notice {
        self.timeout_ms = ms;
        self
    }

    /// The `Notify` hints: urgency, the replace key, and the progress value.
    fn hints(&self) -> HashMap<&'static str, Value<'_>> {
        let mut hints: HashMap<&'static str, Value<'_>> = HashMap::new();
        hints.insert("urgency", Value::U8(self.urgency as u8));
        if !self.kind.is_empty() {
            hints.insert("x-canonical-private-synchronous", Value::Str(self.kind.as_str().into()));
        }
        if let Some(value) = self.value {
            hints.insert("value", Value::I32(i32::from(value)));
        }
        hints
    }
}

#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Error {
        Error(e.to_string())
    }
}

/// The session bus: `DBUS_SESSION_BUS_ADDRESS`, else the user's runtime directory, which an ssh
/// session (where a peer's notices arrive) may not have exported.
fn session_address() -> String {
    if let Some(address) = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok().filter(|a| !a.is_empty()) {
        return address;
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| d.starts_with('/')).unwrap_or_else(|| {
        use std::os::unix::fs::MetadataExt;
        let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
        format!("/run/user/{uid}")
    });
    format!("unix:path={runtime}/bus")
}

fn connect(address: &str) -> Result<Connection, Error> {
    Ok(connection::Builder::address(address)?.build()?)
}

fn shared() -> &'static Mutex<Option<Connection>> {
    static CONNECTION: OnceLock<Mutex<Option<Connection>>> = OnceLock::new();
    CONNECTION.get_or_init(|| Mutex::new(None))
}

fn call(conn: &Connection, notice: &Notice) -> Result<u32, Error> {
    let reply = conn.call_method(
        Some(SERVICE),
        PATH,
        Some(SERVICE),
        "Notify",
        &(APP, 0u32, notice.icon.as_str(), notice.summary.as_str(), notice.body.as_str(), Vec::<&str>::new(), notice.hints(), notice.timeout_ms),
    )?;
    Ok(reply.body().deserialize::<u32>()?)
}

/// Sends `notice` on the session bus and returns its id. Reuses one connection, and connects
/// again once if the bus went away.
pub fn send(notice: &Notice) -> Result<u32, Error> {
    send_to(&session_address(), notice)
}

/// [`send`] to a bus at `address`.
pub fn send_to(address: &str, notice: &Notice) -> Result<u32, Error> {
    let mut slot = shared().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(conn) = slot.as_ref()
        && let Ok(id) = call(conn, notice)
    {
        return Ok(id);
    }
    let conn = connect(address)?;
    let result = call(&conn, notice);
    *slot = Some(conn);
    result
}

/// Hands `notice` to a worker thread and returns at once, so a slow or absent notification
/// service never holds the caller up. Notices go out in the order they were posted.
pub fn post(notice: Notice) {
    static WORKER: OnceLock<Sender<Notice>> = OnceLock::new();
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = channel::<Notice>();
        let _ = std::thread::Builder::new().name("dojo-notify".into()).spawn(move || {
            while let Ok(notice) = rx.recv() {
                let _ = send(&notice);
            }
        });
        tx
    });
    let _ = tx.send(notice);
}

#[cfg(test)]
mod tests;
