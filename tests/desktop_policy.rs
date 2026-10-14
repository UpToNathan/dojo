//! Real MCP/service dispatch refusal. No compositor or desktop effects are needed.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn configured_policy_denies_effect_before_compositor_transport() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("niri.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let policy = directory.path().join("policy.yaml");
    std::fs::write(
        &policy,
        "allow: [desktop_observe]\ndeny: [desktop_action]\n",
    )
    .unwrap();
    let mut child = Process(
        Command::new(env!("CARGO_BIN_EXE_dojo-agent"))
            .args(["--name", "policy-fixture", "--socket"])
            .arg(&socket)
            .env("DOJO_DESKTOP_POLICY", &policy)
            .env("XDG_RUNTIME_DIR", directory.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut input = child.0.stdin.take().unwrap();
    let output = child.0.stdout.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            let Ok(line) = line else { break };
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if sender.send(value).is_err() {
                    break;
                }
            }
        }
    });
    let mut call = |id, method: &str, params: Value| {
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        )
        .unwrap();
        input.flush().unwrap();
        loop {
            let reply = receiver.recv_timeout(Duration::from_secs(15)).unwrap();
            if reply["id"] == id {
                assert!(reply.get("error").is_none(), "{reply}");
                if method == "initialize" {
                    writeln!(
                        input,
                        "{}",
                        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
                    )
                    .unwrap();
                    input.flush().unwrap();
                }
                break reply["result"].clone();
            }
        }
    };
    call(
        1,
        "initialize",
        json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}),
    );
    let denied = call(
        2,
        "tools/call",
        json!({"name":"desktop_action","arguments":{"action":"focus","window_id":1}}),
    );
    assert_eq!(denied["isError"], true, "{denied}");
    assert!(
        denied["content"]
            .to_string()
            .contains("operator desktop policy denied dispatch"),
        "{denied}"
    );
    let catalog = call(3, "tools/list", json!({}));
    assert!(catalog["tools"].as_array().unwrap().iter().all(|tool| {
        !tool["name"]
            .as_str()
            .unwrap()
            .starts_with("desktop_browser_")
    }));
    let released = call(
        4,
        "tools/call",
        json!({"name":"desktop_release","arguments":{}}),
    );
    assert_ne!(released["isError"], true, "{released}");
    let launch = call(
        5,
        "tools/call",
        json!({"name":"desktop_launch","arguments":{"application":"dojo-policy-fixture-missing"}}),
    );
    assert_eq!(launch["isError"], true, "{launch}");
    assert!(
        launch["content"]
            .to_string()
            .contains("operator desktop policy denied dispatch"),
        "{launch}"
    );
    let screenshot = call(
        6,
        "tools/call",
        json!({"name":"desktop_screenshot","arguments":{"output":"fixture"}}),
    );
    assert_eq!(screenshot["isError"], true, "{screenshot}");
    assert!(
        screenshot["content"]
            .to_string()
            .contains("operator desktop policy denied dispatch"),
        "{screenshot}"
    );
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
