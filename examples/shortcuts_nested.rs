//! Runs the global shortcuts portal for real, inside a nested Niri on a private bus:
//! the forked Niri, the real xdg-desktop-portal frontend, the shell as the backend and a small
//! portal client. The client binds a shortcut through the panel (the harness presses Enter) and
//! the run checks the answer, the saved choice and that Niri holds the key. It cannot press the
//! key itself; see the note near the end.
//!
//! ```sh
//! WTYPE=$(nix build nixpkgs#wtype --print-out-paths --no-link)/bin/wtype \
//! DOJO_XDG_DESKTOP_PORTAL=<xdg-desktop-portal>/libexec/xdg-desktop-portal \
//! devenv shell -- cargo run --example shortcuts_nested
//! ```
//!
//! Needs `target/debug/dojo` and `vendor/niri/target/debug/niri` built, and a host Wayland
//! session to nest in. The shell gets its own runtime, state and config directories.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail, ensure};
use futures_lite::StreamExt;
use niri_ipc::socket::Socket;
use niri_ipc::{Action, Request, Response};
use zbus::message::Type;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{MatchRule, MessageStream};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const FRONT: &str = "org.freedesktop.portal.GlobalShortcuts";
const APP: &str = "org.example.Recorder";

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(command: &mut Command, log: &Path) -> anyhow::Result<Process> {
    let file = std::fs::File::create(log)?;
    Ok(Process(
        command
            .stdout(file.try_clone()?)
            .stderr(file)
            .spawn()
            .with_context(|| format!("starting {command:?}"))?,
    ))
}

async fn until<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> anyhow::Result<T> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(value) = check() {
            return Ok(value);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("timed out waiting for {what}")
}

fn niri(socket: &Path, request: Request) -> anyhow::Result<Response> {
    let mut socket = Socket::connect_to(socket)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.send(request)?.map_err(anyhow::Error::msg)
}

fn layers(socket: &Path) -> Vec<String> {
    match niri(socket, Request::Layers) {
        Ok(Response::Layers(layers)) => layers.into_iter().map(|l| l.namespace).collect(),
        _ => Vec::new(),
    }
}

fn wtype(binary: &str, env: &[(&str, &Path)], args: &[&str]) -> anyhow::Result<()> {
    let mut command = Command::new(binary);
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    let status = command.status().context("running wtype")?;
    ensure!(status.success(), "wtype {args:?} failed");
    Ok(())
}

async fn response(stream: &mut MessageStream, what: &str) -> anyhow::Result<(u32, HashMap<String, OwnedValue>)> {
    let message = tokio::time::timeout(Duration::from_secs(20), stream.next())
        .await
        .with_context(|| format!("no Response to {what}"))?
        .context("bus closed")??;
    Ok(message.body().deserialize()?)
}

fn rule(interface: &str, member: &str) -> MatchRule<'static> {
    MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(interface.to_owned())
        .unwrap()
        .member(member.to_owned())
        .unwrap()
        .build()
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let wtype_binary = std::env::var("WTYPE").context("set WTYPE to the wtype binary")?;
    let frontend = std::env::var("DOJO_XDG_DESKTOP_PORTAL")
        .context("set DOJO_XDG_DESKTOP_PORTAL to the xdg-desktop-portal binary")?;
    let niri_binary = root.join("vendor/niri/target/debug/niri");
    let dojo_binary = root.join("target/debug/dojo");
    let dir = tempfile::Builder::new().prefix("dojo-shortcuts-run.").tempdir()?;
    let dir = dir.keep();
    println!("evidence: {}", dir.display());

    // A bus of our own, so nothing here meets the live session's portal or shell.
    let mut bus_config = tempfile::NamedTempFile::new()?;
    std::io::Write::write_all(&mut bus_config, b"<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth><policy context='default'><allow send_destination='*'/><allow receive_sender='*'/><allow own='*'/></policy></busconfig>")?;
    let mut bus = Command::new("dbus-daemon")
        .arg(format!("--config-file={}", bus_config.path().display()))
        .args(["--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut address = String::new();
    BufReader::new(bus.stdout.take().unwrap()).read_line(&mut address)?;
    let address = address.trim().to_owned();
    let _bus = Process(bus);

    // The nested compositor.
    let mut nested_config = std::fs::read_to_string(root.join("tools/agent-input/nested.kdl"))?;
    nested_config.push_str("\nbinds {\n    Ctrl+Alt+F11 { close-window; }\n}\n");
    std::fs::write(dir.join("nested.kdl"), nested_config)?;
    let _niri = spawn(
        Command::new(&niri_binary)
            .args(["--config"])
            .arg(dir.join("nested.kdl"))
            .env("RUST_LOG", "niri=info")
            .env("NO_COLOR", "1"),
        &dir.join("niri.log"),
    )?;
    let (display, socket) = until("the nested Niri", || {
        let log = std::fs::read_to_string(dir.join("niri.log")).ok()?;
        let find = |marker: &str| {
            log.lines()
                .rev()
                .find_map(|line| line.split(marker).nth(1))
                .map(|rest| rest.split_whitespace().next().unwrap_or_default().to_owned())
        };
        let display = find("listening on Wayland socket: ")?;
        let socket = PathBuf::from(find("IPC listening on: ")?);
        socket.exists().then_some((display, socket))
    })
    .await?;
    let host_runtime = PathBuf::from(std::env::var("XDG_RUNTIME_DIR")?);
    let wayland = host_runtime.join(&display);

    // The app's identity and the portal's routing.
    let (runtime, state, config, data) = (
        dir.join("runtime"),
        dir.join("state"),
        dir.join("config"),
        dir.join("data"),
    );
    for path in [&runtime, &state, &config, &data] {
        std::fs::create_dir_all(path)?;
    }
    std::fs::set_permissions(&runtime, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    std::fs::create_dir_all(data.join("applications"))?;
    std::fs::write(
        data.join(format!("applications/{APP}.desktop")),
        "[Desktop Entry]\nType=Application\nName=Recorder\nExec=true\n",
    )?;
    std::fs::create_dir_all(data.join("xdg-desktop-portal/portals"))?;
    std::fs::write(
        data.join("xdg-desktop-portal/portals/dojo.portal"),
        "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.dojo\nInterfaces=org.freedesktop.impl.portal.GlobalShortcuts\nUseIn=niri\n",
    )?;
    std::fs::create_dir_all(config.join("xdg-desktop-portal"))?;
    std::fs::write(
        config.join("xdg-desktop-portal/niri-portals.conf"),
        "[preferred]\ndefault=none\norg.freedesktop.impl.portal.GlobalShortcuts=dojo\n",
    )?;

    let env_command = |program: &str| {
        let mut command = Command::new(program);
        command
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", &config)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CURRENT_DESKTOP", "niri");
        command
    };
    let _frontend = spawn(env_command(&frontend).args(["-r", "-v"]), &dir.join("frontend.log"))?;
    let _shell = spawn(
        env_command(dojo_binary.to_str().unwrap())
            .arg("--enable")
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("WAYLAND_DISPLAY", &wayland)
            .env("NIRI_SOCKET", &socket),
        &dir.join("shell.log"),
    )?;

    // The client, on the same private bus.
    let app = zbus::connection::Builder::address(address.as_str())?.build().await?;
    let mut responses = MessageStream::for_match_rule(rule("org.freedesktop.portal.Request", "Response"), &app, Some(16)).await?;

    // The shell owns the backend's name once its portal thread is up.
    let dbus = zbus::fdo::DBusProxy::new(&app).await?;
    until("the shell to own the backend name", || {
        futures_lite::future::block_on(dbus.name_has_owner("org.freedesktop.impl.portal.desktop.dojo".try_into().unwrap()))
            .ok()
            .filter(|owned| *owned)
    })
    .await
    .with_context(|| format!("shell log:\n{}", std::fs::read_to_string(dir.join("shell.log")).unwrap_or_default()))?;
    println!("backend is on the bus");

    let register = async {
        loop {
            let registered = app
                .call_method(Some(PORTAL), PATH, Some("org.freedesktop.host.portal.Registry"), "Register", &(APP, HashMap::<&str, Value<'_>>::new()))
                .await;
            if registered.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(20), register).await.context("registering the app")?;

    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from("create"));
    options.insert("session_handle_token", Value::from("session"));
    app.call_method(Some(PORTAL), PATH, Some(FRONT), "CreateSession", &(options,)).await?;
    let (code, results) = response(&mut responses, "CreateSession").await?;
    ensure!(code == 0, "CreateSession answered {code}");
    let session = String::try_from(results["session_handle"].try_clone()?)?;
    println!("session {session}");

    let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
    properties.insert("description", Value::from("Push to talk"));
    properties.insert("preferred_trigger", Value::from("CTRL+ALT+k"));
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from("bind"));
    app.call_method(
        Some(PORTAL),
        PATH,
        Some(FRONT),
        "BindShortcuts",
        &(ObjectPath::try_from(session.as_str())?, vec![("ptt", properties)], "", options),
    )
    .await?;

    until("the shortcuts panel", || layers(&socket).iter().any(|n| n == "dojo-shortcuts").then_some(())).await?;
    tokio::time::sleep(Duration::from_millis(600)).await;
    let panel = dir.join("panel.png");
    niri(&socket, Request::Action(Action::ScreenshotScreen { write_to_disk: true, show_pointer: false, path: Some(panel.display().to_string()) }))?;
    println!("panel is up; screenshot {}", panel.display());

    let keys = [("WAYLAND_DISPLAY", wayland.as_path())];
    wtype(&wtype_binary, &keys, &["-k", "Return"])?;
    let (code, results) = response(&mut responses, "BindShortcuts").await?;
    ensure!(code == 0, "BindShortcuts answered {code}");
    let shortcuts: Vec<(String, HashMap<String, OwnedValue>)> = results["shortcuts"].try_clone()?.try_into()?;
    let trigger = String::try_from(shortcuts[0].1["trigger_description"].try_clone()?)?;
    ensure!(trigger == "Ctrl+Alt+K", "bound to {trigger:?}");
    println!("bound: ptt = {trigger}");
    ensure!(!layers(&socket).iter().any(|n| n == "dojo-shortcuts"), "the panel stayed open");

    // Niri must now hold the key for the shell: another connection is told so.
    let mut probe = Socket::connect_to(&socket)?;
    let probe_reply = probe
        .send(Request::GlobalShortcuts { shortcuts: vec![niri_ipc::GlobalShortcut { id: "probe".into(), key: "Ctrl+Alt+K".into() }] })?
        .map_err(anyhow::Error::msg)?;
    println!("probe: {probe_reply:?}");

    // A virtual keyboard (wtype) is delivered straight to the focused client and never passes
    // through Niri's bind matching, so the physical key cannot be pressed from here. That hop is
    // covered by Niri's `global_shortcut_is_consumed_and_reported` test on its input backend.
    ensure!(
        matches!(&probe_reply, Response::GlobalShortcuts(grants) if grants.refused.len() == 1),
        "Niri is not holding the key for the shell"
    );

    let saved = std::fs::read_to_string(state.join("dojo/global-shortcuts.toml")).context("choices were not saved")?;
    ensure!(saved.contains("Ctrl+Alt+K"), "saved choices: {saved}");
    println!("saved: {}", saved.trim().replace('\n', " | "));
    println!("OK");
    Ok(())
}
