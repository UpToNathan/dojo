//! Optional external mutation transport. The operator enables this experimental
//! route; accepted effects require durable recovery and have no settlement hook.
use anyhow::{Context, Result, ensure};
use dojo_cua_browser::browser_observation::BoundNavigationEndpoint;
use dojo_external_admission::broker::{AdmissionReceipt, NavigateIntent};
use std::{net::SocketAddrV4, path::Path};

/// Internal authority data. Tool JSON cannot construct this record because it is
/// neither Deserialize nor public outside the Rust agent module. The service must
/// mint it after consuming capture authority in the operator-enabled route.
/// This reservation is not an authenticated durable consent record.
pub(super) struct NavigationGrant {
    pub(super) peer: u64,
    pub(super) revision: u64,
    pub(super) intent_id: String,
    pub(super) grant_id: String,
    pub(super) capture_geometry: niri_ipc::CapturedInputGeometry,
}
/// Consumes the fresh binding and grant. An admitted navigation is always unknown
/// until a new independently correlated observation/recovery decision confirms it.
/// The experimental navigation tool invokes this helper without a fallback route.
pub(super) fn dispatch(
    socket_path: &Path,
    binding: BoundNavigationEndpoint,
    grant: NavigationGrant,
    url: String,
    request_id: u64,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<AdmissionReceipt> {
    ensure!(
        std::env::var_os("DOJO_EXTERNAL_ADMISSION_SOCKET").as_deref()
            == Some(socket_path.as_os_str()),
        "external mutation route is not explicitly enabled"
    );
    let endpoint = loopback_endpoint(&binding.websocket_url)?;
    ensure!(
        !grant.intent_id.is_empty() && !grant.grant_id.is_empty(),
        "internal intent and capture grant required"
    );
    let intent = NavigateIntent {
        version: 1,
        peer: grant.peer,
        revision: grant.revision,
        window_id: binding.window.window_id,
        browser_pid: binding
            .window
            .pid
            .try_into()
            .context("invalid browser process identity")?,
        browser_start_time: binding.window.process_start,
        page_target: binding.page_target,
        request_id,
        url,
        intent_id: grant.intent_id,
        grant_id: grant.grant_id,
        lifetime_ms: 1000,
        capture_geometry: serde_json::to_value(grant.capture_geometry)?,
    };
    dojo_external_admission::broker::navigate_cancellable(
        socket_path,
        endpoint,
        intent,
        Some(cancelled),
    )
}
fn loopback_endpoint(websocket_url: &str) -> Result<SocketAddrV4> {
    let authority = websocket_url
        .strip_prefix("ws://")
        .context("cleartext websocket endpoint required")?
        .split_once('/')
        .context("websocket path required")?
        .0;
    let endpoint: SocketAddrV4 = authority
        .parse()
        .context("numeric IPv4 endpoint required")?;
    ensure!(endpoint.ip().is_loopback(), "loopback endpoint required");
    Ok(endpoint)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_parser_rejects_tls_credentials_dns_and_non_loopback() {
        assert_eq!(
            loopback_endpoint("ws://127.0.0.1:9222/devtools/browser/id").unwrap(),
            "127.0.0.1:9222".parse::<SocketAddrV4>().unwrap()
        );
        for url in [
            "wss://127.0.0.1:9222/a",
            "ws://user@127.0.0.1:9222/a",
            "ws://localhost:9222/a",
            "ws://192.0.2.1:9222/a",
            "ws://127.0.0.1:9222",
        ] {
            assert!(loopback_endpoint(url).is_err());
        }
    }
}
