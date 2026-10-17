//! A stand-in for logind's session objects on a private bus, for the nested
//! lock handoff run. It answers `Manager.GetUser`, `User.Display` and the
//! `LockedHint` property with `SetLockedHint`, and appends each hint it is
//! given to the file named by `FAKE_LOGIND_LOG`.
//!
//! `DOJO_TEST_SYSTEM_BUS` names the private bus. The real system bus is never used.

use std::io::Write as _;

use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;
use zbus::{connection::Builder, interface};

const SESSION: &str = "/org/freedesktop/login1/session/c9";
const USER: &str = "/org/freedesktop/login1/user/_1000";

struct Manager;

#[interface(name = "org.freedesktop.login1.Manager")]
impl Manager {
    fn get_user(&self, _uid: u32) -> OwnedObjectPath {
        OwnedObjectPath::try_from(USER).expect("a valid path")
    }
}

struct User;

#[interface(name = "org.freedesktop.login1.User")]
impl User {
    #[zbus(property)]
    fn display(&self) -> (String, OwnedObjectPath) {
        (
            "c9".into(),
            OwnedObjectPath::try_from(SESSION).expect("a valid path"),
        )
    }
}

struct Session {
    locked: bool,
}

#[interface(name = "org.freedesktop.login1.Session")]
impl Session {
    #[zbus(property)]
    fn locked_hint(&self) -> bool {
        self.locked
    }

    async fn set_locked_hint(
        &mut self,
        locked: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.locked = locked;
        if let Some(path) = std::env::var_os("FAKE_LOGIND_LOG")
            && let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path)
        {
            let _ = writeln!(file, "LockedHint={locked}");
        }
        self.locked_hint_changed(&emitter).await?;
        Ok(())
    }
}

fn main() -> zbus::Result<()> {
    let address = std::env::var("DOJO_TEST_SYSTEM_BUS").expect("DOJO_TEST_SYSTEM_BUS");
    zbus::block_on(async {
        let _connection = Builder::address(address.as_str())?
            .name("org.freedesktop.login1")?
            .serve_at("/org/freedesktop/login1", Manager)?
            .serve_at(USER, User)?
            .serve_at(SESSION, Session { locked: false })?
            .build()
            .await?;
        std::future::pending::<()>().await;
        Ok(())
    })
}
