use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use dojo::agent_input::{AgentController, InputState};
use niri_ipc::{Request, Response, socket::Socket};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
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
    fn context(&self) -> Result<Value> {
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
        Ok(json!({
            "windows": windows,
            "outputs": outputs,
            "seat_name": self.controller.as_ref().map(AgentController::seat_name),
            "coordinate_system": "logical pixels relative to the named output",
        }))
    }

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
            self.controller = None;
            return Err(error.context("input failed; agent seat released, select a fresh target"));
        }
        Ok(json!({"seat_name": controller.seat_name(), "handled": true}))
    }
}

#[derive(Clone)]
struct DesktopServer {
    session: Arc<Mutex<DesktopSession>>,
}

#[tool_router]
impl DesktopServer {
    fn new(socket: Option<PathBuf>, name: String) -> Self {
        Self {
            session: Arc::new(Mutex::new(DesktopSession {
                socket,
                name,
                controller: None,
            })),
        }
    }

    async fn execute(
        &self,
        operation: impl FnOnce(&mut DesktopSession) -> Result<Value> + Send + 'static,
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
        description = "List Niri windows and outputs with IDs, layout and logical dimensions. Does not create an agent seat. Window/output snapshots are separate queries and may change between calls."
    )]
    async fn desktop_context(&self) -> CallToolResult {
        self.execute(|session| session.context()).await
    }

    #[tool(
        description = "Act using your independent cursor and keyboard seat. Focus a window before keys, or move then click to select it. Coordinates are logical pixels relative to output. Keys use Linux evdev codes: A=30, Enter=28, Shift=42. Use press/release for chords and tap for a single key. Scroll uses pixel deltas. Reset releases held input and clears focus. Errors release the seat; select a fresh target afterward. Human Niri focus is unchanged. Clipboard, Unicode text insertion and window management are unavailable."
    )]
    async fn desktop_action(
        &self,
        Parameters(action): Parameters<DesktopAction>,
    ) -> CallToolResult {
        self.execute(move |session| session.action(action)).await
    }

    #[tool(
        description = "Release this agent's seat, cursor and held input. Safe to repeat. A later action creates a new seat."
    )]
    async fn desktop_release(&self) -> CallToolResult {
        self.execute(|session| {
            session.controller = None;
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

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let mut socket = None;
    let mut name = "assistant".to_string();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--socket" => {
                socket = Some(PathBuf::from(
                    arguments.next().context("--socket requires a path")?,
                ))
            }
            "--name" => name = arguments.next().context("--name requires a name")?,
            "--help" | "-h" => {
                eprintln!(
                    "Usage: dojo-agent [--socket PATH] [--name NAME]\nLocal stdio MCP server for independent desktop input. Defaults to NIRI_SOCKET."
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
    let server = DesktopServer::new(socket, name);
    let cleanup = server.session.clone();
    let service = server.serve(rmcp::transport::stdio()).await?;
    tokio::select! {
        result = service.waiting() => { result?; }
        result = tokio::signal::ctrl_c() => { result?; }
    }
    cleanup.lock().await.controller = None;
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
    fn idle_reset_and_invalid_action_do_not_connect() {
        let mut session = DesktopSession {
            socket: Some(PathBuf::from("/missing-dojo-agent-test.sock")),
            name: "test".into(),
            controller: None,
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
        assert!(session.controller.is_none());
    }
}
