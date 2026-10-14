//! Exercise scene capture against the real renderer in an isolated nested compositor.
//! Run inside Dojo's devenv after building Niri and tools/agent-input's fixture.
use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

struct Peer(BufReader<UnixStream>);
impl Peer {
    fn connect(path: &str) -> Result<Self> {
        let socket = UnixStream::connect(path)?;
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
        socket.set_write_timeout(Some(Duration::from_secs(10)))?;
        Ok(Self(BufReader::new(socket)))
    }
    fn raw(&mut self, request: Value) -> Result<Value> {
        serde_json::to_writer(self.0.get_mut(), &request)?;
        self.0.get_mut().write_all(b"\n")?;
        let mut line = String::new();
        self.0.read_line(&mut line)?;
        Ok(serde_json::from_str(&line)?)
    }
    fn request(&mut self, request: Value) -> Result<Value> {
        let response = self.raw(request)?;
        if let Some(error) = response.get("Err") {
            bail!("{error}");
        }
        Ok(response["Ok"].clone())
    }
    fn focus_id(&mut self) -> Result<Value> {
        Ok(self.request(json!("FocusedWindow"))?["FocusedWindow"]["id"].clone())
    }
}

struct Processes(Vec<Child>);
impl Drop for Processes {
    fn drop(&mut self) {
        for child in self.0.iter_mut().rev() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_for<T>(mut check: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Some(value) = check()? {
            return Ok(value);
        }
        thread::sleep(Duration::from_millis(25));
    }
    bail!("nested scene fixture did not become ready")
}

fn capture(peer: &mut Peer, output: &str) -> Result<Option<Value>> {
    let value = peer.raw(json!({"CaptureScene": {"output": output}}))?;
    if let Some(error) = value.get("Err") {
        ensure!(
            error
                .as_str()
                .is_some_and(|s| s.contains("scene_animation_unsupported")
                    || s.contains("scene_overview_unsupported")),
            "{error}"
        );
        return Ok(None);
    }
    Ok(Some(value["Ok"]["CapturedScene"].clone()))
}

fn check(
    peer: &mut Peer,
    output: &str,
    evidence: &Path,
    label: &str,
    colors: &[(u64, [u8; 4])],
    compositor_pid: u32,
    fixture_pid: u32,
    previous_serial: &mut u64,
) -> Result<Value> {
    wait_for(|| capture(peer, output))?;
    let focus = peer.focus_id()?;
    let scene = wait_for(|| capture(peer, output))?;
    ensure!(peer.focus_id()? == focus, "capture changed focus");
    let serial = scene["frame_serial"].as_u64().context("missing serial")?;
    ensure!(serial > *previous_serial, "serial did not advance");
    *previous_serial = serial;
    ensure!(
        scene["compositor_pid"] == compositor_pid,
        "wrong compositor"
    );
    let png = STANDARD.decode(scene["png_base64"].as_str().context("missing PNG")?)?;
    fs::write(evidence.join(format!("{label}.png")), &png)?;
    let mut reader = png::Decoder::new(Cursor::new(&png)).read_info()?;
    let mut pixels = vec![0; reader.output_buffer_size().context("unbounded PNG")?];
    let info = reader.next_frame(&mut pixels)?;
    ensure!(
        info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight,
        "unexpected PNG format"
    );
    ensure!(
        scene["width"] == info.width && scene["height"] == info.height,
        "wrong dimensions"
    );
    let scale = scene["logical_output"]["scale"]
        .as_f64()
        .context("missing scale")?;
    let windows = scene["windows"].as_array().context("missing windows")?;
    for (id, expected) in colors {
        let window = windows
            .iter()
            .find(|window| window["id"] == *id)
            .context("window omitted")?;
        ensure!(
            window["pid"] == fixture_pid && window["blocked_out"] == (*expected == [0, 0, 0, 255]),
            "wrong identity/privacy"
        );
        let rect = window["output_clip"].as_array().context("missing clip")?;
        let rect: Vec<_> = rect.iter().map(|v| v.as_f64().unwrap()).collect();
        let x = ((rect[0] + rect[2] / 2.) * scale).round() as usize;
        let y = ((rect[1] + rect[3] / 2.) * scale).round() as usize;
        let pos = (y * info.width as usize + x) * 4;
        ensure!(
            pixels.get(pos..pos + 4) == Some(expected.as_slice()),
            "{label}: wrong pixel for {id} at {x},{y}"
        );
    }
    let mut metadata = scene.clone();
    metadata["png_base64"] = json!(format!("<{} PNG bytes>", png.len()));
    fs::write(
        evidence.join(format!("{label}.json")),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    Ok(scene)
}

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let evidence =
        std::env::temp_dir().join(format!("dojo-scene-capture-{}-{stamp}", std::process::id()));
    fs::create_dir(&evidence)?;
    println!("Evidence: {}", evidence.display());
    let config = evidence.join("nested.kdl");
    fs::copy(root.join("tools/agent-input/nested.kdl"), &config)?;
    let log = evidence.join("niri.log");
    let mut processes = Processes(vec![Command::new(
        root.join("vendor/niri/target/debug/niri"),
    )
    .arg("--config")
    .arg(&config)
    .env("RUST_LOG", "niri=info")
    .stdout(fs::File::create(&log)?)
    .stderr(Stdio::from(fs::OpenOptions::new().append(true).open(&log)?))
    .spawn()?]);
    let compositor_pid = processes.0[0].id();
    let (display, ipc) = wait_for(|| {
        ensure!(processes.0[0].try_wait()?.is_none(), "compositor stopped");
        let text = fs::read_to_string(&log)?;
        let find = |marker: &str| {
            text.lines().find_map(|line| {
                line.split_once(marker)
                    .map(|(_, tail)| tail.split_whitespace().next().unwrap().to_owned())
            })
        };
        Ok(find("listening on Wayland socket: ").zip(find("IPC listening on: ")))
    })?;
    let mut peer = Peer::connect(&ipc)?;
    let outputs = peer.request(json!("Outputs"))?;
    let output = outputs["Outputs"]
        .as_object()
        .context("missing outputs")?
        .keys()
        .next()
        .context("no output")?
        .clone();
    processes.0.push(
        Command::new(root.join("tools/agent-input/target/debug/dojo-agent-input-fixture"))
            .env("WAYLAND_DISPLAY", display)
            .stdout(fs::File::create(evidence.join("fixture.log"))?)
            .stderr(Stdio::null())
            .spawn()?,
    );
    let fixture_pid = processes.0[1].id();
    let windows = wait_for(|| {
        let values = peer.request(json!("Windows"))?;
        let mut windows: Vec<_> = values["Windows"]
            .as_array()
            .context("missing windows")?
            .iter()
            .filter(|w| w["app_id"] == "dojo-agent-input-fixture")
            .cloned()
            .collect();
        windows.sort_by(|a, b| a["title"].as_str().cmp(&b["title"].as_str()));
        Ok((windows.len() == 2).then_some(windows))
    })?;
    let colors = [
        (windows[0]["id"].as_u64().unwrap(), [40, 90, 132, 255]),
        (windows[1]["id"].as_u64().unwrap(), [138, 89, 48, 255]),
    ];
    let mut serial = 0;
    let mut check_scene = |peer: &mut Peer, label| {
        check(
            peer,
            &output,
            &evidence,
            label,
            &colors,
            compositor_pid,
            fixture_pid,
            &mut serial,
        )
    };
    let scene = check_scene(&mut peer, "floating")?;
    ensure!(
        scene["windows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["is_floating"] == true),
        "fixture not floating"
    );
    peer.request(json!({"Action": {"ToggleWindowFloating": {"id": colors[1].0}}}))?;
    let scene = check_scene(&mut peer, "tiled")?;
    ensure!(
        scene["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["id"] == colors[1].0 && w["is_floating"] == false),
        "fixture not tiled"
    );
    peer.request(
        json!({"Output": {"output": output, "action": {"Scale": {"scale": {"Specific": 1.25}}}}}),
    )?;
    let scene = check_scene(&mut peer, "fractional")?;
    ensure!(
        scene["logical_output"]["scale"] == 1.25,
        "fractional scale was not applied"
    );
    ensure!(
        scene["logical_output"]["transform"] == "Flipped180",
        "unexpected Winit orientation"
    );
    let refusal = peer.raw(json!({"CaptureScene": {"output": "nonexistent-output"}}))?;
    ensure!(
        refusal["Err"]
            .as_str()
            .is_some_and(|s| s.contains("scene_output_unavailable")),
        "{refusal}"
    );
    peer.request(json!({"Action": {"ToggleOverview": {}}}))?;
    let refusal = peer.raw(json!({"CaptureScene": {"output": output}}))?;
    ensure!(
        refusal["Err"]
            .as_str()
            .is_some_and(|s| s.contains("scene_overview_unsupported")),
        "{refusal}"
    );
    peer.request(json!({"Action": {"ToggleOverview": {}}}))?;
    fs::OpenOptions::new()
        .append(true)
        .open(&config)?
        .write_all(
            br#"
window-rule {
    match app-id="^dojo-agent-input-fixture$" title="^Agent input fixture 1$"
    block-out-from "screen-capture"
}
"#,
        )?;
    wait_for(|| {
        let Some(scene) = capture(&mut peer, &output)? else {
            return Ok(None);
        };
        Ok(scene["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|window| window["id"] == colors[0].0 && window["blocked_out"] == true)
            .then_some(()))
    })?;
    check(
        &mut peer,
        &output,
        &evidence,
        "privacy",
        &[(colors[0].0, [0, 0, 0, 255]), colors[1]],
        compositor_pid,
        fixture_pid,
        &mut serial,
    )?;
    fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(&json!({"passed": true}))?,
    )?;
    println!("Scene capture passed");
    Ok(())
}
