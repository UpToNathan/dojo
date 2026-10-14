use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use dojo_cua_browser::browser_observation::host::{BrowserRefusal, BrowserRefusalCode, Rect};
use dojo_cua_browser::browser_observation::niri::{
    BrowserConfig, NiriBrowserPlatform, NiriWindowDescriptor, NiriWindowSource,
};
use dojo_cua_browser::browser_observation::{
    BrowserElementRef, BrowserReadEpisode, ExactWindow, ExactWindowSource,
};
use niri_ipc::{Request, Response};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn refusal(message: impl Into<String>) -> BrowserRefusal {
    BrowserRefusal::new(BrowserRefusalCode::BrowserRouteUnavailable, message)
}
fn refusal_error(reason: BrowserRefusal) -> anyhow::Error {
    anyhow::anyhow!(json!({"status":"refused","refusal":reason}).to_string())
}

#[derive(Clone, PartialEq)]
struct Frozen {
    identity: ExactWindow,
    compositor_pid: u32,
    compositor_start: u64,
    title: String,
    bounds: Rect,
    scene_bounds: Rect,
    windows_per_pid: usize,
}

impl Frozen {
    fn verification_window(&self, url: &Value) -> Value {
        json!({"pid":self.identity.pid,"window_id":self.identity.window_id,"title":self.title,"url":url,
            "bounds":{"x":self.scene_bounds.x,"y":self.scene_bounds.y,"width":self.scene_bounds.width,"height":self.scene_bounds.height}})
    }
}

struct Source {
    socket: Option<PathBuf>,
    frozen: Mutex<Frozen>,
    cancelled: Arc<AtomicBool>,
    owned: Option<Arc<dyn dojo_cua_browser::browser_observation::OwnedBrowserSource>>,
}

fn freeze(path: Option<&std::path::Path>, window_id: u64) -> Result<Frozen> {
    freeze_bounded(path, window_id, Duration::from_secs(3))
}
fn freeze_bounded(
    path: Option<&std::path::Path>,
    window_id: u64,
    timeout: Duration,
) -> Result<Frozen> {
    let deadline = std::time::Instant::now() + timeout;
    let mut socket = super::desktop_socket(path)?;
    let remaining = |socket: &mut niri_ipc::socket::Socket| -> Result<()> {
        let budget = deadline.saturating_duration_since(std::time::Instant::now());
        ensure!(!budget.is_zero(), "browser native proof deadline exceeded");
        socket.set_read_timeout(Some(budget))?;
        socket.set_write_timeout(Some(budget))?;
        Ok(())
    };
    remaining(&mut socket)?;
    let Response::Windows(windows) = socket.send(Request::Windows)?.map_err(anyhow::Error::msg)?
    else {
        anyhow::bail!("unexpected windows response");
    };
    let window = windows
        .iter()
        .find(|window| window.id == window_id)
        .context("browser window disappeared")?;
    let pid = window
        .pid
        .context("browser window has no exact process identity")?;
    remaining(&mut socket)?;
    let Response::Workspaces(workspaces) = socket
        .send(Request::Workspaces)?
        .map_err(anyhow::Error::msg)?
    else {
        anyhow::bail!("unexpected workspaces response");
    };
    let output = workspaces
        .iter()
        .find(|workspace| Some(workspace.id) == window.workspace_id && workspace.is_active)
        .and_then(|workspace| workspace.output.clone())
        .context("browser window is not on an active output")?;
    remaining(&mut socket)?;
    let Response::CapturedScene(scene) = socket
        .send(Request::CaptureScene { output })?
        .map_err(anyhow::Error::msg)?
    else {
        anyhow::bail!("browser observation requires capture-correlated geometry");
    };
    ensure!(
        socket.peer_process_id()? == scene.compositor_pid,
        "scene compositor peer mismatch"
    );
    let rendered = scene
        .windows
        .iter()
        .find(|candidate| candidate.id == window_id && candidate.pid == Some(pid))
        .context("browser window is not rendered")?;
    ensure!(
        !rendered.blocked_out,
        "browser window is excluded from screen capture"
    );
    let compositor_start = super::screenshot::process_start_time(scene.compositor_pid)?;
    let mut hash = Sha256::new();
    hash.update(std::fs::read("/proc/sys/kernel/random/boot_id")?);
    hash.update(scene.compositor_pid.to_le_bytes());
    hash.update(compositor_start.to_le_bytes());
    let epoch: [u8; 16] = hash.finalize()[..16].try_into()?;
    let (x, y, width, height) = rendered.rect;
    Ok(Frozen {
        identity: ExactWindow {
            compositor_epoch: Uuid::from_bytes(epoch),
            window_id,
            // Niri's mapped window ID is unique for this compositor lifetime.
            client_generation: window_id,
            pid,
            process_start: super::screenshot::process_start_time(pid.try_into()?)?,
        },
        compositor_pid: scene.compositor_pid,
        compositor_start,
        title: window.title.clone().unwrap_or_default(),
        bounds: Rect::new(
            f64::from(scene.logical_output.x) + x,
            f64::from(scene.logical_output.y) + y,
            width,
            height,
        ),
        scene_bounds: Rect::new(x, y, width, height),
        windows_per_pid: windows
            .iter()
            .filter(|candidate| candidate.pid == Some(pid))
            .count(),
    })
}

#[async_trait]
impl NiriWindowSource for Source {
    async fn owned_debugging_arguments(
        &self,
        pid: i64,
        port: u16,
    ) -> std::result::Result<bool, BrowserRefusal> {
        let window = self.frozen.lock().unwrap().identity.window_id;
        Ok(self
            .owned_browser_process(window)
            .await?
            .is_some_and(|identity| identity.0 == pid)
            && self
                .owned
                .as_ref()
                .is_some_and(|proof| proof.debugging_port() == port))
    }
    async fn owned_browser_process(
        &self,
        window_id: u64,
    ) -> std::result::Result<Option<(i64, u64)>, BrowserRefusal> {
        let current = self.current(window_id).await?;
        if let Some(proof) = &self.owned {
            proof.current_owned(current).await?;
            let (pid, start) = proof.browser_process();
            Ok(Some((i64::from(pid), start)))
        } else {
            Ok(None)
        }
    }
    async fn describe(
        &self,
        window_id: u64,
    ) -> std::result::Result<NiriWindowDescriptor, BrowserRefusal> {
        let frozen = self.frozen.lock().unwrap().clone();
        if frozen.identity.window_id != window_id || self.cancelled.load(Ordering::Acquire) {
            return Err(refusal("browser session cancelled or target mismatch"));
        }
        Ok(NiriWindowDescriptor {
            identity: frozen.identity,
            compositor_pid: frozen.compositor_pid,
            compositor_process_start: frozen.compositor_start,
            title: frozen.title,
            bounds: frozen.bounds,
            geometry_exact: true,
            windows_per_pid: Some(frozen.windows_per_pid),
        })
    }
}

#[async_trait]
impl ExactWindowSource for Source {
    async fn current(&self, window_id: u64) -> std::result::Result<ExactWindow, BrowserRefusal> {
        let frozen = self.frozen.lock().unwrap().clone();
        let path = self.socket.clone();
        let cancelled = self.cancelled.clone();
        let owned_process = self.owned.as_ref().map(|proof| proof.browser_process());
        tokio::task::spawn_blocking(move || -> Result<ExactWindow> {
            ensure!(!cancelled.load(Ordering::Acquire) && frozen.identity.window_id == window_id, "browser session cancelled or target mismatch");
            let mut socket = super::desktop_socket(path.as_deref())?;
            ensure!(socket.peer_process_id()? == frozen.compositor_pid && super::screenshot::process_start_time(frozen.compositor_pid)? == frozen.compositor_start, "browser compositor generation changed");
            let Response::Windows(windows) = socket.send(Request::Windows)?.map_err(anyhow::Error::msg)? else { anyhow::bail!("unexpected windows response"); };
            if let Some((root,start))=owned_process {ensure!(windows.iter().filter(|window|window.pid.and_then(|pid|u32::try_from(pid).ok()).is_some_and(|pid|super::browser_owned::descends(pid,root as u32,start))).count()==1,"owned browser has multiple native windows");}
            ensure!(windows.iter().any(|window| window.id == window_id && window.pid == Some(frozen.identity.pid)), "browser native window identity changed");
            ensure!(super::screenshot::process_start_time(frozen.identity.pid.try_into()?)? == frozen.identity.process_start, "browser process generation changed");
            Ok(frozen.identity)
        }).await.map_err(|error| refusal(error.to_string()))?.map_err(|error| refusal(format!("{error:#}")))
    }
}

pub(super) struct BrowserSession {
    runtime: tokio::runtime::Runtime,
    source: Arc<Source>,
    episode: BrowserReadEpisode,
    refs: HashMap<String, BrowserElementRef>,
    verification_session: String,
}
impl BrowserSession {
    pub(super) fn window_id(&self) -> u64 {
        self.source.frozen.lock().unwrap().identity.window_id
    }
    pub(super) fn new(
        socket: Option<PathBuf>,
        window: u64,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Self> {
        Self::configured(
            socket,
            window,
            cancelled,
            std::env::var("DOJO_BROWSER_CDP_ENDPOINT").ok(),
            None,
        )
    }
    pub(super) fn new_owned(
        socket: Option<PathBuf>,
        cancelled: Arc<AtomicBool>,
        owned: &super::browser_owned::OwnedBrowser,
    ) -> Result<Self> {
        Self::configured(
            socket,
            owned.window_id(),
            cancelled,
            Some(owned.endpoint()),
            Some(owned.proof()),
        )
    }
    fn configured(
        socket: Option<PathBuf>,
        window: u64,
        cancelled: Arc<AtomicBool>,
        endpoint: Option<String>,
        owned: Option<Arc<dyn dojo_cua_browser::browser_observation::OwnedBrowserSource>>,
    ) -> Result<Self> {
        let frozen = freeze(socket.as_deref(), window)?;
        let source = Arc::new(Source {
            socket,
            frozen: Mutex::new(frozen),
            cancelled,
            owned: owned.clone(),
        });
        let platform = Arc::new(NiriBrowserPlatform::new(
            BrowserConfig::new(endpoint).map_err(refusal_error)?,
            source.clone(),
        ));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let episode = runtime
            .block_on(async {
                match owned {
                    Some(proof) => {
                        BrowserReadEpisode::new_owned(platform, source.clone(), window, proof).await
                    }
                    None => BrowserReadEpisode::new(platform, source.clone(), window).await,
                }
            })
            .map_err(refusal_error)?;
        Ok(Self {
            runtime,
            source,
            episode,
            refs: HashMap::new(),
            verification_session: Uuid::new_v4().to_string(),
        })
    }
    pub(super) fn retire_for_navigation(
        &mut self,
    ) -> Result<dojo_cua_browser::browser_observation::BoundNavigationEndpoint> {
        let frozen = self.source.frozen.lock().unwrap().clone();
        ensure!(
            freeze(self.source.socket.as_deref(), frozen.identity.window_id)? == frozen,
            "browser geometry or identity changed; observe again"
        );
        self.refs.clear();
        self.runtime
            .block_on(self.episode.take_navigation_binding())
            .map_err(refusal_error)
    }

    pub(super) fn observe(&mut self) -> Result<Value> {
        self.observe_bounded(Duration::from_secs(10))
    }
    fn observe_bounded(&mut self, budget: Duration) -> Result<Value> {
        ensure!(
            budget >= Duration::from_millis(50),
            "insufficient browser observation budget"
        );
        let deadline = std::time::Instant::now() + budget;
        let socket_budget = deadline.saturating_duration_since(std::time::Instant::now());
        let window = self.source.frozen.lock().unwrap().identity.window_id;
        let before = freeze_bounded(self.source.socket.as_deref(), window, socket_budget)?;
        ensure!(
            before.identity == self.source.frozen.lock().unwrap().identity,
            "browser generation changed; reopen observation"
        );
        *self.source.frozen.lock().unwrap() = before.clone();
        self.refs.clear();
        let closed = self.episode.close_handle();
        let cancelled = self.source.cancelled.clone();
        let semantic_budget = deadline.saturating_duration_since(std::time::Instant::now());
        ensure!(
            !semantic_budget.is_zero(),
            "browser semantic proof budget exhausted"
        );
        let observation = self.runtime.block_on(async {
            tokio::select! {
                result = self.episode.observe() => result.map_err(refusal_error),
                _ = async { loop { if cancelled.load(Ordering::Acquire) { break; } tokio::time::sleep(Duration::from_millis(20)).await; } } => { closed.close().await; anyhow::bail!("browser session disconnected") },
                _ = tokio::time::sleep(semantic_budget) => { closed.close().await; anyhow::bail!("browser observation deadline exceeded") },
            }
        })?;
        ensure!(
            freeze_bounded(
                self.source.socket.as_deref(),
                window,
                deadline.saturating_duration_since(std::time::Instant::now())
            )? == before,
            "browser scene changed during observation"
        );
        ensure!(
            observation.png.len() <= 32 * 1024 * 1024 && observation.elements.len() <= 10_000,
            "browser observation exceeds budget"
        );
        ensure!(
            std::time::Instant::now() < deadline,
            "browser observation budget exhausted"
        );
        let elements: Vec<_> = observation
            .elements
            .into_iter()
            .map(|element| {
                let token = Uuid::new_v4().to_string();
                self.refs.insert(token.clone(), element.reference);
                json!({"ref":token,"node_name":element.node_name,"label":element.label})
            })
            .collect();
        Ok(
            json!({"snapshot":observation.snapshot,"url":observation.url,"png_base64":STANDARD.encode(observation.png),"image_width":observation.width,"image_height":observation.height,"elements":elements,"truncated":observation.truncated,"image_semantic_atomic":false,"actionable":false,"route":"browser_read_only"}),
        )
    }
    pub(super) fn verification_observation(
        &mut self,
        pid: i64,
        window_id: u64,
        budget: Duration,
    ) -> Result<dojo_computer_verification::ObservationSnapshot> {
        use dojo_computer_verification::observation::{
            BrowserDomEvidence, DomCoverage, SemanticElement, TargetIdentity, browser_observation,
        };
        let expected = self.source.frozen.lock().unwrap().clone();
        ensure!(
            i64::from(expected.identity.pid) == pid && expected.identity.window_id == window_id,
            "verification target differs from browser episode"
        );
        let observed = self.observe_bounded(budget)?;
        let current = self.source.frozen.lock().unwrap().clone();
        ensure!(
            current.identity == expected.identity,
            "verification browser generation changed"
        );
        let identity = TargetIdentity {
            session_id: self.verification_session.clone(),
            compositor_epoch: expected.identity.compositor_epoch.to_string(),
            window_id,
            client_generation: expected.identity.client_generation,
            pid,
            process_start: expected.identity.process_start,
        };
        let elements = observed["elements"]
            .as_array()
            .context("browser semantic elements missing")?
            .iter()
            .enumerate()
            .map(|(index, element)| SemanticElement {
                index: index as u64,
                label: element["label"].as_str().map(str::to_owned),
                ..Default::default()
            })
            .collect();
        browser_observation(
            &identity,
            BrowserDomEvidence {
                identity: identity.clone(),
                snapshot_id: observed["snapshot"]
                    .as_u64()
                    .context("snapshot identity missing")?,
                document_id: observed["snapshot"].to_string(),
                window: current.verification_window(&observed["url"]),
                elements,
                coverage: DomCoverage {
                    document_binding_current: true,
                    filtered_to_actionable: true,
                    truncated: observed["truncated"].as_bool().unwrap_or(true),
                    ..Default::default()
                },
            },
        )
    }
    pub(super) fn inspect(&self, token: &str) -> Result<Value> {
        ensure!(
            token.len() <= 128 && !self.source.cancelled.load(Ordering::Acquire),
            "invalid or cancelled browser ref"
        );
        let reference = self
            .refs
            .get(token)
            .context("browser ref is stale or belongs to another session")?;
        let cancelled = self.source.cancelled.clone();
        let closed = self.episode.close_handle();
        let element = self.runtime.block_on(async {
            tokio::select! {
                result = self.episode.resolve_ref(reference) => result.map_err(refusal_error),
                _ = async { loop { if cancelled.load(Ordering::Acquire) { break; } tokio::time::sleep(Duration::from_millis(20)).await; } } => { closed.close().await; anyhow::bail!("browser session disconnected") },
                _ = tokio::time::sleep(Duration::from_secs(10)) => { closed.close().await; anyhow::bail!("browser ref inspection deadline exceeded") },
            }
        })?;
        Ok(
            json!({"ref":token,"node_name":element.node_name,"label":element.label,"actionable":false}),
        )
    }
}
impl Drop for BrowserSession {
    fn drop(&mut self) {
        self.runtime.block_on(self.episode.close());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verification_uses_output_relative_bounds_on_offset_outputs() {
        let frozen = Frozen {
            identity: ExactWindow {
                compositor_epoch: Uuid::nil(),
                window_id: 19,
                client_generation: 19,
                pid: 23,
                process_start: 29,
            },
            compositor_pid: 31,
            compositor_start: 37,
            title: "Dojo".into(),
            windows_per_pid: 1,
            bounds: Rect::new(2020., 60., 260., 200.),
            scene_bounds: Rect::new(100., 60., 260., 200.),
        };
        assert_eq!(
            frozen.verification_window(&json!("https://example.test")),
            json!({"pid":23,"window_id":19,"title":"Dojo","url":"https://example.test",
                "bounds":{"x":100.0,"y":60.0,"width":260.0,"height":200.0}})
        );
    }
}
