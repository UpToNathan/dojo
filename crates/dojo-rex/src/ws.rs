//! WebSocket over the server's Unix socket.
use std::os::unix::net::UnixStream;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use tungstenite::WebSocket;
use tungstenite::client::IntoClientRequest as _;
use tungstenite::http::HeaderValue;

pub(crate) fn open(socket: &Path, path: &str, subprotocol: &str) -> Result<WebSocket<UnixStream>> {
    let stream = UnixStream::connect(socket)
        .with_context(|| format!("connecting to the Rex server at {}", socket.display()))?;
    let mut request = format!("ws://rex{path}").into_client_request()?;
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", HeaderValue::from_str(subprotocol)?);
    let (ws, response) = tungstenite::client::client(request, stream)
        .map_err(|error| anyhow::anyhow!("opening {path}: {error}"))?;
    let chosen = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok());
    ensure!(chosen == Some(subprotocol), "server chose subprotocol {chosen:?}, wanted {subprotocol}");
    Ok(ws)
}
