//! The user's graphical logind session, found without `XDG_SESSION_ID`.
//!
//! The shell runs as a user service and Niri's environment has no session id,
//! so neither `GetSessionByPID` nor Niri's own `LockedHint` can be relied on.
//! `Manager.GetUser(uid).Display` names the session, and the locker sets
//! `LockedHint` on it itself.
//!
//! Test builds (`fake-pam-stack`, `record-session-actions`) never reach the
//! real system bus: they connect to `DOJO_TEST_SYSTEM_BUS` or fail.

use std::time::Duration;

use anyhow::Context as _;
use zbus::zvariant::OwnedObjectPath;

const LOGIND: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
/// How long a caller waits for logind before carrying on without the hint.
pub const HINT_TIMEOUT: Duration = Duration::from_millis(500);

#[cfg(not(any(feature = "fake-pam-stack", feature = "record-session-actions")))]
pub async fn connect() -> anyhow::Result<zbus::Connection> {
    Ok(zbus::Connection::system().await?)
}

#[cfg(any(feature = "fake-pam-stack", feature = "record-session-actions"))]
pub async fn connect() -> anyhow::Result<zbus::Connection> {
    let address = std::env::var("DOJO_TEST_SYSTEM_BUS")
        .context("a test build does not use the real system bus; set DOJO_TEST_SYSTEM_BUS")?;
    Ok(zbus::connection::Builder::address(address.as_str())?
        .build()
        .await?)
}

/// The session `Manager.GetUser(uid).Display` names, as a proxy.
pub async fn display_session(
    connection: &zbus::Connection,
) -> anyhow::Result<zbus::Proxy<'static>> {
    // SAFETY: getuid has no failure mode and touches no memory.
    let uid = unsafe { libc::getuid() };
    let manager = zbus::Proxy::new(
        connection,
        LOGIND,
        MANAGER_PATH,
        "org.freedesktop.login1.Manager",
    )
    .await?;
    let user: OwnedObjectPath = manager.call("GetUser", &(uid,)).await?;
    let user = zbus::Proxy::new(connection, LOGIND, user, "org.freedesktop.login1.User").await?;
    let (id, path): (String, OwnedObjectPath) = user.get_property("Display").await?;
    anyhow::ensure!(
        !id.is_empty() && path.as_str() != "/",
        "the user has no graphical logind session"
    );
    Ok(zbus::Proxy::new(connection, LOGIND, path, "org.freedesktop.login1.Session").await?)
}

/// Sets `LockedHint` on the display session over a connection of its own that
/// is gone again on return. Blocks; run it on a thread you can abandon.
pub fn set_locked_hint(locked: bool) -> anyhow::Result<()> {
    zbus::block_on(async {
        let connection = connect().await?;
        let session = display_session(&connection).await?;
        session
            .call::<_, _, ()>("SetLockedHint", &(locked,))
            .await
            .context("SetLockedHint")?;
        Ok(())
    })
}
