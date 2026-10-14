//! Single-use wallpaper chooser, independent of GPUI.
//!
//! Protocol references (read when implementing this boundary):
//! https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Request.html
//! https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.FileChooser.html
//! A Response subscription must exist before OpenFile: a portal may emit its
//! signal before replying to the method. Older portals returning a different
//! handle are deliberately rejected rather than risking a lost Response.

use std::collections::HashMap;
use std::io::Read;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use futures_lite::StreamExt;
use tokio::sync::oneshot;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MessageStream, Proxy};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const DESKTOP: &str = "/org/freedesktop/portal/desktop";
const REQUEST: &str = "org.freedesktop.portal.Request";
type Results = HashMap<String, OwnedValue>;
type Outcome = anyhow::Result<Option<PathBuf>>;

/// Keep this guard alive while awaiting the receiver. Dropping it explicitly
/// cancels the dialog. Drop never waits for D-Bus or joins the worker thread.
/// The receiver yields exactly one result: Some(path), None (cancelled), or Err.
pub struct Request {
    cancel: Option<oneshot::Sender<()>>,
    // Retained for lifecycle assertions in the private-bus tests; never joined
    // on the UI thread. Every worker operation has a finite deadline.
    _worker: std::thread::JoinHandle<()>,
}

impl Request {
    #[cfg(test)]
    pub(super) fn pending_for_test() -> (Self, oneshot::Receiver<()>) {
        let (cancel, cancelled) = oneshot::channel();
        (
            Self {
                cancel: Some(cancel),
                _worker: std::thread::spawn(|| {}),
            },
            cancelled,
        )
    }

    pub fn start() -> anyhow::Result<(Request, async_channel::Receiver<Outcome>)> {
        Self::start_with(Settings::default())
    }

    fn start_with(settings: Settings) -> anyhow::Result<(Self, async_channel::Receiver<Outcome>)> {
        let mut random = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let token = format!("dojo_wallpaper_{:032x}", u128::from_ne_bytes(random));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("create portal runtime")?;
        let (cancel, cancelled) = oneshot::channel();
        let (sender, receiver) = async_channel::bounded(1);
        let worker = std::thread::Builder::new()
            .name("dojo-wallpaper-portal".into())
            .spawn(move || {
                let result = runtime.block_on(run(settings, token, cancelled));
                let _ = sender.try_send(result);
                runtime.shutdown_timeout(Duration::from_secs(1));
            })
            .context("start portal worker")?;
        Ok((
            Self {
                cancel: Some(cancel),
                _worker: worker,
            },
            receiver,
        ))
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }
}

struct Settings {
    address: Option<String>,
    method: Duration,
    response: Duration,
    close: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            address: None,
            method: Duration::from_secs(10),
            response: Duration::from_secs(300),
            close: Duration::from_secs(2),
        }
    }
}

async fn run(settings: Settings, token: String, mut cancel: oneshot::Receiver<()>) -> Outcome {
    // Bound connection establishment AND AddMatch/proxy setup. Cancellation
    // during setup cannot leave a dialog: OpenFile has not yet been sent.
    let setup = async {
        let connection = match &settings.address {
            Some(address) => {
                zbus::connection::Builder::address(address.as_str())?
                    .build()
                    .await?
            }
            None => Connection::session().await?,
        };
        let sender = connection
            .unique_name()
            .context("session bus has no unique name")?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = OwnedObjectPath::try_from(format!("{DESKTOP}/request/{sender}/{token}"))?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(DESTINATION)?
            .interface(REQUEST)?
            .member("Response")?
            .path(path.clone())?
            .build();
        let responses = MessageStream::for_match_rule(rule, &connection, Some(4)).await?;
        Ok::<_, anyhow::Error>((connection, path, responses))
    };
    let (connection, path, mut responses) = tokio::select! {
        biased;
        _ = &mut cancel => return Ok(None),
        result = tokio::time::timeout(settings.method, setup) =>
            result.context("portal setup timed out")??,
    };
    let options: HashMap<&str, Value<'_>> = HashMap::from([
        ("handle_token", Value::from(token.as_str())),
        ("multiple", Value::from(false)),
        ("directory", Value::from(false)),
        (
            "filters",
            Value::from(vec![(
                "Images",
                vec![
                    (1u32, "image/jpeg"),
                    (1u32, "image/png"),
                    (1u32, "image/webp"),
                ],
            )]),
        ),
    ]);
    let open = async {
        // No introspection round trip is needed for this known interface.
        let proxy = Proxy::new(
            &connection,
            DESTINATION,
            DESKTOP,
            "org.freedesktop.portal.FileChooser",
        )
        .await?;
        proxy
            .call::<_, _, OwnedObjectPath>("OpenFile", &("", "Choose wallpaper", options))
            .await
    };
    let open = tokio::time::timeout(settings.method, open);
    tokio::pin!(open);
    let returned = tokio::select! {
        biased;
        _ = &mut cancel => {
            // Close immediately, even if OpenFile has not replied. Keep its
            // future alive for the remaining method deadline: a late-created
            // request needs a second Close, and may have an unexpected handle.
            close(&connection, &path, settings.close).await;
            if let Ok(Ok(actual)) = open.await {
                close(&connection, &actual, settings.close).await;
            }
            return Ok(None);
        }
        result = &mut open => result,
    };
    let actual = match returned {
        Ok(Ok(path)) => path,
        error => {
            close(&connection, &path, settings.close).await;
            return match error {
                Err(e) => Err(e).context("portal OpenFile timed out"),
                Ok(Err(e)) => Err(e).context("portal OpenFile failed"),
                Ok(Ok(_)) => unreachable!(),
            };
        }
    };
    if actual != path {
        close(&connection, &actual, settings.close).await;
        close(&connection, &path, settings.close).await;
        bail!("portal returned unexpected request handle: {actual}");
    }
    let response = tokio::select! {
        biased;
        _ = &mut cancel => {
            close(&connection, &path, settings.close).await;
            return Ok(None);
        }
        result = tokio::time::timeout(settings.response, responses.next()) => result,
    };
    let message = match response {
        Ok(Some(Ok(message))) => message,
        other => {
            close(&connection, &path, settings.close).await;
            return match other {
                Err(e) => Err(e).context("portal Response timed out"),
                Ok(Some(Err(e))) => Err(e).context("portal Response failed"),
                _ => Err(anyhow::anyhow!("portal Response stream ended")),
            };
        }
    };
    // Receipt of Response ends the request lifecycle; do not Close completed
    // requests, even when the response body or URI is malformed.
    let (code, mut results): (u32, Results) = message
        .body()
        .deserialize()
        .context("invalid portal Response")?;
    match code {
        0 => {
            let uris = results
                .remove("uris")
                .context("portal response has no uris")?;
            let uris = Vec::<String>::try_from(uris).context("portal uris must be strings")?;
            ensure!(uris.len() == 1, "portal must return exactly one file");
            Ok(Some(local_file_uri(&uris[0])?))
        }
        1 => Ok(None),
        2 => bail!("portal file selection failed"),
        _ => bail!("unknown portal response code {code}"),
    }
}

async fn close(connection: &Connection, path: &OwnedObjectPath, deadline: Duration) {
    // Cleanup is best-effort: a completed/never-created request may not exist.
    let _ = tokio::time::timeout(deadline, async {
        let proxy = Proxy::new(connection, DESTINATION, path.clone(), REQUEST).await?;
        proxy.call::<_, _, ()>("Close", &()).await
    })
    .await;
}

/// Decode bytes, not UTF-8: Unix filenames can contain arbitrary non-NUL bytes.
/// Accept an empty authority or literal localhost only. No URL normalization,
/// remote authorities, credentials, ports, query, fragment, or implicit paths.
fn local_file_uri(uri: &str) -> anyhow::Result<PathBuf> {
    let rest = uri
        .strip_prefix("file://")
        .context("expected local file:// URI")?;
    let path = if rest.starts_with('/') {
        rest
    } else {
        ensure!(
            rest.starts_with("localhost/"),
            "file URI has a non-local authority"
        );
        &rest["localhost".len()..]
    };
    ensure!(!path.contains(['?', '#']), "file URI has query or fragment");
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'%' {
            ensure!(i + 2 < bytes.len(), "truncated URI escape");
            let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            let high = hex(bytes[i + 1]).context("invalid URI escape")?;
            let low = hex(bytes[i + 2]).context("invalid URI escape")?;
            decoded.push(high * 16 + low);
            i += 3;
        } else {
            // RFC 3986 path characters: unreserved, sub-delims, ':', '@', '/'.
            ensure!(
                byte.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&byte),
                "invalid unescaped URI character"
            );
            decoded.push(byte);
            i += 1;
        }
    }
    ensure!(!decoded.contains(&0), "file URI contains NUL");
    ensure!(decoded.starts_with(b"/"), "file URI path must be absolute");
    Ok(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
}

#[cfg(test)]
#[path = "portal_tests.rs"]
mod tests;
