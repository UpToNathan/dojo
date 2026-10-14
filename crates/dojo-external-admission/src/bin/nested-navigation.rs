//! Run against a disposable nested compositor; no installed binaries/configs are replaced.
use anyhow::{Context, Result, bail, ensure};
use dojo_external_admission::broker::{NavigateIntent, navigate};
use niri_ipc::{AgentInput, CapturedInputGeometry, Request, Response};
use std::{
    fs::File,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn process_start(pid: u32) -> Result<u64> {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))?
        .rsplit_once(')')
        .context("process stat")?
        .1
        .split_whitespace()
        .nth(19)
        .context("process start")?
        .parse()
        .map_err(Into::into)
}
fn wait<T>(mut f: impl FnMut() -> Option<T>, description: &str) -> Result<T> {
    let until = Instant::now() + Duration::from_secs(8);
    while Instant::now() < until {
        if let Some(value) = f() {
            return Ok(value);
        };
        std::thread::sleep(Duration::from_millis(20));
    }
    bail!("timed out waiting for {description}")
}
fn request(socket: &mut niri_ipc::socket::Socket, input: Request) -> Result<Response> {
    socket.send(input)?.map_err(anyhow::Error::msg)
}
fn events(log: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|s| serde_json::from_str(s).ok())
        .collect()
}
fn human(root: &Path, display: &str, geometry: &CapturedInputGeometry) -> Result<()> {
    let (x, y, w, h) = geometry.rect;
    let result = Command::new(root.join("tools/agent-input/target/debug/human-pointer"))
        .env("WAYLAND_DISPLAY", display)
        .args([
            format!("{}", (x + w / 2.) as u32),
            format!("{}", (y + h / 2.) as u32),
            geometry.output_geometry.width.to_string(),
            geometry.output_geometry.height.to_string(),
            "click".into(),
        ])
        .status()?;
    ensure!(result.success(), "nested human pointer failed");
    Ok(())
}
fn geometry(
    socket: &mut niri_ipc::socket::Socket,
    output: &str,
    window: u64,
    revision: u64,
) -> Result<CapturedInputGeometry> {
    let Response::CapturedScene(scene) = request(
        socket,
        Request::CaptureScene {
            output: output.into(),
        },
    )?
    else {
        bail!("capture scene missing")
    };
    let target = scene
        .windows
        .iter()
        .find(|w| w.id == window)
        .context("target is not rendered")?;
    Ok(CapturedInputGeometry {
        window_id: window,
        pid: target.pid.context("target PID")?.try_into()?,
        compositor_pid: scene.compositor_pid,
        compositor_start_time: process_start(scene.compositor_pid)?,
        coordination_revision: revision,
        output: scene.output,
        output_geometry: scene.logical_output,
        rect: target.rect,
    })
}
fn intent(
    geometry: &CapturedInputGeometry,
    peer: u64,
    id: &str,
    request_id: u64,
) -> Result<NavigateIntent> {
    Ok(NavigateIntent {
        version: 1,
        peer,
        revision: geometry.coordination_revision,
        window_id: geometry.window_id,
        browser_pid: geometry.pid,
        browser_start_time: process_start(geometry.pid)?,
        page_target: "PAGE-1".into(),
        request_id,
        url: "https://fixture.invalid/next".into(),
        intent_id: id.into(),
        grant_id: "nested-fixture-operator-capture-grant".into(),
        lifetime_ms: 1000,
        capture_geometry: serde_json::to_value(geometry)?,
    })
}
fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes)?;
    let evidence = std::env::temp_dir().join(format!(
        "dojo-navigation-live-{}-{}",
        std::process::id(),
        u64::from_le_bytes(bytes)
    ));
    std::fs::create_dir(&evidence)?;
    let actor = evidence.join("actor.sock");
    let compositor_log = evidence.join("niri.log");
    let fixture_log = evidence.join("fixture.jsonl");
    let release = evidence.join("release-effect");
    let _compositor = ChildGuard(
        Command::new(root.join("vendor/niri/target/debug/niri"))
            .args([
                "--config",
                root.join("tools/agent-input/nested.kdl").to_str().unwrap(),
            ])
            .env("DOJO_AGENT_INPUT", "1")
            .env("DOJO_EXTERNAL_ADMISSION_SOCKET", &actor)
            .env(
                "DOJO_EXTERNAL_TRUSTED_BROKER_PID",
                std::process::id().to_string(),
            )
            .env(
                "DOJO_EXTERNAL_TRUSTED_BROKER_START",
                process_start(std::process::id())?.to_string(),
            )
            .env("RUST_LOG", "niri=info")
            .env("RUST_LOG_STYLE", "never")
            .stdout(File::create(&compositor_log)?)
            .stderr(Stdio::from(
                File::options().append(true).open(&compositor_log)?,
            ))
            .spawn()?,
    );
    let (display, path) = wait(
        || {
            let log = std::fs::read_to_string(&compositor_log).ok()?;
            let display = log.lines().find_map(|line| {
                line.split_once("listening on Wayland socket: ")
                    .map(|(_, s)| s.trim().to_owned())
            })?;
            let path = log.lines().find_map(|line| {
                line.split_once("IPC listening on: ")
                    .map(|(_, s)| PathBuf::from(s.trim()))
            })?;
            if actor.exists() && path.exists() {
                Some((display, path))
            } else {
                None
            }
        },
        "nested sockets",
    )?;
    let _fixture = ChildGuard(
        Command::new(root.join("tools/agent-input/target/debug/dojo-agent-input-fixture"))
            .env("WAYLAND_DISPLAY", &display)
            .env("DOJO_TEST_CDP", &release)
            .stdout(File::create(&fixture_log)?)
            .stderr(Stdio::from(
                File::options().append(true).open(&fixture_log)?,
            ))
            .spawn()?,
    );
    let port = wait(
        || {
            events(&fixture_log)
                .iter()
                .find(|v| v["event"] == "cdp_listening")
                .and_then(|v| v["port"].as_u64())
        },
        "fixture endpoint",
    )? as u16;
    let mut socket = niri_ipc::socket::Socket::connect_to(&path)?;
    socket.set_read_timeout(Some(Duration::from_secs(4)))?;
    socket.set_write_timeout(Some(Duration::from_secs(4)))?;
    let window = wait(
        || match request(&mut socket, Request::Windows).ok()? {
            Response::Windows(windows) => windows
                .into_iter()
                .find(|w| w.app_id.as_deref() == Some("dojo-agent-input-fixture")),
            _ => None,
        },
        "native fixture window",
    )?;
    let output = match request(&mut socket, Request::Outputs)? {
        Response::Outputs(outputs) => outputs
            .into_iter()
            .find(|(_, o)| o.logical.is_some())
            .map(|(name, _)| name)
            .context("nested output")?,
        _ => bail!("outputs missing"),
    };
    std::thread::sleep(Duration::from_millis(1200));
    let Response::AgentInputCreated { peer_id, .. } = request(
        &mut socket,
        Request::AgentInput(AgentInput::Create {
            name: "external-fixture".into(),
        }),
    )?
    else {
        bail!("seat missing")
    };
    request(&mut socket, Request::YieldHumanInput)?;
    let Response::InputCoordination(coord) = request(&mut socket, Request::InputCoordination)?
    else {
        bail!("coordination missing")
    };
    let stale = geometry(&mut socket, &output, window.id, coord.revision)?;
    human(&root, &display, &stale)?;
    let endpoint = format!("127.0.0.1:{port}").parse()?;
    let denied = navigate(
        &actor,
        endpoint,
        intent(&stale, peer_id, "takeover-before", 1)?,
    )?;
    ensure!(
        denied.effect == "refused" && denied.accepted_bytes == 0,
        "takeover-before was not a zero-byte refusal"
    );
    wait(
        || {
            (events(&fixture_log)
                .iter()
                .filter(|v| v["event"] == "button")
                .count()
                >= 2)
                .then_some(())
        },
        "initial primary-seat press and release",
    )?;
    request(&mut socket, Request::YieldHumanInput)?;
    let Response::InputCoordination(coord) = request(&mut socket, Request::InputCoordination)?
    else {
        bail!("coordination missing")
    };
    let fresh = geometry(&mut socket, &output, window.id, coord.revision)?;
    let admitted = navigate(
        &actor,
        endpoint,
        intent(&fresh, peer_id, "takeover-after", 2)?,
    )?;
    ensure!(
        admitted.effect == "unknown" && admitted.accepted_bytes > 0,
        "admission did not preserve uncertainty"
    );
    wait(
        || {
            events(&fixture_log)
                .iter()
                .any(|v| v["event"] == "cdp_pending" && v["id"] == 2)
                .then_some(())
        },
        "held fixture effect",
    )?;
    let before_human = events(&fixture_log)
        .iter()
        .filter(|v| v["event"] == "button")
        .count();
    human(&root, &display, &fresh)?;
    wait(
        || {
            (events(&fixture_log)
                .iter()
                .filter(|v| v["event"] == "button")
                .count()
                >= before_human + 2)
                .then_some(())
        },
        "primary-seat events while effect held",
    )?;
    ensure!(
        !events(&fixture_log)
            .iter()
            .any(|v| v["event"] == "cdp_effect"),
        "effect escaped fixture hold before takeover"
    );
    std::fs::write(&release, b"release")?;
    wait(
        || {
            events(&fixture_log)
                .iter()
                .any(|v| v["event"] == "cdp_effect" && v["id"] == 2)
                .then_some(())
        },
        "late fixture effect",
    )?;
    request(&mut socket, Request::YieldHumanInput)?;
    let Response::InputCoordination(coord) = request(&mut socket, Request::InputCoordination)?
    else {
        bail!("coordination missing")
    };
    let fresh = geometry(&mut socket, &output, window.id, coord.revision)?;
    let replay = navigate(
        &actor,
        endpoint,
        intent(&fresh, peer_id, "takeover-after", 3)?,
    )?;
    ensure!(
        replay.effect == "refused" && replay.accepted_bytes == 0,
        "used intent replayed"
    );
    let mut another = niri_ipc::socket::Socket::connect_to(&path)?;
    let Response::AgentInputCreated {
        peer_id: other_peer,
        ..
    } = request(
        &mut another,
        Request::AgentInput(AgentInput::Create {
            name: "other-external-peer".into(),
        }),
    )?
    else {
        bail!("second seat missing")
    };
    let cross_peer = navigate(
        &actor,
        endpoint,
        intent(&fresh, other_peer, "fresh-other-peer-intent", 4)?,
    )?;
    ensure!(
        cross_peer.effect == "refused" && cross_peer.accepted_bytes == 0,
        "new peer bypassed unresolved target"
    );
    let native = request(
        &mut another,
        Request::AgentInput(AgentInput::Focus {
            window_id: window.id,
        }),
    )?;
    ensure!(
        matches!(native, Response::AgentInputPaused { reason, .. } if reason.contains("unresolved external effect")),
        "native agent input bypassed unresolved target"
    );
    request(&mut another, Request::AgentInput(AgentInput::Reset))?;
    let all = events(&fixture_log);
    ensure!(
        all.iter().filter(|v| v["event"] == "cdp_effect").count() == 1,
        "fixture received repeated effects"
    );
    ensure!(
        all.iter().any(|v| v["event"] == "button"),
        "native primary-seat event absent"
    );
    let result = serde_json::json!({"passed":true,"target":window.id,"pid":fresh.pid,"takeover_before_zero_bytes":true,"in_flight_unknown":true,"human_input_delivered_while_effect_held":true,"used_intent_refused":true,"cross_peer_fresh_intent_refused":true,"native_agent_follow_on_refused":true,"effect_count":1,"scope":"single-use cleartext loopback Page.navigate final writer; no browser-engine/MCP route parity claim"});
    std::fs::write(
        evidence.join("result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    println!("{}", result);
    println!("Evidence: {}", evidence.display());
    Ok(())
}
