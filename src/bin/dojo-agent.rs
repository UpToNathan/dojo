use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[path = "dojo-agent/worker.rs"]
mod worker;
use worker::ActiveWorker;

use anyhow::{Context, Result, bail};
use dojo::agent_input::{AgentController, InputState};
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
    worker: Option<ActiveWorker>,
}

impl DesktopAdapter {
    fn reap_worker(&mut self) -> Result<()> {
        if let Some(worker) = &mut self.worker {
            if worker.exited()? {
                self.worker = None;
            }
        }
        Ok(())
    }

    fn context(&mut self) -> Result<Value> {
        self.reap_worker()?;
        let mut socket = match &self.socket {
            Some(path) => Socket::connect_to(path),
            None => Socket::connect(),
        }
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
        Ok(json!({
            "coordination": coordination,
            "windows": windows,
            "outputs": outputs,
            "seat_name": self.worker.as_ref().and_then(|worker| worker.seat_name.as_deref()),
            "worker_pid": self.worker.as_ref().map(ActiveWorker::pid),
            "idle_timeout_seconds": self.idle_timeout.as_secs(),
            "coordinate_system": "logical pixels relative to the named output",
        }))
    }

    fn action(&mut self, action: DesktopAction) -> Result<Value> {
        action.validate()?;
        self.reap_worker()?;
        if matches!(action, DesktopAction::Reset {}) && self.worker.is_none() {
            return Ok(json!({"seat_name": null, "handled": true}));
        }
        if self.worker.is_none() {
            self.worker = Some(ActiveWorker::spawn(
                self.socket.as_deref(),
                &self.name,
                self.idle_timeout,
            )?);
        }
        let result = self
            .worker
            .as_mut()
            .context("desktop worker unavailable")?
            .action(&action);
        if result.is_err() {
            self.worker = None;
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
    fn new(socket: Option<PathBuf>, name: String, idle_timeout: Duration) -> Self {
        Self {
            session: Arc::new(Mutex::new(DesktopAdapter {
                socket,
                name,
                idle_timeout,
                worker: None,
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
            Ok(Ok(value)) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
            Ok(Err(error)) => CallToolResult::error(vec![ContentBlock::text(format!("{error:#}"))]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
                "desktop worker failed: {error}"
            ))]),
        }
    }

    #[tool(
        description = "List Niri windows, outputs and input coordination with human_owned_windows and revision. Does not start a worker or create an agent seat. Reports worker_pid and seat_name as null while idle. Window/output snapshots are separate queries and may change between calls."
    )]
    async fn desktop_context(&self) -> CallToolResult {
        self.execute(|session| session.context()).await
    }

    #[tool(
        description = "Act using your independent cursor and keyboard seat. Focus a window before keys, or move then click to select it. Coordinates are logical pixels relative to output. Keys use Linux evdev codes: A=30, Enter=28, Shift=42. Use press/release for chords and tap for a single key. Scroll uses pixel deltas. Reset releases held input and clears focus. Paused results retain the seat and report human ownership or stale state; inspect context and choose a new action without automatically retrying. Errors release the seat; select a fresh target afterward. Human Niri focus is unchanged. After the idle timeout the worker exits and loses its target; focus or click again before typing. Clipboard, Unicode text insertion and window management are unavailable."
    )]
    async fn desktop_action(
        &self,
        Parameters(action): Parameters<DesktopAction>,
    ) -> CallToolResult {
        self.execute(move |session| session.action(action)).await
    }

    #[tool(
        description = "Stop the desktop worker and release this agent's seat, cursor and held input. Safe to repeat. A later action creates a new seat."
    )]
    async fn desktop_release(&self) -> CallToolResult {
        self.execute(|session| {
            session.worker = None;
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
            .with_instructions("Desktop tools use an independent seat in the experimental Dojo Niri fork. Inspect desktop_context before choosing targets. Release the seat when finished. No screenshot or accessibility observation is supplied by this server.")
    }
}

fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let mut socket = None;
    let mut name = "assistant".to_string();
    let mut idle_timeout = Duration::from_secs(60);
    let mut is_worker = false;
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
            "--worker" => is_worker = true,
            "--help" | "-h" => {
                eprintln!(
                    "Usage: dojo-agent [--socket PATH] [--name NAME] [--idle-timeout-seconds SECONDS]\nLocal stdio MCP adapter for on-demand desktop input. Defaults to NIRI_SOCKET and 60 seconds idle timeout."
                );
                return Ok(());
            }
            _ => bail!("unknown argument {argument:?}"),
        }
    }
    if name.is_empty()
        || name.len() > 32
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("name must contain 1..32 ASCII letters, digits, hyphens or underscores");
    }
    if is_worker {
        return worker::run(socket, name, idle_timeout);
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .thread_keep_alive(Duration::from_secs(1))
        .build()?
        .block_on(serve(socket, name, idle_timeout))
}

async fn serve(socket: Option<PathBuf>, name: String, idle_timeout: Duration) -> Result<()> {
    let mut child_exits = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::child())?;
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
            _ = child_exits.recv() => { cleanup.lock().await.reap_worker()?; }
        }
    }
    cleanup.lock().await.worker = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
                json!({"Ok":{"AgentInputCreated":{"seat_name":"dojo-agent-paused"}}}),
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
            worker: None,
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
        assert!(session.worker.is_none());
    }
}
