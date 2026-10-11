//! Blocking controller for a real Niri agent seat. Keep this value alive while
//! the agent acts; closing its socket makes the compositor release held input.

use std::path::Path;

use anyhow::{Context, Result, bail};
use niri_ipc::{AgentInput, Request, Response, socket::Socket};

pub use niri_ipc::AgentInputState as InputState;

pub struct AgentController {
    socket: Socket,
    seat_name: String,
    revision: u64,
}

impl AgentController {
    pub fn connect(name: &str) -> Result<Self> {
        Self::create(Socket::connect().context("connect to Niri")?, name)
    }

    pub fn connect_to(path: impl AsRef<Path>, name: &str) -> Result<Self> {
        Self::create(Socket::connect_to(path).context("connect to Niri")?, name)
    }

    fn create(mut socket: Socket, name: &str) -> Result<Self> {
        let response = socket
            .send(Request::AgentInput(AgentInput::Create {
                name: name.into(),
            }))
            .context("create agent seat")?
            .map_err(anyhow::Error::msg)?;
        let Response::AgentInputCreated { seat_name } = response else {
            bail!("Niri did not acknowledge agent seat creation: {response:?}");
        };
        let Response::InputCoordination(coordination) = socket
            .send(Request::InputCoordination)?
            .map_err(anyhow::Error::msg)?
        else {
            bail!("Niri did not report input coordination");
        };
        Ok(Self {
            socket,
            seat_name,
            revision: coordination.revision,
        })
    }

    pub fn seat_name(&self) -> &str {
        &self.seat_name
    }

    pub fn focus_window(&mut self, window_id: u64) -> Result<()> {
        self.command(AgentInput::Focus { window_id })
    }

    pub fn move_pointer(&mut self, output: &str, x: f64, y: f64) -> Result<()> {
        let response = self.send(AgentInput::Move {
            output: output.into(),
            x,
            y,
        })?;
        match response {
            Response::AgentInputMoving => self.command(AgentInput::Wait),
            Response::Handled => Ok(()),
            response => bail!("unexpected agent movement response: {response:?}"),
        }
    }

    pub fn button(&mut self, button: u32, state: InputState) -> Result<()> {
        self.command(AgentInput::Button { button, state })
    }

    pub fn key(&mut self, evdev: u32, state: InputState) -> Result<()> {
        self.command(AgentInput::Key { evdev, state })
    }

    pub fn scroll(&mut self, horizontal: f64, vertical: f64) -> Result<()> {
        self.command(AgentInput::Axis {
            horizontal,
            vertical,
        })
    }

    pub fn reset(&mut self) -> Result<()> {
        self.command(AgentInput::Reset)
    }

    pub fn destroy(mut self) -> Result<()> {
        self.command(AgentInput::Destroy)
    }

    fn send(&mut self, command: AgentInput) -> Result<Response> {
        let command = if matches!(command, AgentInput::Reset | AgentInput::Destroy) {
            command
        } else {
            AgentInput::Coordinated {
                revision: self.revision,
                command: Box::new(command),
            }
        };
        let response = self
            .socket
            .send(Request::AgentInput(command))?
            .map_err(anyhow::Error::msg)?;
        if let Response::AgentInputPaused {
            reason,
            coordination,
        } = response
        {
            self.revision = coordination.revision;
            return Err(InputPaused {
                reason,
                coordination,
            }
            .into());
        }
        Ok(response)
    }

    fn command(&mut self, command: AgentInput) -> Result<()> {
        match self.send(command)? {
            Response::Handled => Ok(()),
            response => bail!("unexpected agent input response: {response:?}"),
        }
    }
}

#[derive(Debug)]
pub struct InputPaused {
    pub reason: String,
    pub coordination: niri_ipc::InputCoordination,
}
impl std::fmt::Display for InputPaused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "agent paused: {}", self.reason)
    }
}
impl std::error::Error for InputPaused {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::thread;

    #[test]
    fn controller_keeps_one_connection_until_drop() {
        let directory = std::env::temp_dir().join(format!("dojo-agent-sdk-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ipc.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut stream = BufReader::new(stream);
            let mut requests = Vec::new();
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let value: serde_json::Value = serde_json::from_str(&line).unwrap();
                let reply = if requests.is_empty() {
                    "{\"Ok\":{\"AgentInputCreated\":{\"seat_name\":\"dojo-agent-test\"}}}\n"
                } else if value == serde_json::json!("InputCoordination") {
                    "{\"Ok\":{\"InputCoordination\":{\"revision\":7,\"human_owned_windows\":[]}}}\n"
                } else if value
                    .get("AgentInput")
                    .and_then(|v| v.get("Coordinated"))
                    .and_then(|v| v.get("command"))
                    .and_then(|v| v.get("Move"))
                    .is_some()
                {
                    "{\"Ok\":\"AgentInputMoving\"}\n"
                } else {
                    "{\"Ok\":\"Handled\"}\n"
                };
                requests.push(value);
                stream.get_mut().write_all(reply.as_bytes()).unwrap();
            }
            requests
        });
        {
            let mut controller = AgentController::connect_to(&path, "test").unwrap();
            assert_eq!(controller.seat_name(), "dojo-agent-test");
            controller.focus_window(42).unwrap();
            controller.move_pointer("test-output", 100., 200.).unwrap();
            controller.key(30, InputState::Pressed).unwrap();
            controller.key(30, InputState::Released).unwrap();
            controller.reset().unwrap();
        }
        let requests = server.join().unwrap();
        assert_eq!(
            requests[0],
            serde_json::json!({"AgentInput":{"Create":{"name":"test"}}})
        );
        assert_eq!(requests[1], serde_json::json!("InputCoordination"));
        let commands = vec![
            serde_json::json!({"Focus":{"window_id":42}}),
            serde_json::json!({"Move":{"output":"test-output","x":100.0,"y":200.0}}),
            serde_json::json!("Wait"),
            serde_json::json!({"Key":{"evdev":30,"state":"Pressed"}}),
            serde_json::json!({"Key":{"evdev":30,"state":"Released"}}),
        ];
        for (request, command) in requests[2..7].iter().zip(commands) {
            assert_eq!(
                *request,
                serde_json::json!({"AgentInput":{"Coordinated":{"revision":7,"command":command}}})
            );
        }
        assert_eq!(requests[7], serde_json::json!({"AgentInput":"Reset"}));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
