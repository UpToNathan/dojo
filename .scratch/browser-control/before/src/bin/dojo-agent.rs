use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[path = "dojo-agent/browser.rs"]
mod browser;
#[path = "dojo-agent/browser_mutation.rs"]
mod browser_mutation;
#[path = "dojo-agent/browser_owned.rs"]
mod browser_owned;
#[path = "dojo-agent/discovery.rs"]
mod discovery;
#[path = "dojo-agent/driver.rs"]
mod driver;
#[path = "dojo-agent/perception.rs"]
mod perception;
#[path = "dojo-agent/policy.rs"]
mod policy;
#[path = "dojo-agent/screenshot.rs"]
mod screenshot;
#[path = "dojo-agent/service.rs"]
mod service;
#[path = "dojo-agent/verification.rs"]
mod verification;
use service::ActiveSession;

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use dojo::agent_input::{AgentController, InputState};
use dojo_computer::{CaptureAction, CaptureScope};
use niri_ipc::{Request, Response, socket::Socket};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct DesktopLaunch {
    /// Desktop entry id, or "browser" for the user's default browser.
    application: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct DriverCall {
    name: String,
    arguments: Value,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AccessibilityRequest {
    window_id: u64,
    /// Optional read-only query. Returned indices are display labels, not action refs.
    query: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserRequest {
    window_id: u64,
}
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserNavigateRequest {
    capture_id: String,
    url: String,
}
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BrowserRefRequest {
    reference: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(transform = action_object_schema)]
enum DesktopAction {
    Focus { window_id: u64 },
    Move { output: String, x: f64, y: f64 },
    Click { button: MouseButton },
    Key { evdev: u32, state: KeyState },
    Scroll { horizontal: f64, vertical: f64 },
    Reset {},
}

fn action_object_schema(schema: &mut schemars::Schema) {
    schema.insert("type".into(), json!("object"));
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum KeyState {
    Press,
    Release,
    Tap,
}

impl DesktopAction {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Key { evdev, .. } if !(1..=255).contains(evdev) => {
                bail!("evdev must be between 1 and 255")
            }
            Self::Move { output, x, y } => {
                if output.is_empty() || !x.is_finite() || !y.is_finite() || *x < 0.0 || *y < 0.0 {
                    bail!("move requires an output name and finite nonnegative coordinates");
                }
            }
            Self::Scroll {
                horizontal,
                vertical,
            } => {
                if !horizontal.is_finite() || !vertical.is_finite() {
                    bail!("scroll amounts must be finite");
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn apply(self, controller: &mut AgentController) -> Result<()> {
        match self {
            Self::Focus { window_id } => controller.focus_window(window_id),
            Self::Move { output, x, y } => controller.move_pointer(&output, x, y),
            Self::Click { button } => {
                let code = match button {
                    MouseButton::Left => 272,
                    MouseButton::Right => 273,
                    MouseButton::Middle => 274,
                };
                controller.button(code, InputState::Pressed)?;
                controller.button(code, InputState::Released)
            }
            Self::Key { evdev, state } => match state {
                KeyState::Press => controller.key(evdev, InputState::Pressed),
                KeyState::Release => controller.key(evdev, InputState::Released),
                KeyState::Tap => {
                    controller.key(evdev, InputState::Pressed)?;
                    controller.key(evdev, InputState::Released)
                }
            },
            Self::Scroll {
                horizontal,
                vertical,
            } => controller.scroll(horizontal, vertical),
            Self::Reset {} => controller.reset(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct DesktopFeedback {
    /// Brief text to display in a floating speech bubble, if there is a response.
    response: Option<String>,
    /// True when the task is finished. Restores the normal Arch icon.
    #[serde(default)]
    complete: bool,
}

fn shell_event(event: dojo::speech::SpeechEvent) -> Result<()> {
    event.validate()?;
    anyhow::ensure!(
        dojo::control::send(&dojo::control::ControlCommand::Speech(event))?,
        "Dojo shell is not running"
    );
    Ok(())
}

struct DesktopSession {
    socket: Option<PathBuf>,
    name: String,
    controller: Option<AgentController>,
}

impl DesktopSession {
    fn action(&mut self, action: DesktopAction) -> Result<Value> {
        action.validate()?;
        if matches!(action, DesktopAction::Reset {}) && self.controller.is_none() {
            return Ok(json!({"seat_name": null, "handled": true}));
        }
        if self.controller.is_none() {
            self.controller = Some(match &self.socket {
                Some(path) => AgentController::connect_to(path, &self.name),
                None => AgentController::connect(&self.name),
            }?);
        }
        let controller = self
            .controller
            .as_mut()
            .context("agent seat is unavailable")?;
        if let Err(error) = action.apply(controller) {
            if let Some(paused) = error.downcast_ref::<dojo::agent_input::InputPaused>() {
                return Ok(
                    json!({"seat_name": controller.seat_name(), "handled": false, "paused": true,
                    "reason": paused.reason, "coordination": paused.coordination}),
                );
            }
            self.controller = None;
            return Err(error.context("input failed; agent seat released, select a fresh target"));
        }
        Ok(json!({"seat_name": controller.seat_name(), "handled": true}))
    }
}

struct DesktopAdapter {
    socket: Option<PathBuf>,
    name: String,
    idle_timeout: Duration,
    connection: Option<ActiveSession>,
}

impl DesktopAdapter {
    fn driver_connection(&mut self) -> Result<&mut ActiveSession> {
        self.refresh_connection()?;
        if self.connection.is_none() {
            self.connection = Some(ActiveSession::spawn(
                self.socket.as_deref(),
                &self.name,
                self.idle_timeout,
            )?);
        }
        Ok(self.connection.as_mut().unwrap())
    }
    fn refresh_connection(&mut self) -> Result<()> {
        if let Some(connection) = &mut self.connection {
            if connection.inactive()? {
                self.connection = None;
            }
        }
        Ok(())
    }

    fn context(&mut self) -> Result<Value> {
        self.refresh_connection()?;
        let mut socket = desktop_socket(self.socket.as_deref())
            .context("connect to Niri for desktop context")?;
        let Response::Windows(windows) =
            socket.send(Request::Windows)?.map_err(anyhow::Error::msg)?
        else {
            bail!("unexpected windows response");
        };
        let Response::Outputs(outputs) =
            socket.send(Request::Outputs)?.map_err(anyhow::Error::msg)?
        else {
            bail!("unexpected outputs response");
        };
        let coordination = match socket
            .send(Request::InputCoordination)?
            .map_err(anyhow::Error::msg)?
        {
            Response::InputCoordination(coordination) => coordination,
            _ => bail!("unexpected input coordination response"),
        };
        let driver_pid = self
            .connection
            .as_mut()
            .map(ActiveSession::driver_pid)
            .transpose()?
            .flatten();
        Ok(json!({
            "coordination": coordination,
            "driver_worker_pid": driver_pid,
            "windows": windows,
            "outputs": outputs,
            "seat_name": self.connection.as_ref().and_then(|connection| connection.seat_name.as_deref()),
            "worker_pid": self.connection.as_ref().filter(|connection| connection.seat_name.is_some()).map(ActiveSession::pid),
            "idle_timeout_seconds": self.idle_timeout.as_secs(),
            "coordinate_system": "logical pixels relative to the named output",
            "capabilities": {
                "contract_version": dojo_computer::CONTRACT_VERSION,
                "capture_actions": ["move", "click", "double_click", "scroll", "key", "chord", "key_hold", "text_ascii_us", "drag_in_window"],
                "browser": {"observation": "server_owned_isolated_or_operator_endpoint_with_exact_process", "mutation": "operator_opt_in_navigation_only_with_exact_compositor_admission"},
                "accessibility": {"observation": "bounded_application_scope", "mutation": "refused_pending_exact_window_correlation_and_admission"},
                "unicode_text": "unsupported", "clipboard": "unsupported", "cross_window_drag": "unsupported",
                "verification": ["window_exists", "window_title", "rendered_window_bounds", "exact_browser_url", "exact_browser_dom_labels"]
            },
        }))
    }

    fn observe(&mut self, scope: CaptureScope) -> Result<Value> {
        scope.validate()?;
        self.refresh_connection()?;
        if self.connection.is_none() {
            self.connection = Some(ActiveSession::spawn(
                self.socket.as_deref(),
                &self.name,
                self.idle_timeout,
            )?);
        }
        let result = self
            .connection
            .as_mut()
            .context("desktop service unavailable")?
            .observe(scope);
        if result.is_err() {
            self.connection = None;
        }
        result
    }

    fn capture_action(&mut self, action: CaptureAction) -> Result<Value> {
        action.validate()?;
        self.refresh_connection()?;
        let result = self
            .connection
            .as_mut()
            .context("observe a fresh target with desktop_observe before capture-based input")?
            .capture_action(action);
        if result.is_err() {
            self.connection = None;
        }
        result
    }

    fn action(&mut self, action: DesktopAction) -> Result<Value> {
        action.validate()?;
        self.refresh_connection()?;
        if matches!(action, DesktopAction::Reset {}) && self.connection.is_none() {
            return Ok(json!({"seat_name": null, "handled": true}));
        }
        if self.connection.is_none() {
            self.connection = Some(ActiveSession::spawn(
                self.socket.as_deref(),
                &self.name,
                self.idle_timeout,
            )?);
        }
        let result = self
            .connection
            .as_mut()
            .context("desktop service unavailable")?
            .action(action);
        if result.is_err() {
            self.connection = None;
        }
        result
    }
}

#[derive(Clone)]
struct DesktopServer {
    session: Arc<Mutex<DesktopAdapter>>,
}

#[tool_router]
impl DesktopServer {
    #[tool(
        description = "Discover the forked CUA Driver's original runtime tool schemas through Dojo's activated desktop service."
    )]
    async fn desktop_driver_tools(&self) -> CallToolResult {
        self.execute(|session| session.driver_connection()?.driver_tools())
            .await
    }

    #[tool(
        description = "Invoke a forked CUA Driver tool through the activated Dojo service. Use desktop_driver_tools for its runtime argument schema. Native actions use Dojo's coordinated seat and require a fresh capture."
    )]
    async fn desktop_driver_call(
        &self,
        Parameters(input): Parameters<DriverCall>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> CallToolResult {
        let session = self.session.clone();
        let cancellation = context.ct.clone();
        let (handoff, handle) = tokio::sync::oneshot::channel();
        let operation = tokio::task::spawn_blocking(move || -> Result<Value> {
            let mut session = session.blocking_lock();
            anyhow::ensure!(!cancellation.is_cancelled(), "Driver request cancelled");
            let connection = session.driver_connection()?;
            handoff
                .send(connection.cancel_on_drop()?)
                .map_err(|_| anyhow::anyhow!("Driver request cancelled"))?;
            connection.driver_call(input.name, input.arguments)
        });
        let mut guard = match handle.await {
            Ok(guard) => guard,
            Err(_) => {
                return match operation.await {
                    Ok(Err(error)) => {
                        CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))])
                    }
                    _ => CallToolResult::error(vec![ContentBlock::text(
                        "Driver request unavailable",
                    )]),
                };
            }
        };
        let result = tokio::select! {
            result = operation => result,
            _ = context.ct.cancelled() => return CallToolResult::error(vec![ContentBlock::text("Driver request cancelled; observe again before another action")]),
        };
        guard.complete();
        match result {
            Ok(Ok(value)) => serde_json::from_value(value).unwrap_or_else(|error| {
                CallToolResult::error(vec![ContentBlock::text(format!(
                    "invalid Driver result: {error}"
                ))])
            }),
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
                "Driver dispatch failed: {error}"
            ))]),
        }
    }
    fn new(socket: Option<PathBuf>, name: String, idle_timeout: Duration) -> Self {
        Self {
            session: Arc::new(Mutex::new(DesktopAdapter {
                socket,
                name,
                idle_timeout,
                connection: None,
            })),
        }
    }

    async fn execute(
        &self,
        operation: impl FnOnce(&mut DesktopAdapter) -> Result<Value> + Send + 'static,
    ) -> CallToolResult {
        let session = self.session.clone();
        let result =
            tokio::task::spawn_blocking(move || operation(&mut session.blocking_lock())).await;
        match result {
            Ok(Ok(mut value)) => {
                let png = value
                    .get_mut("post_observation")
                    .and_then(Value::as_object_mut)
                    .and_then(|observation| observation.remove("png_base64"))
                    .and_then(|encoded| encoded.as_str().map(str::to_owned));
                let image = png.filter(|encoded| STANDARD.decode(encoded).is_ok());
                let mut content = vec![ContentBlock::text(value.to_string())];
                if let Some(encoded) = image {
                    content.push(ContentBlock::image(encoded, "image/png"));
                }
                CallToolResult::success(content)
            }
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
                "desktop service failed: {error}"
            ))]),
        }
    }

    #[tool(
        description = "List Niri windows, outputs and input coordination with human_owned_windows and revision. Does not start the shared service or create an agent seat. Reports worker_pid as the shared service PID for an active seat; both fields are null while idle. Window/output snapshots are separate queries and may change between calls."
    )]
    async fn desktop_context(&self) -> CallToolResult {
        self.execute(|session| {
            policy::authorize("desktop_context", &json!({}))?;
            session.context()
        })
        .await
    }

    #[tool(
        description = "Discover Niri windows using the pinned CUA list_windows contract. Optional pid and on_screen_only filters. Bounds use global logical workspace-layout coordinates and are separate from capture-bound action authority. Stacking order and minimized state are unknown. Refuses unavailable geometry instead of inventing bounds. Does not create an agent seat.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_windows(
        &self,
        Parameters(input): Parameters<discovery::ListWindowsInput>,
    ) -> CallToolResult {
        self.execute(move |session| {
            policy::authorize("list_windows", &serde_json::to_value(&input)?)?;
            discovery::list_windows(session.socket.as_deref(), input)
        })
        .await
    }

    #[tool(
        description = "Return logical width, height and scale_factor for the focused Niri output, or the sole enabled output. Niri has no primary-display designation; reports the chosen output and selection rule. Refuses ambiguous layouts. Only this transport's implicit session is supported. Does not create an agent seat.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_screen_size(
        &self,
        Parameters(input): Parameters<discovery::ScreenSizeInput>,
    ) -> CallToolResult {
        self.execute(move |session| {
            policy::authorize("get_screen_size", &serde_json::to_value(&input)?)?;
            discovery::screen_size(session.socket.as_deref(), input)
        })
        .await
    }

    #[tool(
        description = "Discover installed desktop applications and Niri window-owning running applications. Uses Dojo's existing desktop-entry catalogue; running does not include background processes without windows. Reports pid zero for installed-only entries, exact observed PIDs for running entries, and active for the focused window's process. Does not launch applications or create an agent seat.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_apps(
        &self,
        Parameters(_input): Parameters<discovery::EmptyInput>,
    ) -> CallToolResult {
        self.execute(|session| {
            policy::authorize("list_apps", &json!({}))?;
            discovery::list_apps(session.socket.as_deref())
        })
        .await
    }

    #[tool(
        description = "Create one server-owned isolated Chromium window with an empty temporary profile and loopback debugging. Uses only the trusted operator browser executable and fixed launch flags. Caller cannot provide a profile, executable, PID, endpoint or argv. Returns its initial browser screenshot and DOM refs. Unsafe to replay. Close or release this session to terminate only its owned child and remove the profile."
    )]
    async fn desktop_browser_create(&self) -> CallToolResult {
        self.execute(|session| {
            session.refresh_connection()?;
            if session.connection.is_none() {
                session.connection = Some(ActiveSession::spawn(
                    session.socket.as_deref(),
                    &session.name,
                    session.idle_timeout,
                )?);
            }
            session
                .connection
                .as_mut()
                .context("desktop service unavailable")?
                .browser_create()
        })
        .await
    }
    #[tool(
        description = "Close this connection's owned isolated browser and retire its browser refs/captures. Reaps only the process launched by this session and deletes only its server-owned temporary profile. Existing user browsers remain outside this lifecycle. Does not clear unknown external-effect recovery barriers."
    )]
    async fn desktop_browser_close(&self) -> CallToolResult {
        self.execute(|session| {
            session.refresh_connection()?;
            if let Some(connection) = session.connection.as_mut() {
                connection.browser_close()
            } else {
                Ok(json!({"closed":false,"recovery_barrier_cleared":false}))
            }
        })
        .await
    }

    #[tool(
        description = "Observe a browser through the retained CUA read-only engine. window_id must have exact Niri scene/process identity and an operator-configured DOJO_BROWSER_CDP_ENDPOINT owned by that process. Does not launch or enable debugging. Consumer profiles require consent and are refused by this route; configured endpoints do not grant access. Returns tab screenshot, semantic elements and session-owned read-only refs. Screenshot and DOM are sequential samples, not atomic. All CDP mutation/preparation methods are refused at transport dispatch."
    )]
    async fn desktop_browser_observe(
        &self,
        Parameters(input): Parameters<BrowserRequest>,
    ) -> CallToolResult {
        let session = self.session.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut session = session.blocking_lock();
            anyhow::ensure!(input.window_id > 0, "invalid window_id");
            session.refresh_connection()?;
            if session.connection.is_none() {
                session.connection = Some(ActiveSession::spawn(
                    session.socket.as_deref(),
                    &session.name,
                    session.idle_timeout,
                )?);
            }
            session
                .connection
                .as_mut()
                .context("desktop service unavailable")?
                .browser_observe(input)
        })
        .await;
        match result {
            Ok(Ok(mut value)) => {
                let Some(png) = value
                    .as_object_mut()
                    .and_then(|v| v.remove("png_base64"))
                    .and_then(|v| v.as_str().map(str::to_owned))
                else {
                    return CallToolResult::error(vec![ContentBlock::text(
                        "browser observation has no image",
                    )]);
                };
                CallToolResult::success(vec![
                    ContentBlock::text(value.to_string()),
                    ContentBlock::image(png, "image/png"),
                ])
            }
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
                "browser observation failed: {error}"
            ))]),
        }
    }

    #[tool(
        description = "Navigate the exact observed embedded or isolated browser window through the optional compositor final writer. Requires a fresh exact-window desktop capture and current browser observation. The operator must enable DOJO_EXTERNAL_ADMISSION_ENABLE_NAVIGATE and pin the trusted broker. Consumes the capture and all browser refs before dispatch. Human takeover refuses queued effects. Admitted effects are unknown and never replayed; obtain fresh observations before recovery. Consumer browser profiles remain refused."
    )]
    async fn desktop_browser_navigate(
        &self,
        Parameters(input): Parameters<BrowserNavigateRequest>,
    ) -> CallToolResult {
        let session = self.session.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<Value> {
            let mut session = session.blocking_lock();
            if let Err(error) = session.refresh_connection() {
                return Ok(json!({"effect":"refused","accepted_bytes":0,"requires_fresh_observation":true,"reason":format!("navigation preflight failed: {error:#}")}));
            }
            let Some(connection) = session.connection.as_mut() else {
                return Ok(json!({"effect":"refused","accepted_bytes":0,"requires_fresh_observation":true,"reason":"fresh desktop and browser observations required"}));
            };
            match connection.browser_navigate(input) {
                Ok(value) => Ok(value),
                Err(error) => Ok(json!({"effect":"unknown","requires_fresh_observation":true,"recovery_required":true,"reason":format!("navigation transport failed: {error:#}")})),
            }
        }).await;
        let value = match result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                json!({"effect":"unknown","recovery_required":true,"requires_fresh_observation":true,"reason":format!("{error:#}")})
            }
            Err(error) => {
                json!({"effect":"unknown","recovery_required":true,"requires_fresh_observation":true,"reason":error.to_string()})
            }
        };
        CallToolResult::success(vec![ContentBlock::text(value.to_string())])
    }

    #[tool(
        description = "Inspect one retained read-only browser ref in this connection. Refuses closed, stale, cross-session or changed-document refs. It never clicks or changes an element. Call desktop_browser_observe for new refs after release, input or a new browser observation."
    )]
    async fn desktop_browser_inspect_ref(
        &self,
        Parameters(input): Parameters<BrowserRefRequest>,
    ) -> CallToolResult {
        self.execute(move |session| {
            anyhow::ensure!(input.reference.len() <= 128, "invalid browser ref");
            session.refresh_connection()?;
            session
                .connection
                .as_mut()
                .context("observe browser first")?
                .browser_inspect(input)
        })
        .await
    }

    #[tool(
        description = "Read a bounded AT-SPI accessibility tree for the process owning window_id. This is application-scoped and may include other windows of that process. It returns no actionable refs; display indices and application-reported rectangles are not native window evidence. Password text is redacted. Reads check process generation and accessibility bus owner identity. Unavailable/truncated trees carry explicit coverage issues. At most 500 nodes, 1024 text characters per field and a five-second deadline. No input, clipboard or accessibility mutation."
    )]
    async fn desktop_accessibility(
        &self,
        Parameters(input): Parameters<AccessibilityRequest>,
    ) -> CallToolResult {
        self.execute(move |session| {
            anyhow::ensure!(
                input.window_id > 0 && input.query.as_ref().is_none_or(|q| q.len() <= 1024),
                "invalid window/query"
            );
            session.refresh_connection()?;
            if session.connection.is_none() {
                session.connection = Some(ActiveSession::spawn(
                    session.socket.as_deref(),
                    &session.name,
                    session.idle_timeout,
                )?);
            }
            session
                .connection
                .as_mut()
                .context("desktop service unavailable")?
                .accessibility(input)
        })
        .await
    }

    #[tool(
        description = "Decode one canonical, UI-TARS/UI-TARS2, Qwen 3.5, Fara, OpenCUA, generic VLM, Anthropic, OpenAI native or Gemini computer-use step in Rust against an existing session-owned capture. Use only the unchanged PNG delivered by Dojo; resized/cropped model views are unsupported by this endpoint. Native envelopes retain provider signatures and pending safety checks; decoding does not acknowledge consent or implement provider transport. Returns typed delivered-image coordinates, units and outcome. This does not authorize, consume or execute input; desktop_capture_action independently revalidates freshness, capabilities and target. Unsupported/malformed programs are refused without evaluation."
    )]
    async fn desktop_decode_model(
        &self,
        Parameters(input): Parameters<dojo_computer::ModelDecode>,
    ) -> CallToolResult {
        self.execute(move |session| {
            input.validate()?;
            session.refresh_connection()?;
            session
                .connection
                .as_mut()
                .context("observe first to obtain a live capture")?
                .decode_model(input)
        })
        .await
    }

    #[tool(input_schema = perception::pinned_input_schema(), description = "Parse OCR and icon regions from an existing session-owned capture using the pinned local Rust perception worker. Read-only; preserves the capture and its expiry, returns noninteractive image-pixel regions, and grants no input permission. Requires a host-configured immutable installation and an exact-window capture. No replacement capture or implicit retry.", annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn parse_visual_regions(
        &self,
        Parameters(input): Parameters<perception::VisualParseInput>,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> CallToolResult {
        let session = self.session.clone();
        let cancellation = context.ct.clone();
        let (handoff, handle) = tokio::sync::oneshot::channel();
        let operation = tokio::task::spawn_blocking(move || -> Result<Value> {
            let mut session = session.blocking_lock();
            anyhow::ensure!(!cancellation.is_cancelled(), "perception request cancelled");
            input
                .0
                .validate()
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            session.refresh_connection()?;
            let connection = session
                .connection
                .as_mut()
                .context("observe first to obtain a live capture")?;
            let guard = connection.cancel_on_drop()?;
            handoff
                .send(guard)
                .map_err(|_| anyhow::anyhow!("perception request cancelled"))?;
            connection.parse_visual_regions(input)
        });
        let mut guard = match handle.await {
            Ok(guard) => guard,
            Err(_) => {
                return match operation.await {
                    Ok(Err(error)) => {
                        CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))])
                    }
                    _ => CallToolResult::error(vec![ContentBlock::text(
                        "perception request unavailable",
                    )]),
                };
            }
        };
        let result = tokio::select! {
            result = operation => result,
            _ = context.ct.cancelled() => return CallToolResult::error(vec![ContentBlock::text("perception request cancelled; observe again")]),
        };
        guard.complete();
        match result {
            Ok(Ok(value)) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
                "perception executor failed: {error}"
            ))]),
        }
    }

    #[tool(
        description = "Deterministically verify predicates against one exact process/window. Returns satisfied, unsatisfied or unknown after bounded consecutive samples. Verifies native window existence, title and rendered bounds; an exact browser session can supply read-only DOM label predicates. Ref-only DOM coverage cannot prove absence; application-scoped accessibility predicates return unknown. timeout_ms is 0 through 10000, stable_samples is 1 through 5. No input seat or persistent polling. include_screenshot=true is unsupported; call desktop_observe for visual evidence. handled action receipts do not imply success."
    )]
    async fn desktop_verify(
        &self,
        Parameters(input): Parameters<dojo_computer_verification::VerifyStateInput>,
    ) -> CallToolResult {
        self.execute(move |session| {
            input.validate()?;
            session.refresh_connection()?;
            if session.connection.is_none() {
                session.connection = Some(ActiveSession::spawn(
                    session.socket.as_deref(),
                    &session.name,
                    session.idle_timeout,
                )?);
            }
            session
                .connection
                .as_mut()
                .context("desktop service unavailable")?
                .verify(input)
        })
        .await
    }

    #[tool(
        description = "Capture a named output, an output-relative logical region, or the visible area of a window whose position Niri supplies. region and window_id are mutually exclusive; omit both for the full output. scale defaults to 1 and accepts 0.1 through 1 to reduce image size. Window capture requires its active workspace on the selected output and known geometry; Rendered tiled and floating geometry is supplied with the captured pixels; unsupported animations/transitions return an error. Returns PNG and logical origin plus per-axis mapping: desktop x/y = logical_x/y + image x/y * logical_x/y_per_image_pixel. No focus, clipboard or seat changes. Inspect before clicking and capture again to verify. Uses native Wayland capture with named-output and compositor-peer checks; DOJO_CAPTURE_BACKEND=grim explicitly selects the migration backend. Capture times out after five seconds."
    )]
    async fn desktop_screenshot(
        &self,
        Parameters(request): Parameters<screenshot::CaptureRequest>,
    ) -> CallToolResult {
        let socket = self.session.lock().await.socket.clone();
        match tokio::task::spawn_blocking(move || {
            policy::authorize("desktop_screenshot", &serde_json::to_value(&request)?)?;
            screenshot::capture(socket.as_deref(), &request)
        })
        .await
        {
            Ok(Ok(content)) => CallToolResult::success(content),
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
            Err(error) => {
                CallToolResult::error(vec![ContentBlock::text(format!("capture failed: {error}"))])
            }
        }
    }

    #[tool(
        description = "Observe visible pixels with a server-owned, session-bound capture ID, digest, exact process/window/output geometry and image mapping. Supply output and a positioned window_id for actionable observations; full output/region observations are read-only. Capture references expire after 30 seconds and are invalid after release, disconnect, restart or input. This starts bounded capture storage but no input seat, and does not renew seat idle time. Images may contain sensitive visible content and persist in the calling agent's transcript; select only the scope you intend to share. Uses native Wayland capture. Window observations use same-render Niri scene geometry; animations/transitions may be refused."
    )]
    async fn desktop_observe(&self, Parameters(scope): Parameters<CaptureScope>) -> CallToolResult {
        let session = self.session.clone();
        let result = tokio::task::spawn_blocking(move || {
            policy::authorize("desktop_observe", &serde_json::to_value(&scope)?)?;
            session.blocking_lock().observe(scope)
        })
        .await;
        match result {
            Ok(Ok(mut value)) => {
                let Some(encoded) = value
                    .as_object_mut()
                    .and_then(|v| v.remove("png_base64"))
                    .and_then(|v| v.as_str().map(str::to_owned))
                else {
                    return CallToolResult::error(vec![ContentBlock::text(
                        "capture service returned no PNG",
                    )]);
                };
                // Validate service image encoding before passing it to model clients.
                if STANDARD.decode(&encoded).is_err() {
                    return CallToolResult::error(vec![ContentBlock::text(
                        "capture service returned invalid PNG encoding",
                    )]);
                }
                CallToolResult::success(vec![
                    ContentBlock::text(value.to_string()),
                    ContentBlock::image(encoded, "image/png"),
                ])
            }
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
                "capture service failed: {error}"
            ))]),
        }
    }

    #[tool(
        description = "Use one fresh actionable desktop_observe capture. x/y are delivered PNG pixels. Set observe_after=true to return a fresh PNG and capture metadata after the attempt; capture failure retains the effect receipt. input kinds: move; printable ASCII text on a verified unmodified US layout; key_hold for 1 through 1000 milliseconds; in-window drag with 1 through 16 image-pixel waypoints; click/double_click with button left|right|middle; key with a physical key name; chord with 2 through 5 distinct key names; scroll with horizontal/vertical logical pixel deltas. Key aliases include Control, Enter, ArrowLeft and Page_Down. Physical letters depend on the target keyboard layout; Unicode text/clipboard insertion is unavailable. Every press rechecks the exact window and pinned coordination revision. References are consumed before effects, even on pause or unknown result. Never replay the ID; observe again. completed_prefix counts complete click/tap/chord operations; partial or unknown can include an unfinished effect. handled does not imply verified UI success. Requires the updated experimental compositor."
    )]
    async fn desktop_capture_action(
        &self,
        Parameters(action): Parameters<CaptureAction>,
    ) -> CallToolResult {
        self.execute(move |session| session.capture_action(action))
            .await
    }

    #[tool(
        description = "Act using your independent cursor and keyboard seat. Focus a window before keys, or move then click to select it. Coordinates are logical pixels relative to output. Keys use Linux evdev codes: A=30, Enter=28, Shift=42. Use press/release for chords and tap for a single key. Scroll uses pixel deltas. Reset releases held input and clears focus. Paused results retain the seat and report human ownership or stale state; inspect context and choose a new action without automatically retrying. Errors release the seat; select a fresh target afterward. Human Niri focus is unchanged. After the idle timeout the session releases its seat and loses its target; focus or click again before typing. Clipboard, Unicode text insertion and window management are unavailable."
    )]
    async fn desktop_action(
        &self,
        Parameters(action): Parameters<DesktopAction>,
    ) -> CallToolResult {
        self.execute(move |session| {
            action.validate()?;
            policy::authorize("desktop_action", &serde_json::to_value(&action)?)?;
            if let Err(error) = shell_event(dojo::speech::SpeechEvent::Phase(
                dojo::speech::SpeechPhase::Working,
            )) {
                eprintln!("Dojo working indicator unavailable: {error:#}");
            }
            let result = session.action(action);
            if result.is_err() {
                let _ = shell_event(dojo::speech::SpeechEvent::Phase(
                    dojo::speech::SpeechPhase::Settled,
                ));
            }
            result
        })
        .await
    }

    #[tool(
        description = "Launch a real desktop application and wait for its new Niri window. Use application=browser for the default desktop browser, or a desktop entry id. Launching is independent of human ownership of existing windows. Returns the verified new window; fails if no new window appears."
    )]
    async fn desktop_launch(
        &self,
        Parameters(request): Parameters<DesktopLaunch>,
    ) -> CallToolResult {
        self.execute(move |session| {
            policy::authorize("desktop_launch", &serde_json::to_value(&request)?)?;
            launch_application(&request.application, session.socket.as_deref())
        })
        .await
    }

    #[tool(
        description = "Show a brief visual response in Dojo's fading speech bubble. Use only when you have something to tell the user. Set complete=true when the task finishes to restore the Arch icon, including when no response is needed. Responses do not change task state on their own."
    )]
    async fn desktop_feedback(
        &self,
        Parameters(feedback): Parameters<DesktopFeedback>,
    ) -> CallToolResult {
        self.execute(move |_| {
            policy::authorize("desktop_feedback", &serde_json::to_value(&feedback)?)?;
            if let Some(text) = feedback.response {
                shell_event(dojo::speech::SpeechEvent::Response(text))?;
            }
            if feedback.complete {
                shell_event(dojo::speech::SpeechEvent::Phase(
                    dojo::speech::SpeechPhase::Settled,
                ))?;
            }
            Ok(json!({"displayed": true, "complete": feedback.complete}))
        })
        .await
    }

    #[tool(
        description = "Release this agent's seat, cursor and held input. Safe to repeat. A later action creates a new seat."
    )]
    async fn desktop_release(&self) -> CallToolResult {
        self.execute(|session| {
            if let Some(connection) = session.connection.take() {
                connection.release();
            }
            let _ = shell_event(dojo::speech::SpeechEvent::Phase(
                dojo::speech::SpeechPhase::Settled,
            ));
            Ok(json!({"released": true}))
        })
        .await
    }
}

#[tool_handler]
impl ServerHandler for DesktopServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("dojo-agent", env!("CARGO_PKG_VERSION")))
            .with_instructions("Desktop tools use an independent seat in the experimental Dojo Niri fork. Inspect desktop_context, then prefer desktop_observe on an exact positioned window and desktop_capture_action in its PNG pixels. Each action attempt requires another observation; never replay a consumed capture. Capture again to verify the UI result. desktop_screenshot and low-level desktop_action remain compatibility tools. Use desktop_feedback for brief visual responses and set complete=true when the task finishes. Release the seat when finished. desktop_accessibility supplies read-only application-scoped AT-SPI observations with no actionable refs. desktop_verify checks bounded window predicates, returning unknown for unavailable evidence. desktop_decode_model parses supported computer-use output against the unchanged delivered PNG without executing it.")
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 32
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("name must contain 1..32 ASCII letters, digits, hyphens or underscores");
    }
    Ok(())
}

fn desktop_socket(socket: Option<&std::path::Path>) -> Result<Socket> {
    let socket = match socket {
        Some(path) => Socket::connect_to(path)?,
        None => Socket::connect()?,
    };
    socket.set_read_timeout(Some(Duration::from_secs(3)))?;
    socket.set_write_timeout(Some(Duration::from_secs(3)))?;
    Ok(socket)
}

fn niri_socket(socket: Option<&std::path::Path>) -> Result<PathBuf> {
    let path = socket
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::var_os("NIRI_SOCKET").map(PathBuf::from))
        .context("NIRI_SOCKET or --socket is required")?;
    path.canonicalize().context("resolve Niri socket")
}

fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--browser-supervisor") {
        let arguments = std::env::args_os().skip(2).collect::<Vec<_>>();
        anyhow::ensure!(
            arguments.len() == 4,
            "invalid internal browser supervisor invocation"
        );
        return browser_owned::supervise(
            PathBuf::from(&arguments[0]),
            PathBuf::from(&arguments[1]),
            arguments[2]
                .to_str()
                .context("supervisor parent PID")?
                .parse()?,
            arguments[3]
                .to_str()
                .context("supervisor parent generation")?
                .parse()?,
        );
    }
    let mut arguments = std::env::args().skip(1);
    let mut socket = None;
    let mut name = "assistant".to_string();
    let mut idle_timeout = Duration::from_secs(60);
    let mut is_service = false;
    let mut print_service_socket = false;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--socket" => {
                socket = Some(PathBuf::from(
                    arguments.next().context("--socket requires a path")?,
                ))
            }
            "--name" => name = arguments.next().context("--name requires a name")?,
            "--idle-timeout-seconds" => {
                let seconds: u64 = arguments
                    .next()
                    .context("--idle-timeout-seconds requires a value")?
                    .parse()?;
                if !(1..=86400).contains(&seconds) {
                    bail!("idle timeout must be between 1 and 86400 seconds");
                }
                idle_timeout = Duration::from_secs(seconds);
            }
            "--service" => is_service = true,
            "--print-service-socket" => print_service_socket = true,
            "--help" | "-h" => {
                eprintln!(
                    "Usage: dojo-agent [--service | --print-service-socket] [--socket PATH] [--name NAME] [--idle-timeout-seconds SECONDS]\nLocal stdio MCP adapter, or shared Unix socket service with --service. Starts directly on demand without systemd. Defaults to NIRI_SOCKET and 60 seconds idle timeout."
                );
                return Ok(());
            }
            _ => bail!("unknown argument {argument:?}"),
        }
    }
    validate_name(&name)?;
    if print_service_socket {
        println!(
            "{}",
            service::paths(&niri_socket(socket.as_deref())?)?
                .0
                .display()
        );
        return Ok(());
    }
    if is_service {
        return service::run(niri_socket(socket.as_deref())?);
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .thread_keep_alive(Duration::from_secs(1))
        .build()?
        .block_on(serve(socket, name, idle_timeout))
}

async fn serve(socket: Option<PathBuf>, name: String, idle_timeout: Duration) -> Result<()> {
    let server = DesktopServer::new(socket, name, idle_timeout);
    let cleanup = server.session.clone();
    let service = server.serve(rmcp::transport::stdio()).await?;
    let waiting = service.waiting();
    tokio::pin!(waiting);
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);
    loop {
        tokio::select! {
            result = &mut waiting => { result?; break; }
            result = &mut interrupt => { result?; break; }
        }
    }
    cleanup.lock().await.connection = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn browser_navigation_without_session_returns_known_zero_byte_refusal() {
        let server = DesktopServer::new(
            Some(PathBuf::from("/not-a-live-desktop.sock")),
            "test".into(),
            Duration::from_secs(5),
        );
        let response = server
            .desktop_browser_navigate(Parameters(BrowserNavigateRequest {
                capture_id: "missing".into(),
                url: "https://example.invalid".into(),
            }))
            .await;
        let content = serde_json::to_value(response).unwrap();
        let outcome: Value =
            serde_json::from_str(content["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(outcome["effect"], "refused");
        assert_eq!(outcome["accepted_bytes"], 0);
        assert_eq!(outcome["requires_fresh_observation"], true);
    }

    #[test]
    fn browser_navigation_arguments_cannot_supply_authority() {
        assert!(
            serde_json::from_value::<BrowserNavigateRequest>(
                json!({"capture_id":"c","url":"https://example.invalid"})
            )
            .is_ok()
        );
        for field in [
            "approval_id",
            "grant_id",
            "intent_id",
            "peer",
            "window_id",
            "endpoint",
        ] {
            let mut value = json!({"capture_id":"c","url":"https://example.invalid"});
            value[field] = json!("forged");
            assert!(serde_json::from_value::<BrowserNavigateRequest>(value).is_err());
        }
    }

    #[test]
    fn action_arguments_are_strict() {
        let schema = schemars::schema_for!(DesktopAction);
        assert_eq!(schema.get("type"), Some(&json!("object")));
        let valid: DesktopAction =
            serde_json::from_value(json!({"action":"key", "evdev":30, "state":"tap"})).unwrap();
        assert!(valid.validate().is_ok());
        assert!(
            serde_json::from_value::<DesktopAction>(json!({"action":"reset", "window_id":1}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<DesktopAction>(
                json!({"action":"key", "evdev":30, "state":"typo"})
            )
            .is_err()
        );
        let invalid: DesktopAction =
            serde_json::from_value(json!({"action":"key", "evdev":256, "state":"tap"})).unwrap();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn paused_action_retains_seat_and_updates_revision() {
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixListener;
        let path = std::env::temp_dir().join(format!("dojo-paused-{}.sock", std::process::id()));
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut stream = BufReader::new(stream);
            let replies = [
                json!({"Ok":{"AgentInputCreated":{"seat_name":"dojo-agent-paused","peer_id":1}}}),
                json!({"Ok":{"InputCoordination":{"revision":0,"human_owned_windows":[]}}}),
                json!({"Ok":{"AgentInputPaused":{"reason":"input ownership changed","coordination":{"revision":1,"human_owned_windows":[42]}}}}),
                json!({"Ok":"Handled"}),
            ];
            let mut requests = Vec::new();
            for reply in replies {
                let mut line = String::new();
                stream.read_line(&mut line).unwrap();
                requests.push(serde_json::from_str::<Value>(&line).unwrap());
                writeln!(stream.get_mut(), "{reply}").unwrap();
            }
            requests
        });
        let mut session = DesktopSession {
            socket: Some(path.clone()),
            name: "paused".into(),
            controller: None,
        };
        let paused = session
            .action(DesktopAction::Focus { window_id: 42 })
            .unwrap();
        assert_eq!(
            paused,
            json!({"seat_name":"dojo-agent-paused","handled":false,"paused":true,
            "reason":"input ownership changed","coordination":{"revision":1,"human_owned_windows":[42]}})
        );
        assert!(session.controller.is_some());
        assert_eq!(
            session
                .action(DesktopAction::Focus { window_id: 43 })
                .unwrap()["handled"],
            true
        );
        let requests = server.join().unwrap();
        assert_eq!(requests[2]["AgentInput"]["Coordinated"]["revision"], 0);
        assert_eq!(requests[3]["AgentInput"]["Coordinated"]["revision"], 1);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn idle_reset_and_invalid_action_do_not_connect() {
        let mut session = DesktopAdapter {
            socket: Some(PathBuf::from("/missing-dojo-agent-test.sock")),
            name: "test".into(),
            idle_timeout: Duration::from_secs(60),
            connection: None,
        };
        assert_eq!(
            session.action(DesktopAction::Reset {}).unwrap(),
            json!({"seat_name":null,"handled":true})
        );
        assert!(
            session
                .action(DesktopAction::Key {
                    evdev: 0,
                    state: KeyState::Press
                })
                .is_err()
        );
        assert!(session.connection.is_none());
    }
}

fn launch_application(application: &str, socket_path: Option<&std::path::Path>) -> Result<Value> {
    let connection = zbus::blocking::connection::Builder::session()?
        .method_timeout(Duration::from_secs(3))
        .build()?;
    let environment = dojo::application_launch::session_environment(&connection)?;
    let id = if application == "browser" {
        let output = std::process::Command::new("xdg-settings")
            .args(["get", "default-web-browser"])
            .env_clear()
            .envs(&environment)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "could not determine default browser"
        );
        String::from_utf8(output.stdout)?.trim().to_owned()
    } else {
        application.to_owned()
    };
    anyhow::ensure!(
        !id.is_empty() && !id.contains('/') && id.ends_with(".desktop"),
        "expected a desktop entry id or browser"
    );
    let data_home = environment
        .get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(environment.get("HOME").cloned().unwrap_or_default()).join(".local/share")
        });
    let mut roots = vec![data_home];
    roots.extend(std::env::split_paths(
        environment
            .get("XDG_DATA_DIRS")
            .map(String::as_str)
            .unwrap_or("/usr/local/share:/usr/share"),
    ));
    let path = roots
        .into_iter()
        .map(|root| root.join("applications").join(&id))
        .find(|path| path.is_file())
        .context("desktop entry not found")?;
    let mut socket = desktop_socket(socket_path)?;
    let windows = |socket: &mut Socket| -> Result<Vec<niri_ipc::Window>> {
        match socket.send(Request::Windows)?.map_err(anyhow::Error::msg)? {
            Response::Windows(windows) => Ok(windows),
            _ => bail!("unexpected Niri window response"),
        }
    };
    let entry = freedesktop_desktop_entry::DesktopEntry::from_path(&path, None::<&[String]>)?;
    let expected = entry
        .startup_wm_class()
        .unwrap_or(id.trim_end_matches(".desktop"))
        .to_lowercase();
    let before: Vec<_> = windows(&mut socket)?
        .into_iter()
        .map(|window| window.id)
        .collect();
    if application == "browser" {
        dojo::application_launch::launch_new_window(&path, &id)?;
    } else {
        dojo::application_launch::launch(&path, &id)?;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(window) = windows(&mut socket)?.into_iter().find(|window| {
            !before.contains(&window.id)
                && window
                    .app_id
                    .as_ref()
                    .is_some_and(|app| app.to_lowercase() == expected)
        }) {
            return Ok(json!({"application": id, "window": window, "verified": true}));
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "application launch did not create a new window"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
