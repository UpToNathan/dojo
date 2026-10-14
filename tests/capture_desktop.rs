//! Live MCP acceptance against an isolated nested desktop. Optional pinned local
//! Rust perception inference; no Python or provider model calls.
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
#[test]
fn fractional_capture_geometry_survives_json_transport() {
    let rect = [80.0_f64, 40.0, 761.1428571428571, 961.1428571428572];
    for width in [
        rect[2],
        f64::from_bits(rect[2].to_bits() + 1),
        f64::from_bits(rect[2].to_bits() - 1),
        465.14285714285717,
    ] {
        let rect = [rect[0], rect[1], width, rect[3]];
        let encoded = serde_json::to_string(&rect).unwrap();
        let decoded: [f64; 4] = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, rect, "capture geometry changed in IPC transport");
    }
}
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
        let initialized = mcp.call("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"capture-test","version":"1"}}))?;
        ensure!(
            initialized["protocolVersion"] == "2025-06-18",
            "pinned MCP protocol negotiation failed"
        );
        writeln!(
            mcp.stdin,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )?;
        Ok(mcp)
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.begin_call(method, params)?;
        self.finish_call(id, Duration::from_secs(45))
    }
    fn begin_call(&mut self, method: &str, params: Value) -> Result<u64> {
        self.next += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":self.next,"method":method,"params":params})
        )?;
        Ok(self.next)
    }
    fn finish_call(&mut self, id: u64, timeout: Duration) -> Result<Value> {
        loop {
            let response = self.replies.recv_timeout(timeout)?;
            if response["id"] == id {
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
#[track_caller]
fn metadata(result: &Value) -> Result<Value> {
    let text = result["content"]
        .as_array()
        .context("MCP content")?
        .iter()
        .find_map(|block| block.get("text").and_then(Value::as_str))
        .context("MCP metadata")?;
    let caller = std::panic::Location::caller();
    serde_json::from_str(text).with_context(|| {
        format!(
            "JSON metadata at {}:{}: {text}",
            caller.file(),
            caller.line()
        )
    })
}
fn request(socket: &Path, value: Value) -> Result<Value> {
    let mut connection = Socket::connect_to(socket)?;
    connection.set_read_timeout(Some(Duration::from_secs(3)))?;
    let request: Request = serde_json::from_value(value)?;
    let response = connection.send(request)?.map_err(anyhow::Error::msg)?;
    Ok(serde_json::to_value(response)?)
}
fn wait<T>(description: &str, mut check: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = check()? {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            bail!("timed out: {description}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn events(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
fn observe(client: &mut Mcp, output: &str, id: u64) -> Result<Value> {
    let result = wait("stable fresh target observation", || {
        let result = client.tool(
            "desktop_observe",
            json!({"output":output,"window_id":id,"scale":0.5}),
        )?;
        if result["isError"] == true {
            let reason = result["content"].to_string();
            if reason.contains("capture identity, geometry or ownership changed")
                || reason.contains("scene_animation_unsupported")
            {
                return Ok(None);
            }
            bail!("observation refused: {result}");
        }
        Ok(Some(result))
    })?;
    ensure!(
        result["content"]
            .as_array()
            .is_some_and(|c| c.iter().any(|b| b["type"] == "image")),
        "observation missing image"
    );
    metadata(&result)
}
fn click(observation: &Value) -> Value {
    json!({"capture_id":observation["capture_id"],"x":observation["image_width"].as_f64().unwrap() * 0.75,
        "y":observation["image_height"].as_f64().unwrap() * 0.75,"input":{"kind":"click","button":"left"}})
}

#[test]
#[ignore = "requires Wayland and built fork/recorder/dojo-agent; creates a nested desktop"]
fn capture_bound_input_reaches_only_its_target_and_cannot_replay() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let evidence = tempfile::Builder::new()
        .prefix("dojo-capture-live.")
        .tempdir()?
        .keep();
    println!("Capture acceptance evidence: {}", evidence.display());
    let niri_log = evidence.join("niri.log");
    let binary = std::env::var_os("NIRI_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("vendor/niri/target/debug/niri"));
    let mut compositor = Process(
        Command::new(binary)
            .arg("--config")
            .arg(root.join("tools/agent-input/nested.kdl"))
            .env("DOJO_AGENT_INPUT", "1")
            .env("RUST_LOG", "niri=info")
            .stdout(fs::File::create(&niri_log)?)
            .stderr(fs::File::options().append(true).open(&niri_log)?)
            .spawn()?,
    );
    let (display, socket) = wait("nested compositor socket", || {
        ensure!(
            compositor.0.try_wait()?.is_none(),
            "nested compositor exited; inspect {}",
            niri_log.display()
        );
        let log = fs::read_to_string(&niri_log)?;
        let find = |marker: &str| {
            log.lines()
                .filter_map(|line| {
                    line.split_once(marker)
                        .map(|(_, text)| text.trim().to_owned())
                })
                .last()
        };
        Ok(find("listening on Wayland socket: ")
            .zip(find("IPC listening on: "))
            .map(|(d, s)| (d, PathBuf::from(s))))
    })?;
    nested_workspace::isolate(compositor.0.id())?;
    let event_log = evidence.join("events.jsonl");
    let _recorder = Process(
        Command::new(root.join("tools/agent-input/target/debug/dojo-agent-input-fixture"))
            .env("WAYLAND_DISPLAY", &display)
            .env("NIRI_SOCKET", &socket)
            .stdout(fs::File::create(&event_log)?)
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let windows = wait("fixture windows", || {
        let response = request(&socket, json!("Windows"))?;
        let mut windows: Vec<_> = response["Windows"]
            .as_array()
            .context("windows")?
            .iter()
            .filter(|w| w["app_id"] == "dojo-agent-input-fixture")
            .cloned()
            .collect();
        windows.sort_by_key(|w| w["title"].as_str().unwrap_or("").to_owned());
        Ok((windows.len() == 2).then_some(windows))
    })?;
    let human = windows[0]["id"].as_u64().unwrap();
    let target = windows[1]["id"].as_u64().unwrap();
    for (i, window) in windows.iter().enumerate() {
        let id = window["id"].as_u64().unwrap();
        request(
            &socket,
            json!({"Action":{"MoveWindowToFloating":{"id":id}}}),
        )?;
        request(
            &socket,
            json!({"Action":{"MoveFloatingWindow":{"id":id,"x":{"SetFixed":20. + i as f64 * 310.},"y":{"SetFixed":60.}}}}),
        )?;
    }
    request(&socket, json!({"Action":{"FocusWindow":{"id":human}}}))?;
    std::thread::sleep(Duration::from_millis(600));
    let workspaces = request(&socket, json!("Workspaces"))?;
    let output = workspaces["Workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == windows[1]["workspace_id"])
        .context("fixture workspace")?["output"]
        .as_str()
        .context("output")?;
    let outputs = request(&socket, json!("Outputs"))?;
    let dimensions = &outputs["Outputs"][output]["logical"];
    let inject = |x: u32, y: u32| -> Result<()> {
        let mut pointer = Process(
            Command::new(root.join("tools/agent-input/target/debug/human-pointer"))
                .args([
                    x.to_string(),
                    y.to_string(),
                    dimensions["width"].to_string(),
                    dimensions["height"].to_string(),
                ])
                .env("WAYLAND_DISPLAY", &display)
                .spawn()?,
        );
        wait("primary pointer injection", || {
            match pointer.0.try_wait()? {
                Some(status) => {
                    ensure!(status.success(), "pointer fixture failed");
                    Ok(Some(()))
                }
                None => Ok(None),
            }
        })
    };
    let mut client = Mcp::open(&root, &display, &socket, "capture-live")?;
    let mut other = Mcp::open(&root, &display, &socket, "capture-other")?;
    if std::env::var_os("DOJO_TEST_SPEECH_BLUR").is_some() {
        let runtime = evidence.join("shell-runtime");
        fs::create_dir(&runtime)?;
        let protocol_log = evidence.join("shell-wayland.log");
        let _shell = Process(
            Command::new(
                std::env::var_os("DOJO_BINARY")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| root.join("target/debug/dojo")),
            )
            .arg("--enable")
            .env(
                "WAYLAND_DISPLAY",
                Path::new(&std::env::var("XDG_RUNTIME_DIR")?).join(&display),
            )
            .env("NIRI_SOCKET", &socket)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("XDG_STATE_HOME", evidence.join("shell-state"))
            .env("WAYLAND_DEBUG", "client")
            .stdout(Stdio::null())
            .stderr(fs::File::create(&protocol_log)?)
            .spawn()?,
        );
        let control = runtime.join("dojo.sock");
        wait("isolated shell control socket", || {
            Ok(control.exists().then_some(()))
        })?;
        for (event, value) in [
            ("transcript", "Test user bubble"),
            ("response", "Test agent bubble"),
            ("phase", "working"),
        ] {
            let mut connection = std::os::unix::net::UnixStream::connect(&control)?;
            writeln!(connection, "{}", json!({"event":event,"value":value}))?;
        }
        std::thread::sleep(Duration::from_secs(3));
        let scene = request(&socket, json!({"CaptureScene":{"output":output}}))?;
        use base64::Engine;
        fs::write(
            evidence.join("speech-bubbles.png"),
            base64::engine::general_purpose::STANDARD.decode(
                scene["CapturedScene"]["png_base64"]
                    .as_str()
                    .context("speech scene PNG")?,
            )?,
        )?;
        let log = fs::read_to_string(&protocol_log)?;
        let clears = log
            .lines()
            .filter(|line| line.contains("set_blur_region(nil)"))
            .count();
        let updates = log
            .lines()
            .filter(|line| line.contains("set_blur_region(") && !line.contains("(nil)"))
            .count();
        println!("Speech blur updates: {updates}; clears: {clears}");
        ensure!(updates > 0, "speech bubbles never submitted a blur region");
        ensure!(
            clears <= 1,
            "speech redraw repeatedly clears blur: {clears}"
        );
        return Ok(());
    }
    if std::env::var_os("DOJO_TEST_TILED_CHORD").is_some() {
        request(
            &socket,
            json!({"Output":{"output":output,"action":{"Scale":{"scale":{"Specific":1.75}}}}}),
        )?;
        request(
            &socket,
            json!({"Action":{"MoveWindowToTiling":{"id":target}}}),
        )?;
        request(
            &socket,
            json!({"Action":{"SetWindowWidth":{"id":target,"change":{"SetProportion":63.8}}}}),
        )?;
        std::thread::sleep(Duration::from_millis(1000));
        request(&socket, json!("YieldHumanInput"))?;
        for attempt in 0..10 {
            let observation = observe(&mut client, output, target)?;
            let before = events(&event_log).len();
            let result = client.tool(
                "desktop_capture_action",
                json!({
                    "capture_id": observation["capture_id"],
                    "x":observation["image_width"].as_f64().unwrap() * 0.5, "y":17,
                    "input":{"kind":"chord","keys":["Control","t"]},
                    "observe_after":true
                }),
            )?;
            let receipt = metadata(&result)?;
            fs::write(
                evidence.join(format!("tiled-chord-{attempt}.json")),
                serde_json::to_vec_pretty(&json!({"observation":observation,"receipt":receipt}))?,
            )?;
            ensure!(receipt["handled"] == true, "tiled chord refused: {receipt}");
            wait("tiled Ctrl+T delivery", || {
                let keys: Vec<_> = events(&event_log)
                    .into_iter()
                    .skip(before)
                    .filter(|event| event["event"] == "key" && event["window"] == 2)
                    .collect();
                Ok(([29, 20].into_iter().all(|key| {
                    keys.iter().any(|event| {
                        event["key"] == key
                            && event["state"]
                                .as_str()
                                .is_some_and(|state| state.contains("Pressed"))
                    })
                }))
                .then_some(()))
            })?;
        }
        return Ok(());
    }
    if std::env::var_os("DOJO_TEST_CUA_DRIVER").is_some() {
        fs::write(
            evidence.join("driver-placements.json"),
            serde_json::to_vec_pretty(&request(
                &socket,
                json!({"WindowPlacements":{"output":output}}),
            )?)?,
        )?;
        let catalog = metadata(&client.tool("desktop_driver_tools", json!({}))?)?;
        fs::write(
            evidence.join("driver-catalog.json"),
            serde_json::to_vec_pretty(&catalog)?,
        )?;
        let tools = catalog["tools"]
            .as_array()
            .context("original Driver tools")?;
        ensure!(
            tools.iter().any(|tool| tool["name"] == "click"),
            "original click absent"
        );
        for name in [
            "run_script",
            "run_steps",
            "start_session",
            "install_ffmpeg",
            "browser_navigate",
        ] {
            ensure!(
                !tools.iter().any(|tool| tool["name"] == name),
                "unadmitted Driver tool exposed: {name}"
            );
            let refused =
                client.tool("desktop_driver_call", json!({"name":name,"arguments":{}}))?;
            ensure!(
                refused["isError"] == true,
                "unadmitted Driver call accepted: {name}"
            );
        }

        for (name, extra, event) in [
            ("click", json!({"x":100,"y":100}), "button"),
            ("press_key", json!({"key":"enter"}), "key"),
            ("type_text", json!({"text":"Dojo"}), "key"),
            (
                "scroll",
                json!({"x":100,"y":100,"direction":"down","amount":1}),
                "axis",
            ),
        ] {
            let capture = client.tool("desktop_driver_call", json!({"name":"get_window_state","arguments":{"pid":windows[1]["pid"],"window_id":target,"include_screenshot":true}}))?;
            fs::write(
                evidence.join(format!("driver-{name}-capture.json")),
                serde_json::to_vec_pretty(&capture)?,
            )?;
            ensure!(
                capture["isError"] != true,
                "Driver capture refused: {capture}"
            );
            let state = match capture.get("structuredContent") {
                Some(value) => value.clone(),
                None => metadata(&capture)?,
            };
            let id = state["capture_id"]
                .as_str()
                .context("original immutable capture id")?;
            let mut args = extra;
            args["pid"] = windows[1]["pid"].clone();
            args["window_id"] = json!(target);
            args["capture_id"] = json!(id);
            let before = events(&event_log).len();
            let result =
                client.tool("desktop_driver_call", json!({"name":name,"arguments":args}))?;
            fs::write(
                evidence.join(format!("driver-{name}-result.json")),
                serde_json::to_vec_pretty(&result)?,
            )?;
            ensure!(result["isError"] != true, "Driver action refused: {result}");
            wait("Driver input reached target", || {
                Ok(events(&event_log)
                    .iter()
                    .skip(before)
                    .any(|item| item["event"] == event && item["window"] == 2)
                    .then_some(()))
            })?;
            if name == "type_text" {
                wait("complete Driver text delivery", || {
                    Ok((events(&event_log)
                        .iter()
                        .skip(before)
                        .filter(|item| item["event"] == "key" && item["window"] == 2)
                        .count()
                        >= 10)
                        .then_some(()))
                })?;
                let delivered: Vec<_> = events(&event_log)
                    .into_iter()
                    .skip(before)
                    .filter(|item| item["event"] == "key" && item["window"] == 2)
                    .map(|item| {
                        (
                            item["key"].as_u64().unwrap(),
                            item["state"].as_str().unwrap().to_owned(),
                        )
                    })
                    .collect();
                let expected: Vec<_> = [
                    (42, "Value(Pressed)"),
                    (32, "Value(Pressed)"),
                    (32, "Value(Released)"),
                    (42, "Value(Released)"),
                    (24, "Value(Pressed)"),
                    (24, "Value(Released)"),
                    (36, "Value(Pressed)"),
                    (36, "Value(Released)"),
                    (24, "Value(Pressed)"),
                    (24, "Value(Released)"),
                ]
                .into_iter()
                .map(|(key, state)| (key, state.to_owned()))
                .collect();
                ensure!(
                    delivered == expected,
                    "Driver text transitions differ: {delivered:?}"
                );
            }
            let reused =
                client.tool("desktop_driver_call", json!({"name":name,"arguments":args}))?;
            ensure!(reused["isError"] == true, "Driver reused capture");
        }
        let context = metadata(&client.tool("desktop_context", json!({}))?)?;
        let worker = context["driver_worker_pid"]
            .as_u64()
            .context("activated Driver worker PID")?;
        fs::write(
            evidence.join("driver-context.json"),
            serde_json::to_vec_pretty(&context)?,
        )?;
        drop(client);
        wait("disconnected Driver worker reaped", || {
            Ok((!Path::new(&format!("/proc/{worker}")).exists()).then_some(()))
        })?;
        other.tool("desktop_driver_tools", json!({}))?;
        let context = metadata(&other.tool("desktop_context", json!({}))?)?;
        let other_worker = context["driver_worker_pid"]
            .as_u64()
            .context("second Driver worker PID")?;
        ensure!(
            other_worker != worker,
            "Driver worker shared across client connections"
        );
        other.tool("desktop_release", json!({}))?;
        wait("released Driver worker reaped", || {
            Ok((!Path::new(&format!("/proc/{other_worker}")).exists()).then_some(()))
        })?;
        println!(
            "Original Driver handlers passed through activated Dojo service: {}",
            evidence.display()
        );
        return Ok(());
    }
    let catalog = client.call("tools/list", json!({}))?;
    let pinned: Value = serde_json::from_str(include_str!(
        "../provenance/contracts/cua-driver.manifest.json"
    ))?;
    for name in [
        "list_windows",
        "get_screen_size",
        "list_apps",
        "parse_visual_regions",
    ] {
        let exposed = catalog["tools"]
            .as_array()
            .context("tool catalog")?
            .iter()
            .find(|tool| tool["name"] == name)
            .context("missing discovery tool")?;
        let expected = pinned["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        let mut schema = exposed["inputSchema"].clone();
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            object.remove("title");
        }
        ensure!(
            schema == expected["input_schema"],
            "discovery input schema differs from pinned contract: {name}: {schema}"
        );
        ensure!(
            exposed["annotations"]
                == json!({"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}),
            "discovery annotations differ: {name}"
        );
    }
    fs::write(
        evidence.join("discovery-catalog.json"),
        serde_json::to_vec_pretty(&catalog)?,
    )?;
    let listed = metadata(&client.tool(
        "list_windows",
        json!({"pid":windows[1]["pid"],"on_screen_only":true}),
    )?)?;
    ensure!(
        listed["windows"].as_array().is_some_and(|items| items
            .iter()
            .any(|window| window["window_id"] == target && window["z_index"].is_null())),
        "native window discovery: {listed}"
    );
    ensure!(
        listed["action_authority"] == false,
        "window listing became an input grant"
    );
    let screen = metadata(&client.tool("get_screen_size", json!({}))?)?;
    ensure!(
        screen["width"].as_u64().is_some_and(|value| value > 0)
            && screen["scale_factor"]
                .as_f64()
                .is_some_and(|value| value > 0.0),
        "native screen dimensions: {screen}"
    );
    let apps = metadata(&client.tool("list_apps", json!({}))?)?;
    ensure!(
        apps["apps"].as_array().is_some_and(|items| items
            .iter()
            .any(|app| app["pid"] == windows[1]["pid"] && app["running"] == true)),
        "native app discovery: {apps}"
    );
    let idle = metadata(&client.tool("desktop_context", json!({}))?)?;
    ensure!(
        idle["seat_name"].is_null() && idle["worker_pid"].is_null(),
        "discovery created an agent seat"
    );
    fs::write(
        evidence.join("discovery.json"),
        serde_json::to_vec_pretty(
            &json!({"windows":listed,"screen":screen,"apps":apps,"idle":idle}),
        )?,
    )?;
    let verification = metadata(&client.tool(
        "desktop_verify",
        json!({
            "pid": windows[1]["pid"], "window_id": target,
            "expect": [{"window":{"exists":true}}], "timeout_ms":1000, "stable_samples":2
        }),
    )?)?;
    ensure!(
        verification["status"] == "satisfied" && verification["samples"] == 2,
        "window existence verification: {verification}"
    );
    let unsupported = metadata(&client.tool(
        "desktop_verify",
        json!({
            "pid": windows[1]["pid"], "window_id": target,
            "expect": [{"window":{"bounds":{"x":0,"y":0,"width":100,"height":100}}}], "timeout_ms":0
        }),
    )?)?;
    ensure!(
        unsupported["status"] == "unsatisfied",
        "mismatched rendered bounds result: {unsupported}"
    );
    let catalog = client.call("tools/list", json!({}))?;
    ensure!(
        catalog["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "desktop_capture_action"),
        "capture tools missing"
    );
    let context = metadata(&client.tool("desktop_context", json!({}))?)?;
    ensure!(
        context["seat_name"].is_null(),
        "observation created input seat"
    );
    inject(100, 150)?;
    wait("preexisting primary ownership", || {
        let coordination = request(&socket, json!("InputCoordination"))?;
        Ok(coordination["InputCoordination"]["human_owned_windows"]
            .as_array()
            .is_some_and(|windows| windows.iter().any(|window| window == target))
            .then_some(()))
    })?;
    let preowned = observe(&mut client, output, target)?;
    fs::write(
        evidence.join("preowned-observation.json"),
        serde_json::to_vec_pretty(&preowned)?,
    )?;
    let preowned_before = events(&event_log).len();
    let preowned_action = click(&preowned);
    let preowned_receipt =
        metadata(&client.tool("desktop_capture_action", preowned_action.clone())?)?;
    fs::write(
        evidence.join("preowned-action.json"),
        serde_json::to_vec_pretty(&preowned_action)?,
    )?;
    fs::write(
        evidence.join("preowned-receipt.json"),
        serde_json::to_vec_pretty(&preowned_receipt)?,
    )?;
    ensure!(
        preowned_receipt["effect"] == "paused"
            && preowned_receipt["paused"] == true
            && preowned_receipt["handled_effects"] == 0
            && preowned_receipt["handled"] == false
            && preowned_receipt["capture_consumed"] == true
            && preowned_receipt["completed_prefix"] == 0,
        "preexisting ownership receipt: {preowned_receipt}"
    );
    ensure!(
        !events(&event_log)
            .iter()
            .skip(preowned_before)
            .any(|event| {
                matches!(event["event"].as_str(), Some("button" | "key"))
                    && event["state"]
                        .as_str()
                        .is_some_and(|state| state.contains("Pressed"))
            }),
        "preexisting ownership delivered input"
    );
    ensure!(
        client.tool("desktop_capture_action", preowned_action)?["isError"] == true,
        "paused capture replay accepted"
    );
    // Start on blank space to the left: the animation crosses window 1 en route
    // to captured window 2, without granting pointer delivery to window 1.
    inject(0, 150)?;
    wait("primary ownership leaves fixture", || {
        let coordination = request(&socket, json!("InputCoordination"))?;
        Ok(coordination["InputCoordination"]["human_owned_windows"]
            .as_array()
            .is_some_and(|windows| {
                !windows
                    .iter()
                    .any(|window| window == target || window == human)
            })
            .then_some(()))
    })?;
    let observation = observe(&mut client, output, target)?;
    if std::env::var_os("DOJO_PERCEPTION_INSTALLATION").is_some() {
        let other_observation = observe(&mut other, output, target)?;
        let pending = client.begin_call("tools/call", json!({"name":"parse_visual_regions","arguments":{"capture_id":observation["capture_id"],"options":{"max_regions":32}}}))?;
        let other_decoded = other.tool("desktop_decode_model", json!({"capture_id":other_observation["capture_id"],"profile":"ui_tars","raw":"Action: click(start_box='(0,0)')"}))?;
        ensure!(
            other_decoded["isError"] != true,
            "another session's read was blocked or refused during perception"
        );
        ensure!(
            matches!(client.replies.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "perception completed before the concurrent service read could be demonstrated"
        );
        let parsed = metadata(&client.finish_call(pending, Duration::from_secs(40))?)?;
        ensure!(
            parsed["capture"]["capture_id"] == observation["capture_id"]
                && parsed["capture"]["screenshot"]["sha256"] == observation["digest"],
            "live perception changed capture identity: {parsed}"
        );
        ensure!(
            parsed["regions"]
                .as_array()
                .is_some_and(|regions| regions.iter().all(|region| region["interactive"] == false)),
            "live perception introduced action authority"
        );
        fs::write(
            evidence.join("perception.json"),
            serde_json::to_vec_pretty(&parsed)?,
        )?;
        let preserved = client.tool("desktop_decode_model", json!({"capture_id":observation["capture_id"],"profile":"ui_tars","raw":"Action: click(start_box='(0,0)')"}))?;
        ensure!(
            preserved["isError"] != true,
            "perception consumed or retired the live capture"
        );
    }
    fs::write(
        evidence.join("initial-observation.json"),
        serde_json::to_vec_pretty(&observation)?,
    )?;
    ensure!(
        observation["actionable"] == true,
        "exact target not actionable"
    );
    let decoded = metadata(&client.tool("desktop_decode_model", json!({
        "capture_id":observation["capture_id"],"profile":"ui_tars","raw":"Action: click(start_box='(0,0)')"
    }))?)?;
    ensure!(
        decoded["authorized"] == false
            && decoded["capture_consumed"] == false
            && decoded["outcome"]["Action"]["Click"]["at"]["delivered_x"] == 0.0,
        "server-owned model view: {decoded}"
    );
    ensure!(other.tool("desktop_decode_model", json!({"capture_id":observation["capture_id"],"profile":"ui_tars","raw":"Action: click(start_box='(0,0)')"}))?["isError"] == true, "cross-session model decode accepted");
    let cross = other.tool("desktop_capture_action", click(&observation))?;
    ensure!(
        cross["isError"] == true,
        "another client accepted a capture"
    );
    let before = events(&event_log).len();
    let initial_action = click(&observation);
    fs::write(
        evidence.join("initial-action.json"),
        serde_json::to_vec_pretty(&initial_action)?,
    )?;
    let receipt = metadata(&client.tool("desktop_capture_action", initial_action)?)?;
    fs::write(
        evidence.join("initial-receipt.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    ensure!(
        receipt["effect"] == "handled" && receipt["verified"] == false,
        "click receipt: {receipt}"
    );
    wait("actual target button delivery", || {
        Ok(events(&event_log)
            .iter()
            .skip(before)
            .any(|e| {
                e["event"] == "button"
                    && e["window"] == 2
                    && e["state"].as_str().is_some_and(|s| s.contains("Pressed"))
            })
            .then_some(()))
    })?;
    let path_events = events(&event_log);
    let agent_seat = path_events
        .iter()
        .find(|event| event["event"] == "seat_name" && event["name"] == receipt["seat_name"])
        .and_then(|event| event["seat"].as_u64())
        .context("capture agent seat")?;
    ensure!(
        !path_events.iter().skip(before).any(|event| {
            event["seat"] == agent_seat && event["event"] == "pointer_enter" && event["window"] == 1
        }),
        "captured animation entered an unobserved window"
    );
    fs::write(
        evidence.join("captured-path-events.json"),
        serde_json::to_vec_pretty(&path_events[before..])?,
    )?;
    let replay = client.tool("desktop_capture_action", click(&observation))?;
    ensure!(replay["isError"] == true, "capture reused");
    let focused = request(&socket, json!("FocusedWindow"))?;
    ensure!(
        focused["FocusedWindow"]["id"] == human,
        "agent changed human focus"
    );
    let double_observation = observe(&mut client, output, target)?;
    let mut double = click(&double_observation);
    double["input"]["kind"] = "double_click".into();
    let double_before = events(&event_log).len();
    let double_receipt = metadata(&client.tool("desktop_capture_action", double)?)?;
    ensure!(
        double_receipt["effect"] == "handled" && double_receipt["completed_prefix"] == 2,
        "double-click receipt: {double_receipt}"
    );
    wait("two target click presses", || {
        Ok((events(&event_log)
            .iter()
            .skip(double_before)
            .filter(|e| {
                e["event"] == "button"
                    && e["window"] == 2
                    && e["state"].as_str().is_some_and(|s| s.contains("Pressed"))
            })
            .count()
            == 2)
            .then_some(()))
    })?;
    let chord_observation = observe(&mut client, output, target)?;
    let mut chord = click(&chord_observation);
    chord["input"] = json!({"kind":"chord","keys":["Control","a"]});
    let chord_before = events(&event_log).len();
    let chord_receipt = metadata(&client.tool("desktop_capture_action", chord)?)?;
    ensure!(
        chord_receipt["effect"] == "handled" && chord_receipt["completed_prefix"] == 1,
        "chord receipt: {chord_receipt}"
    );
    wait("target physical chord delivery", || {
        let records = events(&event_log);
        let keys: Vec<_> = records
            .iter()
            .skip(chord_before)
            .filter(|e| e["event"] == "key" && e["window"] == 2)
            .collect();
        Ok((keys
            .iter()
            .any(|e| e["key"] == 29 && e["state"].as_str().is_some_and(|s| s.contains("Pressed")))
            && keys.iter().any(|e| {
                e["key"] == 30 && e["state"].as_str().is_some_and(|s| s.contains("Released"))
            }))
        .then_some(()))
    })?;
    ensure!(
        request(&socket, json!("FocusedWindow"))?["FocusedWindow"]["id"] == human,
        "rich action changed human focus"
    );
    client.tool("desktop_release", json!({}))?;
    let text_observation = observe(&mut client, output, target)?;
    let frame = &text_observation["frame"];
    let bounds_receipt = metadata(&client.tool("desktop_verify", json!({
        "pid":windows[1]["pid"],"window_id":target,
        "expect":[{"window":{"bounds":{"x":frame["logical_x"],"y":frame["logical_y"],"width":frame["logical_width"],"height":frame["logical_height"]}}}],"timeout_ms":1000,"stable_samples":2
    }))?)?;
    ensure!(
        bounds_receipt["status"] == "satisfied",
        "same-render bounds verification: {bounds_receipt}"
    );
    let mut text = click(&text_observation);
    text["input"] = json!({"kind":"text","text":"Az!? "});
    text["observe_after"] = true.into();
    let text_before = events(&event_log).len();
    let text_result = client.tool("desktop_capture_action", text)?;
    let text_receipt = metadata(&text_result)?;
    ensure!(
        text_receipt["effect"] == "handled" && text_receipt["completed_prefix"] == 5,
        "physical text receipt: {text_receipt}"
    );
    ensure!(
        text_receipt["post_observation"]["capture_id"] != text_observation["capture_id"]
            && text_result["content"]
                .as_array()
                .unwrap()
                .iter()
                .any(|block| block["type"] == "image"),
        "execute-observe did not deliver fresh capture and image"
    );
    wait("physical text delivery and shift release", || {
        let records = events(&event_log);
        let keys: Vec<_> = records
            .iter()
            .skip(text_before)
            .filter(|e| e["event"] == "key" && e["window"] == 2)
            .collect();
        Ok((keys.iter().any(|e| {
            e["key"] == 57 && e["state"].as_str().is_some_and(|s| s.contains("Released"))
        }) && keys
            .iter()
            .filter(|e| {
                e["key"] == 42 && e["state"].as_str().is_some_and(|s| s.contains("Pressed"))
            })
            .count()
            == 3)
            .then_some(()))
    })?;
    let hold_observation = observe(&mut client, output, target)?;
    let mut hold = click(&hold_observation);
    hold["input"] = json!({"kind":"key_hold","key":"a","duration_ms":30});
    let hold_before = events(&event_log).len();
    let hold_receipt = metadata(&client.tool("desktop_capture_action", hold)?)?;
    ensure!(
        hold_receipt["effect"] == "handled" && hold_receipt["completed_prefix"] == 1,
        "bounded key hold receipt: {hold_receipt}"
    );
    wait("bounded key hold release", || {
        Ok(events(&event_log)
            .iter()
            .skip(hold_before)
            .any(|e| {
                e["event"] == "key"
                    && e["window"] == 2
                    && e["key"] == 30
                    && e["state"].as_str().is_some_and(|s| s.contains("Released"))
            })
            .then_some(()))
    })?;
    let drag_observation = observe(&mut client, output, target)?;
    let mut drag = click(&drag_observation);
    drag["input"] = json!({"kind":"drag","button":"left","path":[
        {"x":drag_observation["image_width"].as_f64().unwrap()*0.6,"y":drag_observation["image_height"].as_f64().unwrap()*0.6},
        {"x":drag_observation["image_width"].as_f64().unwrap()*0.5,"y":drag_observation["image_height"].as_f64().unwrap()*0.5}
    ]});
    let drag_before = events(&event_log).len();
    let drag_receipt = metadata(&client.tool("desktop_capture_action", drag)?)?;
    ensure!(
        drag_receipt["effect"] == "handled" && drag_receipt["completed_prefix"] == 2,
        "in-window drag receipt: {drag_receipt}"
    );
    wait("drag button release", || {
        Ok(events(&event_log)
            .iter()
            .skip(drag_before)
            .any(|e| {
                e["event"] == "button"
                    && e["window"] == 2
                    && e["state"].as_str().is_some_and(|s| s.contains("Released"))
            })
            .then_some(()))
    })?;
    ensure!(
        request(&socket, json!("FocusedWindow"))?["FocusedWindow"]["id"] == human,
        "text/hold/drag changed human focus"
    );
    let geometry_hold_observation = observe(&mut client, output, target)?;
    let mut geometry_hold = click(&geometry_hold_observation);
    geometry_hold["input"] = json!({"kind":"key_hold","key":"a","duration_ms":1000});
    let geometry_before = events(&event_log).len();
    let holding = std::thread::spawn(move || {
        let result = client.tool("desktop_capture_action", geometry_hold);
        (client, result)
    });
    wait("key held before same-window movement", || {
        Ok(events(&event_log)
            .iter()
            .skip(geometry_before)
            .any(|e| {
                e["event"] == "key"
                    && e["window"] == 2
                    && e["key"] == 30
                    && e["state"].as_str().is_some_and(|s| s.contains("Pressed"))
            })
            .then_some(()))
    })?;
    // Shift by ten pixels so the pointer stays inside the same exact root.
    request(
        &socket,
        json!({"Action":{"MoveFloatingWindow":{"id":target,"x":{"SetFixed":340.},"y":{"SetFixed":60.}}}}),
    )?;
    let (returned_client, geometry_result) = holding
        .join()
        .map_err(|_| anyhow::anyhow!("geometry hold worker panicked"))?;
    client = returned_client;
    let geometry_receipt = metadata(&geometry_result?)?;
    ensure!(
        geometry_receipt["handled"] == false
            && geometry_receipt["capture_consumed"] == true
            && geometry_receipt["completed_prefix"] == 0,
        "same target movement completed held action: {geometry_receipt}"
    );
    wait("held key release on captured geometry change", || {
        Ok(events(&event_log)
            .iter()
            .skip(geometry_before)
            .any(|e| {
                e["event"] == "key"
                    && e["key"] == 30
                    && e["state"].as_str().is_some_and(|s| s.contains("Released"))
            })
            .then_some(()))
    })?;
    request(
        &socket,
        json!({"Action":{"MoveFloatingWindow":{"id":target,"x":{"SetFixed":330.},"y":{"SetFixed":60.}}}}),
    )?;
    std::thread::sleep(Duration::from_millis(600));
    let before_pi = events(&event_log).len();
    let mut pi = Process(
        Command::new("node")
            .arg(root.join("tools/speech/check-capture-live.mjs"))
            .arg(&evidence)
            .arg(output)
            .arg(target.to_string())
            .current_dir(&root)
            .env("WAYLAND_DISPLAY", &display)
            .env("NIRI_SOCKET", &socket)
            .stdout(fs::File::create(evidence.join("pi.log"))?)
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    wait("Pi desktop acceptance", || match pi.0.try_wait()? {
        Some(status) => {
            ensure!(status.success(), "Pi acceptance failed: {status}");
            Ok(Some(()))
        }
        None => Ok(None),
    })?;
    wait("actual Pi-owned click delivery", || {
        Ok(events(&event_log)
            .iter()
            .skip(before_pi)
            .any(|e| {
                e["event"] == "button"
                    && e["window"] == 2
                    && e["state"].as_str().is_some_and(|s| s.contains("Pressed"))
            })
            .then_some(()))
    })?;
    let takeover_capture = observe(&mut client, output, target)?;
    let mut takeover_hold = click(&takeover_capture);
    takeover_hold["input"] = json!({"kind":"key_hold","key":"a","duration_ms":1000});
    let takeover_before = events(&event_log).len();
    let holding = std::thread::spawn(move || {
        let result = client.tool("desktop_capture_action", takeover_hold);
        (client, result)
    });
    wait("held key before human takeover", || {
        Ok(events(&event_log)
            .iter()
            .skip(takeover_before)
            .any(|e| {
                e["event"] == "key"
                    && e["window"] == 2
                    && e["key"] == 30
                    && e["state"].as_str().is_some_and(|s| s.contains("Pressed"))
            })
            .then_some(()))
    })?;
    inject(100, 150)?;
    let (returned_client, takeover_result) = holding
        .join()
        .map_err(|_| anyhow::anyhow!("hold worker panicked"))?;
    client = returned_client;
    let takeover_receipt = metadata(&takeover_result?)?;
    ensure!(
        takeover_receipt["handled"] == false
            && takeover_receipt["capture_consumed"] == true
            && takeover_receipt["completed_prefix"] == 0,
        "human takeover completed held action: {takeover_receipt}"
    );
    wait("held key release on human takeover", || {
        Ok(events(&event_log)
            .iter()
            .skip(takeover_before)
            .any(|e| {
                e["event"] == "key"
                    && e["key"] == 30
                    && e["state"].as_str().is_some_and(|s| s.contains("Released"))
            })
            .then_some(()))
    })?;
    inject(100, 150)?;
    let blocked = client.tool("desktop_capture_action", click(&takeover_capture))?;
    ensure!(
        blocked["isError"] == true,
        "human ownership change accepted stale capture"
    );
    inject(
        dimensions["width"].as_u64().unwrap() as u32 - 20,
        dimensions["height"].as_u64().unwrap() as u32 - 20,
    )?;
    let stale = observe(&mut client, output, target)?;
    request(
        &socket,
        json!({"Action":{"MoveFloatingWindow":{"id":target,"x":{"SetFixed":350.},"y":{"SetFixed":80.}}}}),
    )?;
    std::thread::sleep(Duration::from_millis(600));
    ensure!(
        client.tool("desktop_capture_action", click(&stale))?["isError"] == true,
        "changed geometry accepted"
    );
    // Cover the target with the other exact window. The visible crop is not an isolated surface capture.
    request(
        &socket,
        json!({"Action":{"MoveFloatingWindow":{"id":human,"x":{"SetFixed":350.},"y":{"SetFixed":80.}}}}),
    )?;
    request(&socket, json!({"Action":{"FocusWindow":{"id":human}}}))?;
    std::thread::sleep(Duration::from_millis(600));
    let covered = observe(&mut client, output, target)?;
    let before = events(&event_log).len();
    let refused = metadata(&client.tool("desktop_capture_action", click(&covered))?)?;
    ensure!(
        refused["handled"] == false,
        "occluded target clicked: {refused}"
    );
    std::thread::sleep(Duration::from_millis(100));
    ensure!(
        !events(&event_log)
            .iter()
            .skip(before)
            .any(|e| e["event"] == "button"
                && e["state"].as_str().is_some_and(|s| s.contains("Pressed"))),
        "click leaked into covering window"
    );
    client.tool("desktop_release", json!({}))?;
    other.tool("desktop_release", json!({}))?;
    ensure!(
        client.tool("desktop_capture_action", click(&covered))?["isError"] == true,
        "release retained capture"
    );
    request(
        &socket,
        json!({"Action":{"MoveFloatingWindow":{"id":human,"x":{"SetFixed":20.0},"y":{"SetFixed":60.0}}}}),
    )?;
    request(
        &socket,
        json!({"Action":{"MoveWindowToTiling":{"id":target}}}),
    )?;
    std::thread::sleep(Duration::from_millis(700));
    let tiled = observe(&mut client, output, target)?;
    ensure!(
        tiled["scene_frame_serial"]
            .as_u64()
            .is_some_and(|serial| serial > 0)
            && tiled["frame"]["output"]["compositor"]["pid"] == compositor.0.id(),
        "tiled capture lacks same-render evidence: {tiled}"
    );
    let before_tile = events(&event_log).len();
    let tile_receipt = metadata(&client.tool("desktop_capture_action", click(&tiled))?)?;
    ensure!(
        tile_receipt["effect"] == "handled",
        "tiled input receipt: {tile_receipt}"
    );
    wait("tiled target actual button delivery", || {
        Ok(events(&event_log)
            .iter()
            .skip(before_tile)
            .any(|e| {
                e["event"] == "button"
                    && e["window"] == 2
                    && e["state"].as_str().is_some_and(|s| s.contains("Pressed"))
            })
            .then_some(()))
    })?;
    client.tool("desktop_release", json!({}))?;
    client.tool(
        "desktop_action",
        json!({"action":"focus","window_id":target}),
    )?;
    let before_hold = events(&event_log).len();
    client.tool(
        "desktop_action",
        json!({"action":"key","evdev":42,"state":"press"}),
    )?;
    let held_seat = wait("held modifier delivery", || {
        Ok(events(&event_log)
            .iter()
            .skip(before_hold)
            .find(|e| {
                e["event"] == "key"
                    && e["key"] == 42
                    && e["state"].as_str().is_some_and(|s| s.contains("Pressed"))
            })
            .and_then(|e| e["seat"].as_u64()))
    })?;
    // Abrupt MCP loss must release held input through controller/socket ownership.
    drop(client);
    wait("held modifier release on MCP loss", || {
        Ok(events(&event_log)
            .iter()
            .skip(before_hold)
            .any(|e| {
                e["event"] == "key"
                    && e["seat"] == held_seat
                    && e["key"] == 42
                    && e["state"].as_str().is_some_and(|s| s.contains("Released"))
            })
            .then_some(()))
    })?;
    wait("seat removal on MCP loss", || {
        Ok(events(&event_log)
            .iter()
            .skip(before_hold)
            .any(|e| e["event"] == "seat_removed" && e["seat"] == held_seat)
            .then_some(()))
    })?;
    if std::env::var_os("DOJO_PERCEPTION_INSTALLATION").is_some() {
        let mut cancelled = Mcp::open(&root, &display, &socket, "capture-cancelled")?;
        let cancelled_capture = observe(&mut cancelled, output, target)?;
        let pending = cancelled.begin_call("tools/call", json!({"name":"parse_visual_regions","arguments":{"capture_id":cancelled_capture["capture_id"]}}))?;
        std::thread::sleep(Duration::from_millis(100));
        writeln!(
            cancelled.stdin,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":pending,"reason":"fixture cancellation"}})
        )?;
        std::thread::sleep(Duration::from_millis(200));
        let stale = cancelled.tool("desktop_decode_model", json!({"capture_id":cancelled_capture["capture_id"],"profile":"ui_tars","raw":"Action: click(start_box='(0,0)')"}))?;
        ensure!(
            stale["isError"] == true,
            "cancelled MCP request retained its retired connection capture"
        );
        let other_observation = observe(&mut other, output, target)?;
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let retry = other.tool(
                "parse_visual_regions",
                json!({"capture_id":other_observation["capture_id"]}),
            )?;
            if retry["isError"] != true {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "perception worker remained busy after MCP cancellation"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        fs::write(
            evidence.join("perception-cancellation.json"),
            serde_json::to_vec_pretty(
                &json!({"mcp_cancelled":true,"cancelled_capture_retired":true,"other_session_parse_recovered":true}),
            )?,
        )?;
    }
    fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":true,"preexisting_human_ownership_paused_without_effects":true,"captured_path_never_enters_other_window":true,"target":target,"human":human,"cross_session_refused":true,"replay_refused":true,"geometry_change_refused":true,"human_takeover_refused":true,"occluding_window_preserved":true,"pi_live_mcp":true,"pi_text_profile_rust_pipeline":true,"disconnect_released_held_input":true,"physical_ascii_text":true,"bounded_key_hold":true,"in_window_drag":true,"execute_observe_fresh_image":true,"same_render_bounds_verification":true,"human_takeover_mid_hold_released":true,"same_window_geometry_mid_hold_released":true}),
        )?,
    )?;
    // Drop MCP subprocesses before the fixture and compositor, triggering owned socket cleanup.
    drop(other);
    Ok(())
}
