//! Explicitly owned browser observation, with no registry or action scheduler.
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::sync::Notify;
use uuid::Uuid;

use super::engine::BrowserEngine;
use super::platform::BrowserPlatform;
use super::refusal::{BrowserRefusal, BrowserRefusalCode};
use super::types::ProcessFingerprint;
use crate::embedded::ExactWindow;

fn refused(message: impl Into<String>) -> BrowserRefusal {
    BrowserRefusal::new(BrowserRefusalCode::BrowserRouteUnavailable, message)
}

#[async_trait]
pub trait ExactWindowSource: Send + Sync {
    async fn current(&self, window_id: u64) -> Result<ExactWindow, BrowserRefusal>;
}

/// Host-owned isolated process proof. Tool arguments cannot install this provider.
#[async_trait]
pub trait OwnedBrowserSource: Send + Sync {
    fn browser_process(&self) -> (i32,u64);
    fn debugging_port(&self)->u16;
    async fn current_owned(&self, window: ExactWindow) -> Result<(), BrowserRefusal>;
}

pub(crate) struct ReadContext {
    pub key: String,
    pub window: ExactWindow,
    pub process_pid: i64,
    pub endpoint: String,
    pub owned: Option<Arc<dyn OwnedBrowserSource>>,
    fingerprint: ProcessFingerprint,
    platform: Arc<dyn BrowserPlatform>,
    identities: Arc<dyn ExactWindowSource>,
    pub state: Mutex<bool>,
    changed: Notify,
}

impl ReadContext {
    pub fn guard(&self) -> Result<MutexGuard<'_, bool>, BrowserRefusal> {
        let guard = self.state.lock().unwrap();
        if !*guard {
            return Err(refused("browser episode closed"));
        }
        Ok(guard)
    }
    pub fn close(&self) {
        *self.state.lock().unwrap() = false;
        self.changed.notify_waiters();
    }
    pub async fn closed(&self) {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !*self.state.lock().unwrap() {
                return;
            }
            notified.await;
        }
    }
    pub async fn check(&self) -> Result<(), BrowserRefusal> {
        drop(self.guard()?);
        let validate = async {
            if let Some(owned) = &self.owned { owned.current_owned(self.window).await?; }
            let current = self.identities.current(self.window.window_id).await?;
            let fingerprint = self
                .platform
                .process_fingerprint(self.process_pid)
                .await?;
            let endpoint = self
                .platform
                .discover_owned_endpoint(self.process_pid)
                .await?
                .ok_or_else(|| refused("owned endpoint disappeared"))?;
            if current != self.window
                || fingerprint != self.fingerprint
                || endpoint.ws_url != self.endpoint
                || endpoint.ownership.owner_pid != self.process_pid
            {
                return Err(refused("browser native/process/endpoint identity changed"));
            }
            drop(self.guard()?);
            Ok(())
        };
        tokio::select! {
            _ = self.closed() => Err(refused("browser episode closed")),
            result = tokio::time::timeout(Duration::from_secs(2), validate) =>
                result.map_err(|_| refused("browser identity proof timed out"))?,
        }
    }
    pub async fn bounded<T>(
        &self,
        work: impl Future<Output = Result<T, BrowserRefusal>>,
    ) -> Result<T, BrowserRefusal> {
        tokio::select! {
            _ = self.closed() => Err(refused("browser episode closed")),
            result = tokio::time::timeout(Duration::from_secs(5), work) =>
                result.map_err(|_| refused("browser observation timed out"))?,
        }
    }
}

/// Commands are observations or bounded protocol attachments, with exact
/// parameter shapes. Unknown commands and arbitrary JavaScript are refused.
pub(crate) fn permits(method: &str, params: &Value) -> bool {
    let Some(object) = params.as_object() else {
        return false;
    };
    let keys = |allowed: &[&str]| object.keys().all(|key| allowed.contains(&key.as_str()));
    let string = |key: &str| params[key].as_str().is_some_and(|value| !value.is_empty());
    match method {
        "Target.getTargets" | "Page.getFrameTree" | "Page.getLayoutMetrics" | "Page.enable" => {
            object.is_empty()
        }
        "Browser.getWindowForTarget" => keys(&["targetId"]) && string("targetId"),
        "Browser.getWindowBounds" => keys(&["windowId"]) && params["windowId"].as_i64().is_some(),
        "Target.attachToTarget" => {
            keys(&["targetId", "flatten"]) && string("targetId") && params["flatten"] == true
        }
        "Target.detachFromTarget" => keys(&["sessionId"]) && string("sessionId"),
        "Target.setAutoAttach" => {
            keys(&["autoAttach", "waitForDebuggerOnStart", "flatten"])
                && params["autoAttach"].is_boolean()
                && params["waitForDebuggerOnStart"] == false
                && params["flatten"] == true
        }
        "DOM.getDocument" => {
            keys(&["depth", "pierce"])
                && params["pierce"] == true
                && params["depth"]
                    .as_i64()
                    .is_some_and(|depth| (-1..=64).contains(&depth))
        }
        "DOM.describeNode" => {
            keys(&["backendNodeId", "depth", "pierce"])
                && params["backendNodeId"].as_i64().is_some_and(|id| id > 0)
                && params["depth"]
                    .as_i64()
                    .is_some_and(|depth| (-1..=64).contains(&depth))
                && params["pierce"] == true
        }
        "DOMSnapshot.captureSnapshot" => {
            keys(&["computedStyles", "includePaintOrder", "includeDOMRects"])
                && params["computedStyles"] == json!(super::semantic::SEMANTIC_COMPUTED_STYLES)
                && params["includePaintOrder"] == true
                && params["includeDOMRects"] == true
        }
        "Accessibility.getFullAXTree" => {
            keys(&["frameId"]) && (object.is_empty() || string("frameId"))
        }
        "Page.captureScreenshot" => {
            keys(&["format", "fromSurface", "captureBeyondViewport", "clip"])
                && params["format"] == "png"
                && params["fromSurface"] == true
                && params["captureBeyondViewport"] == false
                && params["clip"].as_object().is_some_and(|clip| {
                    clip.len() == 5
                        && ["x", "y", "width", "height", "scale"].iter().all(|key| {
                            clip.get(*key)
                                .and_then(Value::as_f64)
                                .is_some_and(f64::is_finite)
                        })
                        && params["clip"]["width"]
                            .as_f64()
                            .is_some_and(|n| n > 0.0 && n <= 32768.0)
                        && params["clip"]["height"]
                            .as_f64()
                            .is_some_and(|n| n > 0.0 && n <= 32768.0)
                        && params["clip"]["scale"] == 1.0
                })
        }
        "Runtime.evaluate" => {
            params
                == &json!({
                    "expression": "document.visibilityState === 'visible'", "returnByValue": true, "awaitPromise": false
                })
        }
        // Imported observations use DOM/AX methods. No callFunctionOn query is
        // needed here, so every function invocation remains refused.
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub struct BrowserElementRef {
    owner: String,
    target: String,
    tab: String,
    external: String,
}
#[derive(Clone, Debug)]
pub struct BrowserElement {
    pub reference: BrowserElementRef,
    pub node_name: String,
    pub label: Option<String>,
}
pub struct BrowserReadObservation {
    pub snapshot: u64,
    pub url: String,
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub elements: Vec<BrowserElement>,
    pub truncated: bool,
    pub image_semantic_atomic: bool,
}

pub struct BrowserReadEpisode {
    pub(super) engine: Arc<BrowserEngine>,
    pub(crate) context: Arc<ReadContext>,
    binding: Option<(String, String)>,
}

/// Fresh engine-produced page/window binding for an external admission broker.
/// Producing this record retires the read episode; it does not authorize mutation.
#[derive(Debug)]
pub struct BoundNavigationEndpoint {
    pub window: ExactWindow,
    pub websocket_url: String,
    pub page_target: String,
}

impl BrowserReadEpisode {
    pub async fn new(
        platform: Arc<dyn BrowserPlatform>,
        identities: Arc<dyn ExactWindowSource>,
        window_id: u64,
    ) -> Result<Self, BrowserRefusal> {
        tokio::time::timeout(
            Duration::from_secs(5),
            Self::new_inner(platform, identities, window_id, None),
        )
        .await
        .map_err(|_| refused("browser attachment timed out"))?
    }

    pub async fn new_owned(platform: Arc<dyn BrowserPlatform>, identities: Arc<dyn ExactWindowSource>, window_id:u64, owned:Arc<dyn OwnedBrowserSource>) -> Result<Self,BrowserRefusal> {
        tokio::time::timeout(Duration::from_secs(5), Self::new_inner(platform,identities,window_id,Some(owned))).await.map_err(|_|refused("owned browser attachment timed out"))?
    }

    async fn new_inner(
        platform: Arc<dyn BrowserPlatform>,
        identities: Arc<dyn ExactWindowSource>,
        window_id: u64,
        owned: Option<Arc<dyn OwnedBrowserSource>>,
    ) -> Result<Self, BrowserRefusal> {
        let window = identities.current(window_id).await?;
        let (process_pid,process_start)=owned.as_ref().map(|proof|proof.browser_process()).unwrap_or((window.pid,window.process_start));
        let process_pid=i64::from(process_pid);
        let fingerprint = platform.process_fingerprint(process_pid).await?;
        if fingerprint.start_time != Some(process_start) {
            return Err(refused("process generation is unproved"));
        }
        let endpoint = platform
            .discover_owned_endpoint(process_pid)
            .await?
            .ok_or_else(|| {
                refused("read-only browser attachment requires an existing owned endpoint")
            })?;
        if endpoint.ownership.owner_pid != process_pid {
            return Err(refused("endpoint owner mismatch"));
        }
        let context = Arc::new(ReadContext {
            key: Uuid::new_v4().to_string(),
            window,
            process_pid,
            endpoint: endpoint.ws_url,
            owned,
            fingerprint,
            platform: Arc::clone(&platform),
            identities,
            state: Mutex::new(true),
            changed: Notify::new(),
        });
        context.check().await?;
        let engine = BrowserEngine::new_read_only(platform, Arc::clone(&context));
        Ok(Self {
            engine,
            context,
            binding: None,
        })
    }

    pub async fn observe(&mut self) -> Result<BrowserReadObservation, BrowserRefusal> {
        self.context.check().await?;
        {
            let _publication = self.context.guard()?;
            self.engine.store.remove_session(&self.context.key);
            self.binding = None;
        }
        let (target, record) = self
            .context
            .bounded(self.engine.bind_native(
                &self.context.key,
                None,
                self.context.process_pid,
                self.context.window.window_id,
            ))
            .await?;
        let tabs = record
            .tabs
            .values()
            .filter(|tab| tab.active == Some(true))
            .collect::<Vec<_>>();
        if tabs.len() != 1 {
            return Err(refused("selected browser tab is not uniquely proved"));
        }
        self.binding = Some((target, tabs[0].tab_id.clone()));
        let (target, tab) = self.binding.as_ref().unwrap();
        let screenshot = self
            .context
            .bounded(
                self.engine
                    .capture_tab_screenshot(&self.context.key, target, tab),
            )
            .await?;
        let snapshot = self
            .context
            .bounded(self.engine.snapshot_tab(&self.context.key, target, tab))
            .await?;
        self.context.check().await?;
        let _publication = self.context.guard()?;
        Ok(BrowserReadObservation {
            snapshot: snapshot.snapshot_id,
            url: snapshot.url,
            png: STANDARD
                .decode(screenshot.data_base64)
                .map_err(|_| refused("malformed screenshot encoding"))?,
            width: screenshot.width,
            height: screenshot.height,
            truncated: snapshot.truncated,
            image_semantic_atomic: false,
            elements: snapshot
                .refs
                .into_iter()
                .map(|(external, entry)| BrowserElement {
                    reference: BrowserElementRef {
                        owner: self.context.key.clone(),
                        target: target.clone(),
                        tab: tab.clone(),
                        external,
                    },
                    node_name: entry.node_name,
                    label: entry.label,
                })
                .collect(),
        })
    }

    pub async fn resolve_ref(
        &self,
        reference: &BrowserElementRef,
    ) -> Result<BrowserElement, BrowserRefusal> {
        self.context.check().await?;
        if reference.owner != self.context.key {
            return Err(refused("browser reference belongs to another episode"));
        }
        let entry = self.engine.store.resolve_ref(
            &self.context.key,
            &reference.target,
            &reference.tab,
            &reference.external,
        )?;
        let expected = entry
            .frame
            .identity
            .as_ref()
            .ok_or_else(|| refused("browser document identity unproved"))?;
        if entry.frame.oopif_target_id.is_some() {
            return Err(refused(
                "OOPIF read resolution requires its attached frame session",
            ));
        }
        let record = self
            .engine
            .store
            .get_target(&self.context.key, &reference.target)?;
        let tab = record
            .tabs
            .get(&reference.tab)
            .ok_or_else(|| refused("browser tab stale"))?;
        let conn = self.engine.connect(&record.ws_url).await?;
        let cdp_session = self.engine.attach(&conn, &tab.cdp_target_id).await?;
        let tree = self
            .context
            .bounded(async {
                conn.call(Some(&cdp_session), "Page.getFrameTree", json!({}))
                    .await
                    .map_err(|e| refused(e.to_string()))
            })
            .await?;
        fn contains(tree: &Value, id: &str, loader: &str) -> bool {
            tree["frame"]["id"] == id && tree["frame"]["loaderId"] == loader
                || tree["childFrames"].as_array().is_some_and(|children| {
                    children.iter().any(|child| contains(child, id, loader))
                })
        }
        if !contains(&tree["frameTree"], &expected.frame_id, &expected.loader_id) {
            return Err(refused("browser frame document changed"));
        }
        let _publication = self.context.guard()?;
        Ok(BrowserElement {
            reference: reference.clone(),
            node_name: entry.node_name,
            label: entry.label,
        })
    }

    /// Revalidate the existing exact selected page and relinquish read-only
    /// connections before a separately approved compositor broker may connect.
    pub async fn take_navigation_binding(
        &mut self,
    ) -> Result<BoundNavigationEndpoint, BrowserRefusal> {
        self.context.check().await?;
        if self.context.process_pid != i64::from(self.context.window.pid) {return Err(refused("navigation final writer does not admit split native-client/browser-root bindings"));}
        let (target, tab) = self
            .binding
            .as_ref()
            .ok_or_else(|| refused("fresh browser observation required"))?;
        let record = self.engine.store.get_target(&self.context.key, target)?;
        if record.quality != super::types::BindingQuality::Exact
            || !matches!(
                record.endpoint_access_class,
                super::types::EndpointAccessClass::EmbeddedApplication
                    | super::types::EndpointAccessClass::DriverOwned
            )
            || record.pid != self.context.process_pid
            || record.window_id != self.context.window.window_id
            || record.endpoint_owner_pid != record.pid
        {
            return Err(refused("exact permitted native browser binding required"));
        }
        let page = record
            .tabs
            .get(tab)
            .ok_or_else(|| refused("browser tab stale"))?;
        if page.active != Some(true) {
            return Err(refused("active page identity unproved"));
        }
        // Re-bind read-only selection rather than trusting cached active flags.
        let (_, live) = self
            .context
            .bounded(self.engine.bind_native(
                &self.context.key,
                None,
                self.context.process_pid,
                self.context.window.window_id,
            ))
            .await?;
        let selected = live
            .tabs
            .values()
            .filter(|tab| tab.active == Some(true))
            .collect::<Vec<_>>();
        if live.quality != super::types::BindingQuality::Exact
            || selected.len() != 1
            || selected[0].cdp_target_id != page.cdp_target_id
            || live.ws_url != record.ws_url
        {
            return Err(refused(
                "browser selected page changed during admission preparation",
            ));
        }
        let binding = BoundNavigationEndpoint {
            window: self.context.window,
            websocket_url: record.ws_url.clone(),
            page_target: page.cdp_target_id.clone(),
        };
        self.context.check().await?;
        self.binding = None;
        self.close().await;
        Ok(binding)
    }

    pub fn close_handle(&self) -> BrowserReadCloseHandle {
        BrowserReadCloseHandle {
            context: Arc::clone(&self.context),
            engine: Arc::downgrade(&self.engine),
        }
    }

    pub async fn close(&self) {
        self.context.close();
        self.engine.store.remove_session(&self.context.key);
        self.engine.pool.close_read_only().await;
    }
}
pub struct BrowserReadCloseHandle {
    context: Arc<ReadContext>,
    engine: std::sync::Weak<BrowserEngine>,
}
impl BrowserReadCloseHandle {
    pub async fn close(&self) {
        self.context.close();
        if let Some(engine) = self.engine.upgrade() {
            engine.store.remove_session(&self.context.key);
            engine.pool.close_read_only().await;
        }
    }
}

impl Drop for BrowserReadEpisode {
    fn drop(&mut self) {
        self.context.close();
        self.engine.store.remove_session(&self.context.key);
    }
}

#[cfg(test)]
mod tests {
    use super::super::mock_cdp::{MockCdpServer, MockReply};
    use super::super::platform::{PrepareProfile, PrepareProfileMode, PrepareRequest};
    use super::super::v2_tests;
    use super::*;

    struct Identity(Mutex<ExactWindow>);
    #[async_trait]
    impl ExactWindowSource for Identity {
        async fn current(&self, _window_id: u64) -> Result<ExactWindow, BrowserRefusal> {
            Ok(*self.0.lock().unwrap())
        }
    }
    fn identity() -> Arc<Identity> {
        Arc::new(Identity(Mutex::new(ExactWindow {
            compositor_epoch: Uuid::new_v4(),
            window_id: 7,
            client_generation: 1,
            pid: 1,
            process_start: 1,
        })))
    }
    async fn episode(fixture: &v2_tests::Fixture) -> BrowserReadEpisode {
        BrowserReadEpisode::new(Arc::clone(&fixture.engine.platform), identity(), 7)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn retained_engine_queries_real_mock_endpoint_and_preserves_dom_refs() {
        let fixture = v2_tests::fixture().await;
        let mut reader = episode(&fixture).await;
        let observed = reader.observe().await.unwrap();
        assert_eq!((observed.width, observed.height), (1, 1));
        assert_eq!(&observed.png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            observed
                .elements
                .iter()
                .map(|element| element.node_name.as_str())
                .collect::<Vec<_>>(),
            vec!["button", "input", "input", "button", "input"]
        );
        assert_eq!(
            reader
                .resolve_ref(&observed.elements[0].reference)
                .await
                .unwrap()
                .node_name,
            "button"
        );
        assert!(
            fixture
                .state
                .lock()
                .unwrap()
                .calls
                .iter()
                .any(|(_, method, _)| method == "DOM.getDocument")
        );
        assert!(reader.engine.session_end_hook.lock().unwrap().is_none());
        reader.close().await;
    }

    #[tokio::test]
    async fn cross_episode_and_superseded_refs_refuse() {
        let fixture = v2_tests::fixture().await;
        let mut first = episode(&fixture).await;
        let second = episode(&fixture).await;
        let old = first.observe().await.unwrap().elements.remove(0).reference;
        assert!(second.resolve_ref(&old).await.is_err());
        first.observe().await.unwrap();
        assert!(first.resolve_ref(&old).await.is_err());
        first.close().await;
        second.close().await;
    }

    #[tokio::test]
    async fn changed_document_loader_refuses_even_without_another_snapshot() {
        let fixture = v2_tests::fixture().await;
        let mut reader = episode(&fixture).await;
        let reference = reader.observe().await.unwrap().elements.remove(0).reference;
        fixture.state.lock().unwrap().main_loader = "changed-document".into();
        assert!(reader.resolve_ref(&reference).await.is_err());
        reader.close().await;
    }

    #[tokio::test]
    async fn changed_native_process_generation_refuses_before_new_transport_work() {
        let fixture = v2_tests::fixture().await;
        let identities = identity();
        let mut reader =
            BrowserReadEpisode::new(Arc::clone(&fixture.engine.platform), identities.clone(), 7)
                .await
                .unwrap();
        reader.observe().await.unwrap();
        let before = fixture.state.lock().unwrap().calls.len();
        identities.0.lock().unwrap().process_start = 2;
        assert!(reader.observe().await.is_err());
        assert_eq!(fixture.state.lock().unwrap().calls.len(), before);
        reader.close().await;
    }

    #[tokio::test]
    async fn inner_transport_refuses_mutation_arbitrary_javascript_and_unknown_commands() {
        let fixture = v2_tests::fixture().await;
        let reader = episode(&fixture).await;
        let conn = reader
            .engine
            .connect(&reader.context.endpoint)
            .await
            .unwrap();
        let before = fixture.state.lock().unwrap().calls.len();
        for (method, params) in [
            ("Page.navigate", json!({"url":"https://example.test"})),
            ("Input.insertText", json!({"text":"bad"})),
            ("Browser.setDownloadBehavior", json!({"behavior":"allow"})),
            (
                "Runtime.evaluate",
                json!({"expression":"document.body.innerHTML = 'bad'"}),
            ),
            (
                "Runtime.callFunctionOn",
                json!({"functionDeclaration":"function(){this.click()}"}),
            ),
            (
                "Target.setAutoAttach",
                json!({"autoAttach":true,"flatten":true,"waitForDebuggerOnStart":true}),
            ),
            ("Unknown.command", json!({})),
        ] {
            assert!(conn.call(None, method, params).await.is_err(), "{method}");
        }
        assert_eq!(fixture.state.lock().unwrap().calls.len(), before);
        let prepared = reader
            .engine
            .prepare_browser(PrepareRequest {
                pid: None,
                window_id: None,
                session: reader.context.key.clone(),
                transport_session: None,
                strategy: None,
                profile: Some(PrepareProfile {
                    mode: PrepareProfileMode::IsolatedNew,
                    name: None,
                }),
                allow_launch: true,
                foreground: false,
            })
            .await;
        assert!(prepared.is_err());
        reader.close().await;
    }

    #[tokio::test]
    async fn close_invalidates_refs_and_is_not_reversed_by_imported_session_revive() {
        let fixture = v2_tests::fixture().await;
        let mut reader = episode(&fixture).await;
        let reference = reader.observe().await.unwrap().elements.remove(0).reference;
        let conn = reader
            .engine
            .connect(&reader.context.endpoint)
            .await
            .unwrap();
        reader.close().await;
        crate::session::end_session(&reader.context.key);
        crate::session::revive_session(&reader.context.key);
        assert!(reader.resolve_ref(&reference).await.is_err());
        assert!(
            conn.call(None, "Target.getTargets", json!({}))
                .await
                .is_err()
        );
        assert_eq!(reader.engine.store.target_count(&reader.context.key), 0);
    }

    #[tokio::test]
    async fn close_cancels_a_real_pending_cdp_reply_and_prevents_late_publication() {
        let fixture = v2_tests::fixture().await;
        let reader = episode(&fixture).await;
        let requested = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let request_notice = requested.clone();
        let reply_gate = release.clone();
        let server = MockCdpServer::start(Arc::new(move |_| {
            request_notice.notify_one();
            MockReply::ok(json!({"targetInfos":[]})).with_reply_gate(reply_gate.clone())
        }))
        .await;
        // Use the same actual platform seam, attesting this new fixture-owned
        // endpoint through its recorded discovery responses.
        let mut endpoint = fixture
            .engine
            .platform
            .discover_owned_endpoint(1)
            .await
            .unwrap()
            .unwrap();
        endpoint.ws_url = server.ws_url();
        fixture.state.lock().unwrap().endpoint_responses =
            std::collections::VecDeque::from([Ok(Some(endpoint))]);
        reader.context.close();
        let reader = episode(&fixture).await;
        let conn = reader
            .engine
            .connect(&reader.context.endpoint)
            .await
            .unwrap();
        let calling = conn.clone();
        let pending =
            tokio::spawn(async move { calling.call(None, "Target.getTargets", json!({})).await });
        tokio::time::timeout(Duration::from_secs(1), requested.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), reader.close())
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), pending)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        release.notify_one();
        assert_eq!(reader.engine.store.target_count(&reader.context.key), 0);
    }

    #[tokio::test]
    async fn closing_a_real_engine_observation_cancels_reply_and_retains_no_snapshot() {
        let fixture = v2_tests::fixture().await;
        let requested = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let notice = requested.clone();
        let gate = release.clone();
        let retained_handler = v2_tests::fixture_handler(fixture.state.clone());
        let server = MockCdpServer::start(Arc::new(move |call| {
            let reply = retained_handler(call);
            if call.method == "DOM.getDocument" {
                notice.notify_one();
                reply.with_reply_gate(gate.clone())
            } else {
                reply
            }
        }))
        .await;
        let mut endpoint = fixture
            .engine
            .platform
            .discover_owned_endpoint(1)
            .await
            .unwrap()
            .unwrap();
        endpoint.ws_url = server.ws_url();
        fixture.state.lock().unwrap().endpoint_responses =
            std::collections::VecDeque::from([Ok(Some(endpoint))]);
        let mut reader = episode(&fixture).await;
        let close = reader.close_handle();
        let engine = reader.engine.clone();
        let key = reader.context.key.clone();
        let observation = tokio::spawn(async move { reader.observe().await });
        tokio::time::timeout(Duration::from_secs(1), requested.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), close.close())
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), observation)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        release.notify_one();
        assert_eq!(engine.store.target_count(&key), 0);
    }

    #[tokio::test]
    async fn every_imported_mutation_tool_refuses_before_parsing_or_adapters() {
        use super::super::download::BrowserDownloadTool;
        use super::super::pointer::BrowserPointerTool;
        use super::super::tools::{
            BrowserClickTool, BrowserDialogTool, BrowserNavigateTool, BrowserPrepareTool,
            BrowserSetInputFilesTool, BrowserTypeTool,
        };
        use crate::tool::Tool;
        let fixture = v2_tests::fixture().await;
        let reader = episode(&fixture).await;
        let engine = reader.engine.clone();
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(BrowserPrepareTool::new(engine.clone())),
            Box::new(BrowserNavigateTool::new(engine.clone())),
            Box::new(BrowserClickTool::new(engine.clone())),
            Box::new(BrowserTypeTool::new(engine.clone())),
            Box::new(BrowserDialogTool::new(engine.clone())),
            Box::new(BrowserSetInputFilesTool::new(engine.clone())),
            Box::new(BrowserDownloadTool::new(engine.clone())),
            Box::new(BrowserPointerTool::new(engine)),
        ];
        let before = fixture.state.lock().unwrap().calls.len();
        for tool in tools {
            let result = tool.invoke(json!({})).await;
            let structured = result.structured_content.unwrap();
            assert_eq!(structured["status"], "refused", "{}", tool.def().name);
            assert_eq!(
                structured["refusal"]["message"],
                "external mutation admission unproved in read-only episode"
            );
        }
        assert_eq!(fixture.state.lock().unwrap().calls.len(), before);
        reader.close().await;
    }

    #[test]
    fn trusted_visibility_query_requires_exact_expression_and_parameters() {
        let trusted = json!({"expression":"document.visibilityState === 'visible'","returnByValue":true,"awaitPromise":false});
        assert!(permits("Runtime.evaluate", &trusted));
        assert!(!permits(
            "Runtime.evaluate",
            &json!({"expression":"document.visibilityState === 'visible'; alert(1)","returnByValue":true,"awaitPromise":false})
        ));
        assert!(!permits(
            "Runtime.evaluate",
            &json!({"expression":"document.visibilityState === 'visible'","returnByValue":true,"awaitPromise":false,"userGesture":true})
        ));
    }
}
