//! The machine around the display manager: logind for power and for the greeter's
//! session, and fprintd for whether a user has fingers enrolled. Each is a trait the
//! service holds, so no test touches the real machine, and one small implementation that
//! speaks D-Bus. Every call here is bounded by a timeout and none of them can fail a login.

use std::time::Duration;

use zbus::blocking::{Connection, Proxy, connection::Builder};
use zbus::zvariant::OwnedObjectPath;

use super::protocol::{Login, PowerOccasion};

/// The logind `Manager` method that performs the occasion. The names are the ones the
/// desktop's own system menu calls (`src/system_menu.rs`).
pub fn logind_method(occasion: PowerOccasion) -> &'static str {
    match occasion {
        PowerOccasion::Suspend => "Suspend",
        PowerOccasion::Hibernate => "Hibernate",
        PowerOccasion::Restart => "Reboot",
        PowerOccasion::Shutdown => "PowerOff",
    }
}

pub trait Power: Send + Sync {
    /// Blocks until logind has answered. The service calls it off its event loop.
    fn perform(&self, occasion: PowerOccasion) -> Result<(), String>;
}

/// What logind knows about sessions. `None` is always "logind could not be asked".
pub trait Seat: Send + Sync {
    /// Whether logind still knows a session of a user.
    fn has_session_of(&self, uid: u32) -> Option<bool>;
    /// Whether logind lists a session of anyone but `uid`.
    fn has_session_besides(&self, uid: u32) -> Option<bool>;
}

/// Why the login screen may not start `occasion` now, if it may not. Restarting, powering
/// off and hibernating end or park other people's sessions, so they are only for a machine
/// where the greeter is the one session; a compromised greeter must not be able to cut
/// short someone logged in elsewhere. Suspend leaves every session intact. `others` is
/// only asked when the answer matters, and no answer is a refusal.
pub fn power_refusal(
    occasion: PowerOccasion,
    others: impl FnOnce() -> Option<bool>,
) -> Option<&'static str> {
    if occasion == PowerOccasion::Suspend {
        return None;
    }
    match others() {
        Some(false) => None,
        Some(true) => Some("another session is open"),
        None => Some("logind could not say whether another session is open"),
    }
}

pub trait Fingerprints: Send {
    /// A reader exists and the user has an enrolled print. `false` on any error.
    fn enrolled(&mut self, login: &Login) -> bool;
}

const CALL_TIMEOUT: Duration = Duration::from_secs(2);

fn system_bus(timeout: Duration) -> zbus::Result<Connection> {
    Builder::system()?.method_timeout(timeout).build()
}

fn logind(connection: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(
        connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
}

pub struct Logind;

impl Power for Logind {
    fn perform(&self, occasion: PowerOccasion) -> Result<(), String> {
        let connection = system_bus(Duration::from_secs(15)).map_err(|e| e.to_string())?;
        logind(&connection)
            .and_then(|proxy| proxy.call::<_, _, ()>(logind_method(occasion), &(true,)))
            .map_err(|e| e.to_string())
    }
}

impl Logind {
    /// The owner of every session logind lists.
    fn session_owners() -> Option<Vec<u32>> {
        let connection = system_bus(CALL_TIMEOUT).ok()?;
        let sessions: Vec<(String, u32, String, String, OwnedObjectPath)> =
            logind(&connection).ok()?.call("ListSessions", &()).ok()?;
        Some(sessions.into_iter().map(|(_, owner, ..)| owner).collect())
    }
}

impl Seat for Logind {
    fn has_session_of(&self, uid: u32) -> Option<bool> {
        Some(Self::session_owners()?.contains(&uid))
    }

    fn has_session_besides(&self, uid: u32) -> Option<bool> {
        Some(Self::session_owners()?.iter().any(|owner| *owner != uid))
    }
}

pub struct Fprintd;

impl Fingerprints for Fprintd {
    fn enrolled(&mut self, login: &Login) -> bool {
        let ask = || -> zbus::Result<bool> {
            let connection = system_bus(Duration::from_millis(1500))?;
            let manager = Proxy::new(
                &connection,
                "net.reactivated.Fprint",
                "/net/reactivated/Fprint/Manager",
                "net.reactivated.Fprint.Manager",
            )?;
            let device: OwnedObjectPath = manager.call("GetDefaultDevice", &())?;
            let device = Proxy::new(
                &connection,
                "net.reactivated.Fprint",
                device.into_inner(),
                "net.reactivated.Fprint.Device",
            )?;
            let fingers: Vec<String> = device.call("ListEnrolledFingers", &(login.as_str(),))?;
            Ok(!fingers.is_empty())
        };
        ask().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_suspend_is_allowed_while_anyone_else_is_logged_in() {
        for occasion in [
            PowerOccasion::Hibernate,
            PowerOccasion::Restart,
            PowerOccasion::Shutdown,
        ] {
            assert_eq!(power_refusal(occasion, || Some(false)), None);
            assert!(power_refusal(occasion, || Some(true)).is_some());
            assert!(
                power_refusal(occasion, || None).is_some(),
                "an unanswered question is a no"
            );
        }
        let never = || -> Option<bool> { panic!("suspend does not ask") };
        assert_eq!(power_refusal(PowerOccasion::Suspend, never), None);
    }

    #[test]
    fn occasions_call_the_logind_methods_the_system_menu_calls() {
        let menu = include_str!("../system_menu.rs");
        for (occasion, method) in [
            (PowerOccasion::Suspend, "Suspend"),
            (PowerOccasion::Hibernate, "Hibernate"),
            (PowerOccasion::Restart, "Reboot"),
            (PowerOccasion::Shutdown, "PowerOff"),
        ] {
            assert_eq!(logind_method(occasion), method);
            assert!(
                menu.contains(&format!("Some(\"{method}\")")),
                "{method} is not a method src/system_menu.rs calls"
            );
        }
    }
}
