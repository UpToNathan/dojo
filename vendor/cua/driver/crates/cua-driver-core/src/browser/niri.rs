//! Niri identity adapter with exact Linux root-process endpoint ownership.
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use url::{Host, Url};

use super::niri_evidence as evidence;
use super::platform::{BrowserPlatform, PrepareOutcome, PrepareRequest};
use super::read::ExactWindowSource;
use super::refusal::{BrowserRefusal, BrowserRefusalCode};
use super::types::{
    BrowserClassification, BrowserEngineFamily, BrowserProduct, EndpointOwnershipMethod,
    EndpointOwnershipProof, EndpointTransport, NativeOwnershipMethod, NativeOwnershipProof,
    NativeWindowInfo, OwnedEndpoint, ProcessFingerprint, Rect,
};
use crate::embedded::ExactWindow;

fn refuse(message: impl Into<String>) -> BrowserRefusal {
    BrowserRefusal::new(BrowserRefusalCode::BrowserRouteUnavailable, message)
}

/// Constructed from trusted host configuration. Tools cannot supply endpoints.
pub struct BrowserConfig {
    endpoint: Option<(String, u16)>,
}
impl BrowserConfig {
    pub fn new(endpoint: Option<String>) -> Result<Self, BrowserRefusal> {
        let endpoint = endpoint.map(|value| {
            let parsed = Url::parse(&value).map_err(|_| refuse("configured browser endpoint URL invalid"))?;
            if parsed.scheme() != "ws" || !parsed.username().is_empty() || parsed.password().is_some()
                || parsed.fragment().is_some() || !matches!(parsed.host(), Some(Host::Ipv4(ip)) if ip.is_loopback())
                    && !matches!(parsed.host(), Some(Host::Ipv6(ip)) if ip.is_loopback()) {
                return Err(refuse("configured browser endpoint requires literal loopback ws URL without credentials"));
            }
            let port = parsed.port().ok_or_else(|| refuse("browser endpoint port must be explicit"))?;
            if port == 0 { return Err(refuse("browser endpoint port cannot be zero")); }
            Ok((value, port))
        }).transpose()?;
        Ok(Self { endpoint })
    }
}

/// Current compositor evidence, supplied by Dojo's trusted Niri adapter.
/// Bounds use CDP's device-independent global coordinate convention.
pub struct NiriWindowDescriptor {
    pub identity: ExactWindow,
    pub compositor_pid: u32,
    pub compositor_process_start: u64,
    pub title: String,
    pub bounds: Rect,
    pub geometry_exact: bool,
    pub windows_per_pid: Option<usize>,
}

#[async_trait]
pub trait NiriWindowSource: Send + Sync {
    async fn describe(&self, window_id: u64) -> Result<NiriWindowDescriptor, BrowserRefusal>;
    async fn owned_browser_process(&self,_window_id:u64)->Result<Option<(i64,u64)>,BrowserRefusal> {Ok(None)}
    async fn owned_debugging_arguments(&self,_pid:i64,_port:u16)->Result<bool,BrowserRefusal> {Ok(false)}
}

pub struct NiriBrowserPlatform {
    config: BrowserConfig,
    windows: Arc<dyn NiriWindowSource>,
}
impl NiriBrowserPlatform {
    pub fn new(config: BrowserConfig, windows: Arc<dyn NiriWindowSource>) -> Self {
        Self { config, windows }
    }

    async fn checked_window(&self, window_id: u64) -> Result<NiriWindowDescriptor, BrowserRefusal> {
        let window = self.windows.describe(window_id).await?;
        if window.identity.window_id != window_id
            || window.identity.pid <= 0
            || linux_process_start(window.compositor_pid)? != window.compositor_process_start
            || linux_process_start(window.identity.pid as u32)? != window.identity.process_start
        {
            return Err(refuse("Niri compositor/window/process generation changed"));
        }
        if ![
            window.bounds.x,
            window.bounds.y,
            window.bounds.width,
            window.bounds.height,
        ]
        .into_iter()
        .all(f64::is_finite)
            || window.bounds.width <= 0.0
            || window.bounds.height <= 0.0
        {
            return Err(refuse("Niri browser bounds unavailable"));
        }
        Ok(window)
    }
}

pub fn linux_process_start(pid: u32) -> Result<u64, BrowserRefusal> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| refuse("Linux process generation unavailable"))?;
    parse_start_time(&stat)
}
fn parse_start_time(stat: &str) -> Result<u64, BrowserRefusal> {
    let (_, fields) = stat
        .rsplit_once(')')
        .ok_or_else(|| refuse("invalid Linux process identity"))?;
    fields
        .split_whitespace()
        .nth(19)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| refuse("Linux process start time unavailable"))
}

fn argv_debug_port(bytes: &[u8], expected: u16) -> bool {
    let args = bytes
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .collect::<Vec<_>>();
    let mut ports = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        let value = if let Some(port) = arg.strip_prefix(b"--remote-debugging-port=") {
            Some(port)
        } else if *arg == b"--remote-debugging-port" {
            args.get(index + 1).copied()
        } else {
            None
        };
        if let Some(value) = value {
            let Ok(value) = std::str::from_utf8(value) else {
                return false;
            };
            let Ok(port) = value.parse::<u16>() else {
                return false;
            };
            ports.push(port);
        }
    }
    matches!(ports.as_slice(), [port] if *port == 0 || *port == expected)
}

fn root_socket_inodes(pid: i64) -> Result<HashSet<u64>, BrowserRefusal> {
    let entries = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .map_err(|_| refuse("browser root file descriptors unavailable"))?;
    Ok(entries
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter_map(|path| {
            path.to_string_lossy()
                .strip_prefix("socket:[")
                .and_then(|value| value.strip_suffix(']'))
                .and_then(|value| value.parse().ok())
        })
        .collect())
}
fn root_owns_listener(pid: i64, port: u16) -> Result<bool, BrowserRefusal> {
    let owned = root_socket_inodes(pid)?;
    for kind in ["tcp", "tcp6"] {
        let text = std::fs::read_to_string(format!("/proc/{pid}/net/{kind}"))
            .map_err(|_| refuse("browser network namespace listener table unavailable"))?;
        if evidence::parse_proc_net_loopback_listeners(&text)
            .into_iter()
            .any(|(found_port, inode)| found_port == port && owned.contains(&inode))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[async_trait]
impl ExactWindowSource for NiriBrowserPlatform {
    async fn current(&self, window_id: u64) -> Result<ExactWindow, BrowserRefusal> {
        Ok(self.checked_window(window_id).await?.identity)
    }
}

#[async_trait]
impl BrowserPlatform for NiriBrowserPlatform {
    async fn classify_browser(&self, pid: i64) -> Result<BrowserClassification, BrowserRefusal> {
        let fingerprint = self.process_fingerprint(pid).await?;
        let executable = fingerprint
            .executable
            .as_ref()
            .ok_or_else(|| refuse("browser executable unavailable"))?;
        if !evidence::trusted_root_owned_installation(Path::new(executable)) {
            return Err(refuse("browser executable installation is not trusted"));
        }
        let product = evidence::browser_product(executable);
        let chromium = evidence::is_chromium_identity(executable);
        let gecko = product == BrowserProduct::Firefox;
        Ok(BrowserClassification {
            is_browser: chromium || gecko,
            engine: if chromium {
                BrowserEngineFamily::Chromium
            } else if gecko {
                BrowserEngineFamily::Gecko
            } else {
                BrowserEngineFamily::Unknown
            },
            product_kind: product,
            product: Path::new(executable)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            channel: None,
            process_role: evidence::process_role_for_pid(pid, product),
            supports_cdp: chromium,
        })
    }
    async fn native_window(
        &self,
        pid: i64,
        window_id: u64,
    ) -> Result<NativeWindowInfo, BrowserRefusal> {
        let window = self.checked_window(window_id).await?;
        if i64::from(window.identity.pid) != pid && self.windows.owned_browser_process(window_id).await?.map(|identity|identity.0)!=Some(pid) {
            return Err(refuse("Niri browser window owner mismatch"));
        }
        Ok(NativeWindowInfo {
            pid,
            window_id,
            title: window.title,
            bounds: window.bounds,
            geometry_exact: window.geometry_exact,
            ownership: NativeOwnershipProof {
                method: NativeOwnershipMethod::PlatformAttested,
                owner_pid: pid,
                detail: Some(
                    "exact Niri compositor/client/window and Linux process generation".into(),
                ),
            },
        })
    }
    async fn is_only_exact_native_window(
        &self,
        pid: i64,
        window_id: u64,
    ) -> Result<Option<bool>, BrowserRefusal> {
        let window = self.checked_window(window_id).await?;
        if i64::from(window.identity.pid) != pid && self.windows.owned_browser_process(window_id).await?.map(|identity|identity.0)!=Some(pid) {
            return Err(refuse("Niri browser window owner mismatch"));
        }
        Ok(window.windows_per_pid.map(|count| count == 1))
    }
    async fn discover_owned_endpoint(
        &self,
        pid: i64,
    ) -> Result<Option<OwnedEndpoint>, BrowserRefusal> {
        let Some((endpoint, port)) = &self.config.endpoint else {
            return Ok(None);
        };
        let before = self.process_fingerprint(pid).await?;
        let argv = std::fs::read(format!("/proc/{pid}/cmdline"))
            .map_err(|_| refuse("browser command line unavailable"))?;
        let arguments_bound=argv_debug_port(&argv,*port) || self.windows.owned_debugging_arguments(pid,*port).await?;
        let listener_bound=root_owns_listener(pid,*port)?;
        if !arguments_bound || !listener_bound {
            return Err(refuse(format!("configured endpoint lacks exact root-PID listener and debugging-argument proof (pid={pid}, arguments={arguments_bound}, listener={listener_bound})")));
        }
        if self.process_fingerprint(pid).await? != before {
            return Err(refuse(
                "browser process changed during endpoint attestation",
            ));
        }
        Ok(Some(OwnedEndpoint {
            ws_url: endpoint.clone(),
            http_port: Some(*port),
            transport: EndpointTransport::LegacyJsonVersion,
            ownership: EndpointOwnershipProof {
                method: EndpointOwnershipMethod::ListeningSocketPid,
                owner_pid: pid,
                listener_pid: Some(pid),
                detail: Some(
                    "exact /proc root socket inode and matching remote-debugging-port".into(),
                ),
            },
        }))
    }
    async fn process_fingerprint(&self, pid: i64) -> Result<ProcessFingerprint, BrowserRefusal> {
        let pid_u32 = u32::try_from(pid)
            .ok()
            .filter(|pid| *pid > 0)
            .ok_or_else(|| refuse("invalid browser PID"))?;
        let start = linux_process_start(pid_u32)?;
        let executable = std::fs::read_link(format!("/proc/{pid}/exe"))
            .map_err(|_| refuse("browser executable unavailable"))?;
        if linux_process_start(pid_u32)? != start {
            return Err(refuse("process changed during executable attestation"));
        }
        Ok(ProcessFingerprint {
            pid,
            start_time: Some(start),
            executable: Some(executable.to_string_lossy().into_owned()),
        })
    }
    async fn prepare_endpoint(
        &self,
        _request: PrepareRequest,
    ) -> Result<PrepareOutcome, BrowserRefusal> {
        Err(refuse(
            "browser preparation and mutation admission are unavailable",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_debugging_argument_is_exact_and_unambiguous() {
        assert!(argv_debug_port(
            b"chrome\0--remote-debugging-port=9222\0",
            9222
        ));
        assert!(argv_debug_port(
            b"chrome\0--remote-debugging-port\09222\0",
            9222
        ));
        assert!(argv_debug_port(
            b"chrome\0--remote-debugging-port=0\0",
            9222
        ));
        assert!(!argv_debug_port(
            b"chrome\0--remote-debugging-port=9333\0",
            9222
        ));
        assert!(!argv_debug_port(
            b"chrome\0--remote-debugging-port=9222\0--remote-debugging-port=0\0",
            9222
        ));
        assert!(!argv_debug_port(b"chrome\0--remote-debugging-pipe\0", 9222));
    }
    #[test]
    fn configured_endpoints_require_literal_loopback_and_explicit_port() {
        assert!(
            BrowserConfig::new(Some("ws://127.0.0.1:9222/devtools/browser/owned".into())).is_ok()
        );
        assert!(BrowserConfig::new(Some("ws://[::1]:9222/devtools/browser/owned".into())).is_ok());
        for endpoint in [
            "ws://localhost:9222/",
            "ws://example.com:9222/",
            "wss://127.0.0.1:9222/",
            "ws://user@127.0.0.1:9222/",
            "ws://127.0.0.1/",
            "ws://127.0.0.1:0/",
        ] {
            assert!(
                BrowserConfig::new(Some(endpoint.into())).is_err(),
                "{endpoint}"
            );
        }
    }
    #[test]
    fn process_start_parser_handles_parentheses_and_space_in_name() {
        let fields = (0..20)
            .map(|index| if index == 19 { "4242" } else { "0" })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            parse_start_time(&format!("100 (name (with) spaces) {fields}")).unwrap(),
            4242
        );
        assert!(parse_start_time("100 (name) S").is_err());
    }
    #[test]
    fn real_socket_inode_is_owned_only_by_its_process() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let pid = i64::from(std::process::id());
        assert!(root_owns_listener(pid, listener.local_addr().unwrap().port()).unwrap());
        assert!(!root_owns_listener(1, listener.local_addr().unwrap().port()).unwrap_or(false));
    }
    #[test]
    fn retained_cua_listener_parser_excludes_nonlisten_and_nonloopback() {
        let header = "sl local_address rem_address st tx_queue rx_queue tr tm->when retrnsmt uid timeout inode\n";
        let rows = "0: 0100007F:2406 00000000:0000 0A 0000 00:00 0000 1000 0 123\n1: 00000000:2406 00000000:0000 0A 0000 00:00 0000 1000 0 124\n2: 0100007F:2406 00000000:0000 01 0000 00:00 0000 1000 0 125\n";
        assert_eq!(
            evidence::parse_proc_net_loopback_listeners(&(header.to_owned() + rows)),
            vec![(9222, 123)]
        );
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use std::sync::Mutex;
    use uuid::Uuid;
    struct Windows {
        start: Mutex<u64>,
    }
    #[async_trait]
    impl NiriWindowSource for Windows {
        async fn describe(&self, window_id: u64) -> Result<NiriWindowDescriptor, BrowserRefusal> {
            let pid = std::process::id();
            Ok(NiriWindowDescriptor {
                identity: ExactWindow {
                    compositor_epoch: Uuid::nil(),
                    window_id,
                    client_generation: 1,
                    pid: pid as i32,
                    process_start: *self.start.lock().unwrap(),
                },
                compositor_pid: pid,
                compositor_process_start: linux_process_start(pid)?,
                title: "fixture".into(),
                bounds: Rect::new(0.0, 0.0, 800.0, 600.0),
                geometry_exact: true,
                windows_per_pid: Some(2),
            })
        }
    }
    #[tokio::test]
    async fn adapter_checks_live_process_identity_and_exact_window_cardinality() {
        let windows = Arc::new(Windows {
            start: Mutex::new(linux_process_start(std::process::id()).unwrap()),
        });
        let adapter = NiriBrowserPlatform::new(BrowserConfig::new(None).unwrap(), windows.clone());
        let native = adapter
            .native_window(i64::from(std::process::id()), 7)
            .await
            .unwrap();
        assert_eq!(native.window_id, 7);
        assert!(native.geometry_exact);
        assert_eq!(
            adapter
                .is_only_exact_native_window(i64::from(std::process::id()), 7)
                .await
                .unwrap(),
            Some(false)
        );
        assert!(adapter.native_window(1, 7).await.is_err());
        *windows.start.lock().unwrap() += 1;
        assert!(adapter.current(7).await.is_err());
    }
    #[tokio::test]
    async fn configured_owned_listener_does_not_grant_debugging_or_classification() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let windows = Arc::new(Windows {
            start: Mutex::new(linux_process_start(std::process::id()).unwrap()),
        });
        let endpoint = format!(
            "ws://127.0.0.1:{}/devtools/browser/fixture",
            listener.local_addr().unwrap().port()
        );
        let adapter =
            NiriBrowserPlatform::new(BrowserConfig::new(Some(endpoint)).unwrap(), windows);
        assert!(adapter
            .discover_owned_endpoint(i64::from(std::process::id()))
            .await
            .is_err());
        assert!(adapter
            .classify_browser(i64::from(std::process::id()))
            .await
            .is_err());
    }
}
