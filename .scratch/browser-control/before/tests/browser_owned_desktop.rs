//! Live MCP acceptance against an isolated nested desktop. No model or Python calls.
#[path = "support/nested_workspace.rs"]
mod nested_workspace;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use niri_ipc::{Request, socket::Socket};
use serde_json::{Value, json};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Mcp {
    _process: Process,
    stdin: ChildStdin,
    replies: Receiver<Value>,
    next: u64,
}
impl Mcp {
    fn open(root: &Path, display: &str, socket: &Path, name: &str) -> Result<Self> {
        let mut process = Command::new(root.join("target/debug/dojo-agent"))
            .args(["--name", name, "--socket"])
            .arg(socket)
            .env("WAYLAND_DISPLAY", display)
            .env("NIRI_SOCKET", socket)
            .env("OPENAI_API_KEY", "DOJO_TEST_NOT_A_CREDENTIAL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = process.stdin.take().context("MCP stdin")?;
        let stdout = process.stdout.take().context("MCP stdout")?;
        let (tx, replies) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Ok(value) = serde_json::from_str(&line) {
                    if tx.send(value).is_err() {
                        break;
                    }
                }
            }
        });
        let mut mcp = Self {
            _process: Process(process),
            stdin,
            replies,
            next: 0,
        };
        mcp.call("initialize", json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"capture-test","version":"1"}}))?;
        writeln!(
            mcp.stdin,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )?;
        Ok(mcp)
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params})
        )?;
        loop {
            let response = self.replies.recv_timeout(Duration::from_secs(40))?;
            if response["id"] == self.next {
                ensure!(response.get("error").is_none(), "MCP error: {response}");
                return Ok(response["result"].clone());
            }
        }
    }
    fn tool(&mut self, name: &str, args: Value) -> Result<Value> {
        let result = self.call("tools/call", json!({"name":name,"arguments":args}))?;
        if result["isError"] == true {
            eprintln!(
                "fixture tool {name} refused: {}",
                result["content"]
                    .as_array()
                    .map(|blocks| blocks
                        .iter()
                        .filter_map(|block| block["text"].as_str())
                        .collect::<Vec<_>>()
                        .join(" "))
                    .unwrap_or_default()
            );
        }
        Ok(result)
    }
}
fn metadata(result: &Value) -> Result<Value> {
    let text = result["content"]
        .as_array()
        .context("MCP content")?
        .iter()
        .find_map(|block| block.get("text").and_then(Value::as_str))
        .context("MCP metadata")?;
    serde_json::from_str(text).with_context(|| format!("JSON metadata: {text}"))
}

fn process_identity(pid: u32) -> Option<(char, u64)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    Some((
        fields.first()?.chars().next()?,
        fields.get(19)?.parse().ok()?,
    ))
}

#[test]
#[ignore = "requires Wayland and built Niri/dojo-agent; launches isolated browser in nested desktop"]
fn owned_browser_create_observe_and_close() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = tempfile::Builder::new()
        .prefix("dojo-owned-browser-live.")
        .tempdir()?
        .keep();
    println!("Owned browser evidence: {}", evidence.display());
    let log = evidence.join("niri.log");
    let binary = std::env::var_os("NIRI_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("vendor/niri/target/debug/niri"));
    let mut compositor = Process(
        Command::new(binary)
            .arg("--config")
            .arg(root.join("tools/agent-input/nested.kdl"))
            .env("DOJO_AGENT_INPUT", "1")
            .env("RUST_LOG", "niri=info")
            .stdout(fs::File::create(&log)?)
            .stderr(fs::File::options().append(true).open(&log)?)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    let (display, socket) = loop {
        ensure!(compositor.0.try_wait()?.is_none(), "nested desktop exited");
        let text = fs::read_to_string(&log)?;
        let find = |marker: &str| {
            text.lines()
                .filter_map(|line| {
                    line.split_once(marker)
                        .map(|(_, tail)| tail.trim().to_string())
                })
                .last()
        };
        if let Some(pair) = find("listening on Wayland socket: ").zip(find("IPC listening on: ")) {
            break (pair.0, PathBuf::from(pair.1));
        }
        ensure!(Instant::now() < deadline, "nested startup timed out");
        std::thread::sleep(Duration::from_millis(50));
    };
    nested_workspace::isolate(compositor.0.id())?;
    let mut client = Mcp::open(&root, &display, &socket, "owned-browser-live")?;
    let created = client.tool("desktop_browser_create", json!({}))?;
    fs::write(
        evidence.join("create.json"),
        serde_json::to_vec_pretty(&created)?,
    )?;
    ensure!(
        created["isError"] != true,
        "browser creation failed: {created}"
    );
    let data = metadata(&created)?;
    ensure!(data["created"] == true, "browser was not created");
    if std::env::var_os("DOJO_TEST_BROWSER_CURSOR").is_some() {
        let mut niri = Socket::connect_to(&socket)?;
        let niri_ipc::Response::Outputs(outputs) =
            niri.send(Request::Outputs)?.map_err(anyhow::Error::msg)?
        else {
            bail!("outputs response");
        };
        let output = outputs.keys().next().context("nested output")?;
        niri.send(Request::Output {
            output: output.clone(),
            action: niri_ipc::OutputAction::Scale {
                scale: niri_ipc::ScaleToSet::Specific(1.75),
            },
        })?
        .map_err(anyhow::Error::msg)?;
        std::thread::sleep(Duration::from_secs(1));
        niri.send(Request::YieldHumanInput)?
            .map_err(anyhow::Error::msg)?;
        for input in [
            json!({"kind":"move"}),
            json!({"kind":"chord","keys":["Control","t"]}),
        ] {
            let observation = client.tool(
                "desktop_observe",
                json!({"output":output,"window_id":data["window_id"]}),
            )?;
            let capture = metadata(&observation)?;
            let result = client.tool("desktop_capture_action", json!({"capture_id":capture["capture_id"],"x":164,"y":17,"input":input,"observe_after":true}))?;
            let receipt = metadata(&result)?;
            fs::write(
                evidence.join(format!("cursor-{}.json", input["kind"].as_str().unwrap())),
                serde_json::to_vec_pretty(&json!({"capture":capture,"receipt":receipt}))?,
            )?;
            ensure!(
                receipt["handled"] == true,
                "browser cursor action refused: {receipt}"
            );
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let niri_ipc::Response::Windows(windows) =
                niri.send(Request::Windows)?.map_err(anyhow::Error::msg)?
            else {
                bail!("windows response");
            };
            let window = windows
                .iter()
                .find(|window| Some(window.id) == data["window_id"].as_u64())
                .context("browser target disappeared")?;
            if window
                .title
                .as_deref()
                .is_some_and(|title| title.contains("New Tab"))
            {
                fs::write(
                    evidence.join("new-tab-window.json"),
                    serde_json::to_vec_pretty(window)?,
                )?;
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "Ctrl+T did not open a browser tab: {:?}",
                window.title
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        client.tool("desktop_browser_close", json!({}))?;
        return Ok(());
    }
    ensure!(
        created["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["type"] == "image"),
        "missing actual browser image"
    );
    let first_ref = data["post_observation"]["elements"]
        .as_array()
        .context("initial refs")?
        .iter()
        .find(|element| {
            element["label"]
                .as_str()
                .is_some_and(|label| label.contains("Open blank page"))
        })
        .context("real starter-page link ref missing")?["ref"]
        .clone();
    ensure!(
        client.tool(
            "desktop_browser_inspect_ref",
            json!({"reference":first_ref})
        )?["isError"]
            != true,
        "current ref refused"
    );
    let mut other = Mcp::open(&root, &display, &socket, "owned-browser-other")?;
    ensure!(
        other.tool(
            "desktop_browser_inspect_ref",
            json!({"reference":first_ref})
        )?["isError"]
            == true,
        "foreign ref accepted"
    );
    ensure!(
        client.tool(
            "desktop_browser_observe",
            json!({"window_id":data["window_id"].as_u64().unwrap()+1000})
        )?["isError"]
            == true,
        "wrong native window accepted"
    );
    let observed = client.tool(
        "desktop_browser_observe",
        json!({"window_id":data["window_id"]}),
    )?;
    fs::write(
        evidence.join("observe.json"),
        serde_json::to_vec_pretty(&observed)?,
    )?;
    ensure!(
        observed["isError"] != true,
        "browser read failed: {observed}"
    );
    ensure!(
        client.tool(
            "desktop_browser_inspect_ref",
            json!({"reference":first_ref})
        )?["isError"]
            == true,
        "ref survived reobservation"
    );
    let mut niri = Socket::connect_to(&socket)?;
    let niri_ipc::Response::Windows(windows) =
        niri.send(Request::Windows)?.map_err(anyhow::Error::msg)?
    else {
        bail!("windows response");
    };
    let native = windows
        .iter()
        .find(|window| Some(window.id) == data["window_id"].as_u64())
        .context("owned native window missing")?;
    ensure!(
        native
            .title
            .as_deref()
            .is_some_and(|title| title.starts_with("Dojo Browser")),
        "unexpected starter-page title"
    );
    let pid = native.pid.context("native browser PID missing")?;
    let observed_data = metadata(&observed)?;
    let profile = observed_data["url"]
        .as_str()
        .and_then(|url| url.strip_prefix("file://"))
        .and_then(|path| path.strip_suffix("/start.html"))
        .context("exact generated starter URL missing")?
        .to_string();
    ensure!(
        Path::new(&profile)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("dojo-browser-")),
        "unexpected owned profile source"
    );
    use std::os::unix::fs::PermissionsExt;
    ensure!(
        fs::metadata(&profile)?.permissions().mode() & 0o777 == 0o700,
        "profile is not private"
    );
    let environment = fs::read(format!("/proc/{pid}/environ"))?;
    ensure!(
        !environment
            .split(|byte| *byte == 0)
            .any(|entry| entry.starts_with(b"OPENAI_API_KEY=")),
        "provider credential inherited by browser"
    );
    let owned_processes = fs::read_dir("/proc")?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            fs::read(format!("/proc/{pid}/cmdline"))
                .ok()
                .is_some_and(|args| {
                    args.windows(profile.len())
                        .any(|window| window == profile.as_bytes())
                })
        })
        .filter_map(|pid| process_identity(pid).map(|(_, generation)| (pid, generation)))
        .collect::<Vec<_>>();
    ensure!(
        owned_processes.len() >= 3,
        "supervised browser process evidence missing"
    );
    fs::write(
        evidence.join("owned-processes.json"),
        serde_json::to_vec_pretty(&owned_processes)?,
    )?;
    let verified=client.tool("desktop_verify",json!({"pid":pid,"window_id":native.id,"timeout_ms":5000,"stable_samples":1,"expect":[{"window":{"url_equals":metadata(&observed)?["url"]}},{"window":{"title_equals":native.title}},{"element":{"selector":{"label_contains":"Open blank page"},"exists":true}}]}))?;
    fs::write(
        evidence.join("verify.json"),
        serde_json::to_vec_pretty(&verified)?,
    )?;
    ensure!(
        metadata(&verified)?["status"] == "satisfied",
        "browser positive verification failed: {verified}"
    );
    let negative=client.tool("desktop_verify",json!({"pid":pid,"window_id":native.id,"timeout_ms":500,"stable_samples":1,"expect":[{"element":{"selector":{"label_contains":"absent label"},"exists":false}}]}))?;
    ensure!(
        metadata(&negative)?["status"] == "unknown",
        "filtered DOM absence was fabricated: {negative}"
    );
    let closed = client.tool("desktop_browser_close", json!({}))?;
    ensure!(
        metadata(&closed)?["closed"] == true,
        "owned browser did not close"
    );
    for (process, generation) in owned_processes {
        let alive = process_identity(process)
            .is_some_and(|(state, current)| state != 'Z' && current == generation);
        ensure!(!alive, "owned browser descendant {process} survived close");
    }
    ensure!(
        !Path::new(&profile).exists(),
        "isolated profile survived confirmed close"
    );
    ensure!(
        !Path::new(&format!("/proc/{pid}/exe")).exists(),
        "owned browser process survived close"
    );
    ensure!(
        client.tool(
            "desktop_browser_inspect_ref",
            json!({"reference":first_ref})
        )?["isError"]
            == true,
        "ref survived close"
    );
    let mut niri = Socket::connect_to(&socket)?;
    let niri_ipc::Response::Windows(windows) =
        niri.send(Request::Windows)?.map_err(anyhow::Error::msg)?
    else {
        bail!("windows response");
    };
    ensure!(
        !windows
            .iter()
            .any(|window| serde_json::to_value(window.id).ok() == Some(data["window_id"].clone())),
        "owned window survived close"
    );
    let idle = metadata(&client.tool("desktop_browser_create", json!({}))?)?;
    fs::write(
        evidence.join("idle-create.json"),
        serde_json::to_vec_pretty(&idle)?,
    )?;
    let idle_profile = idle["post_observation"]["url"]
        .as_str()
        .and_then(|url| url.strip_prefix("file://"))
        .and_then(|path| path.strip_suffix("/start.html"))
        .context("idle browser profile missing")?
        .to_owned();
    ensure!(
        Path::new(&idle_profile).exists(),
        "idle profile was absent before expiry test"
    );
    // Reset retires semantic observations without renewing the
    // owned browser. No service request is made while its expiry timer must fire.
    let reset = client.tool("desktop_action", json!({"action":"reset"}))?;
    ensure!(
        reset["isError"] != true && metadata(&reset)?["handled"] == true,
        "idle native reset refused"
    );
    ensure!(
        Path::new(&idle_profile).exists(),
        "reset retired the owned browser before its expiry"
    );
    let deadline = Instant::now() + Duration::from_secs(dojo_computer::CAPTURE_TTL_SECONDS + 12);
    while Path::new(&idle_profile).exists() {
        ensure!(
            Instant::now() < deadline,
            "owned browser expiry required another service request"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    fs::write(
        evidence.join("idle-expiry.json"),
        serde_json::to_vec_pretty(&json!({
            "profile_removed_without_request":true,"owned_browser_expiry_seconds":dojo_computer::CAPTURE_TTL_SECONDS
        }))?,
    )?;
    Ok(())
}
