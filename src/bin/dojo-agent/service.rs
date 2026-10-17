use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use dojo_computer::{
    CAPTURE_TTL_SECONDS, CaptureAction, CaptureAuthority, CaptureInput, CaptureScope,
    CaptureSession, normalize_key, physical_text,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{DesktopAction, DesktopSession, screenshot, validate_name};

const VERSION: u32 = 9;
const MAX_LINE: u64 = 65536;
const MAX_REPLY: u64 = 48 * 1024 * 1024;
const EMPTY_GRACE: Duration = Duration::from_millis(250);
const DOUBLE_CLICK_GAP: Duration = Duration::from_millis(80);

#[derive(Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Message {
    Hello {
        version: u32,
        policy_sha256: Option<String>,
        name: String,
        idle_timeout_seconds: u64,
    },
    Action {
        action: DesktopAction,
    },
    Observe {
        scope: CaptureScope,
    },
    CaptureAction {
        action: CaptureAction,
    },
    Verify {
        input: dojo_computer_verification::VerifyStateInput,
    },
    DecodeModel {
        input: dojo_computer::ModelDecode,
    },
    ParseVisualRegions {
        input: super::perception::VisualParseInput,
    },
    AccessibilityAction {
        input: super::AccessibilityActionRequest,
    },
    Accessibility {
        input: super::AccessibilityRequest,
    },
    Status {},
    DriverTools {},
    DriverCall {
        name: String,
        arguments: Value,
    },
    Release {},
}

fn authorize_message(policy: &super::policy::DesktopPolicy, message: &Message) -> Result<()> {
    let (name, args) = match message {
        Message::Hello { .. } | Message::Status {} | Message::Release {} => return Ok(()),
        Message::Action { action } => ("desktop_action", serde_json::to_value(action)?),
        Message::Observe { scope } => ("desktop_observe", serde_json::to_value(scope)?),
        Message::CaptureAction { action } => {
            ("desktop_capture_action", serde_json::to_value(action)?)
        }
        Message::Verify { input } => ("desktop_verify", serde_json::to_value(input)?),
        Message::DecodeModel { input } => ("desktop_decode_model", serde_json::to_value(input)?),
        Message::ParseVisualRegions { input } => {
            ("parse_visual_regions", serde_json::to_value(input)?)
        }
        Message::Accessibility { input } => ("desktop_accessibility", serde_json::to_value(input)?),
        Message::AccessibilityAction { input } => {
            ("desktop_accessibility_action", serde_json::to_value(input)?)
        }
        Message::DriverTools {} => ("desktop_driver_tools", json!({})),
        Message::DriverCall { name, arguments } => (name.as_str(), arguments.clone()),
    };
    policy.authorize(name, &args)
}

type Reply = std::result::Result<Value, String>;

pub(super) struct ActiveSession {
    stream: BufReader<UnixStream>,
    pid: u32,
    pub(super) seat_name: Option<String>,
}

pub(super) struct CancelConnection(Option<UnixStream>);
impl CancelConnection {
    pub(super) fn complete(&mut self) {
        self.0.take();
    }
}
impl Drop for CancelConnection {
    fn drop(&mut self) {
        if let Some(stream) = &self.0 {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

pub(super) fn paths(socket: &Path) -> Result<(PathBuf, PathBuf)> {
    let runtime =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is required")?);
    let directory = runtime.join("dojo-desktop");
    match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(&directory)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
        bail!("desktop service directory must be a private directory with mode 0700");
    }
    let policy = super::policy::DesktopPolicy::load()?;
    let mut key_hash = Sha256::new();
    key_hash.update(socket.as_os_str().as_encoded_bytes());
    if let Some(policy) = policy {
        key_hash.update(policy.digest.as_bytes());
    }
    if let Some(installation) = std::env::var_os("DOJO_PERCEPTION_INSTALLATION") {
        let path = std::fs::canonicalize(installation)?;
        ensure!(
            std::fs::metadata(&path)?.len() <= 16_384,
            "perception installation too large"
        );
        key_hash.update(path.as_os_str().as_encoded_bytes());
        key_hash.update(std::fs::read(path)?);
    }
    if let Ok(manifest) = std::fs::read(super::driver::artifact_manifest()) {
        key_hash.update(super::driver::binary_path().as_os_str().as_encoded_bytes());
        key_hash.update(manifest);
    }
    let digest = key_hash.finalize();
    let key = format!("{digest:x}");
    Ok((
        directory.join(format!("{}.v{VERSION}.sock", &key[..24])),
        directory.join(format!("{}.v{VERSION}.lock", &key[..24])),
    ))
}

fn read_message<T: serde::de::DeserializeOwned>(reader: &mut impl BufRead) -> Result<Option<T>> {
    read_bounded(reader, MAX_LINE)
}

fn read_bounded<T: serde::de::DeserializeOwned>(
    reader: &mut impl BufRead,
    limit: u64,
) -> Result<Option<T>> {
    let mut line = Vec::new();
    let length = reader.take(limit + 1).read_until(b'\n', &mut line)?;
    if length == 0 {
        return Ok(None);
    }
    if length as u64 > limit || line.last() != Some(&b'\n') {
        bail!("desktop message exceeds limit or is incomplete");
    }
    Ok(Some(serde_json::from_slice(&line)?))
}

fn write_message(writer: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

impl ActiveSession {
    pub(super) fn cancel_on_drop(&self) -> Result<CancelConnection> {
        Ok(CancelConnection(Some(self.stream.get_ref().try_clone()?)))
    }
    pub(super) fn spawn(socket: Option<&Path>, name: &str, idle_timeout: Duration) -> Result<Self> {
        let socket = super::niri_socket(socket)?;
        let (path, _) = paths(&socket)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut last_spawn = None;
        loop {
            match UnixStream::connect(&path) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
                    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                    let mut session = Self {
                        stream: BufReader::new(stream),
                        pid: 0,
                        seat_name: None,
                    };
                    match session.request(&Message::Hello {
                        version: VERSION,
                        policy_sha256: super::policy::DesktopPolicy::load()?
                            .map(|policy| policy.digest),
                        name: name.into(),
                        idle_timeout_seconds: idle_timeout.as_secs(),
                    }) {
                        Ok(hello) => {
                            session.pid = hello["service_pid"]
                                .as_u64()
                                .context("missing service PID")?
                                .try_into()?;
                            return Ok(session);
                        }
                        Err(error)
                            if Instant::now() < deadline
                                && error.downcast_ref::<std::io::Error>().is_some() => {}
                        Err(error) => return Err(error).context("initialize desktop session"),
                    }
                }
                Err(error)
                    if Instant::now() < deadline
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                        ) =>
                {
                    if last_spawn
                        .is_none_or(|last: Instant| last.elapsed() >= Duration::from_millis(100))
                    {
                        let mut child = Command::new(format!("/proc/{}/exe", std::process::id()))
                            .arg("--service")
                            .arg("--socket")
                            .arg(&socket)
                            .process_group(0)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::inherit())
                            .spawn()
                            .context("start desktop service")?;
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                        last_spawn = Some(Instant::now());
                    }
                }
                Err(error) => return Err(error).context("connect to desktop service"),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn request(&mut self, message: &Message) -> Result<Value> {
        self.stream
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(match message {
                Message::DriverTools {}
                | Message::DriverCall { .. }
                | Message::ParseVisualRegions { .. } => 40,
                Message::Verify { .. } => 12,
                _ => 10,
            })))?;
        write_message(self.stream.get_mut(), message)?;
        let reply: Reply = read_bounded(&mut self.stream, MAX_REPLY)?.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "desktop service stopped; select a fresh target before continuing",
            )
        })?;
        reply.map_err(anyhow::Error::msg)
    }

    pub(super) fn pid(&self) -> u32 {
        self.pid
    }

    pub(super) fn driver_pid(&mut self) -> Result<Option<u32>> {
        Ok(self.request(&Message::Status {})?["driver_worker_pid"]
            .as_u64()
            .map(u32::try_from)
            .transpose()?)
    }

    pub(super) fn inactive(&mut self) -> Result<bool> {
        match self.request(&Message::Status {}) {
            Ok(value) => {
                self.seat_name = value["seat_name"].as_str().map(str::to_owned);
                Ok(!value["session_active"]
                    .as_bool()
                    .unwrap_or(self.seat_name.is_some()))
            }
            Err(_) => Ok(true),
        }
    }

    pub(super) fn action(&mut self, action: DesktopAction) -> Result<Value> {
        let value = self.request(&Message::Action { action })?;
        self.seat_name = value["seat_name"].as_str().map(str::to_owned);
        Ok(value)
    }

    pub(super) fn observe(&mut self, scope: CaptureScope) -> Result<Value> {
        self.request(&Message::Observe { scope })
    }

    pub(super) fn capture_action(&mut self, action: CaptureAction) -> Result<Value> {
        let value = self.request(&Message::CaptureAction { action })?;
        self.seat_name = value["seat_name"].as_str().map(str::to_owned);
        Ok(value)
    }

    pub(super) fn verify(
        &mut self,
        input: dojo_computer_verification::VerifyStateInput,
    ) -> Result<Value> {
        self.request(&Message::Verify { input })
    }

    pub(super) fn decode_model(&mut self, input: dojo_computer::ModelDecode) -> Result<Value> {
        self.request(&Message::DecodeModel { input })
    }

    pub(super) fn parse_visual_regions(
        &mut self,
        input: super::perception::VisualParseInput,
    ) -> Result<Value> {
        self.request(&Message::ParseVisualRegions { input })
    }

    pub(super) fn accessibility_action(
        &mut self,
        input: super::AccessibilityActionRequest,
    ) -> Result<Value> {
        self.request(&Message::AccessibilityAction { input })
    }

    pub(super) fn accessibility(&mut self, input: super::AccessibilityRequest) -> Result<Value> {
        self.request(&Message::Accessibility { input })
    }

    pub(super) fn release(mut self) {
        let _ = self.request(&Message::Release {});
    }
    pub(super) fn driver_tools(&mut self) -> Result<Value> {
        self.request(&Message::DriverTools {})
    }
    pub(super) fn driver_call(&mut self, name: String, arguments: Value) -> Result<Value> {
        self.request(&Message::DriverCall { name, arguments })
    }
}

struct Session {
    desktop: DesktopSession,
    captures: CaptureSession,
    semantics: super::accessibility_actions::Session,
    cancelled: Arc<AtomicBool>,
    native_retirement: NativeRetirement,
    driver: Option<super::driver::Driver>,
    last_observation: Option<Instant>,
    idle_timeout: Duration,
    last_action: Instant,
}

impl Session {
    fn ensure_driver(&mut self) -> Result<()> {
        if self.desktop.controller.is_some() {
            self.release_controller();
        }
        if self.driver.is_none() {
            let retirement = self
                .native_retirement
                .lock()
                .map_err(|_| anyhow::anyhow!("retirement state unavailable"))?
                .driver
                .clone();
            self.driver = Some(super::driver::Driver::open(
                self.desktop
                    .socket
                    .as_deref()
                    .context("Driver requires this Niri socket")?,
                &self.desktop.name,
                self.idle_timeout,
                &self.cancelled,
                &retirement,
            )?);
        }
        Ok(())
    }
    fn release_controller(&mut self) {
        self.driver = None;
        self.desktop.controller = None;
        self.native_retirement
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .native
            .take();
    }
    fn ensure_controller(&mut self) -> Result<()> {
        if self.driver.is_some() {
            self.release_controller();
        }
        if self.desktop.controller.is_none() {
            let controller = match &self.desktop.socket {
                Some(path) => super::AgentController::connect_to(path, &self.desktop.name),
                None => super::AgentController::connect(&self.desktop.name),
            }?;
            let handle = controller.retirement_handle()?;
            let mut slot = self
                .native_retirement
                .lock()
                .map_err(|_| anyhow::anyhow!("native retirement state unavailable"))?;
            ensure!(
                !self.cancelled.load(Ordering::Acquire),
                "desktop session disconnected during seat creation"
            );
            slot.native = Some(handle);
            self.desktop.controller = Some(controller);
        }
        Ok(())
    }
    fn next_expiry(&self) -> Option<Instant> {
        self.desktop
            .controller
            .as_ref()
            .map(|_| self.last_action + self.idle_timeout)
            .into_iter()
            .chain(
                self.last_observation
                    .map(|last| last + Duration::from_secs(CAPTURE_TTL_SECONDS)),
            )
            .chain(self.semantics.next_expiry())
            .min()
    }

    fn active(&self) -> bool {
        self.semantics.active()
            || self.driver.is_some()
            || self.desktop.controller.is_some()
            || self
                .last_observation
                .is_some_and(|last| last.elapsed() < Duration::from_secs(CAPTURE_TTL_SECONDS))
    }

    fn expire(&mut self, now: Instant) -> Result<()> {
        if now.duration_since(self.last_action) >= self.idle_timeout {
            self.release_controller();
        }
        if self.last_observation.is_some_and(|last| {
            now.duration_since(last) >= Duration::from_secs(CAPTURE_TTL_SECONDS)
        }) {
            self.captures.retire()?;
            self.last_observation = None;
        }
        Ok(())
    }

    fn observe(&mut self, scope: CaptureScope) -> Result<Value> {
        scope.validate()?;
        ensure!(
            !self.cancelled.load(Ordering::Acquire),
            "desktop session disconnected"
        );
        let before = screenshot::current_frame(self.desktop.socket.as_deref(), &scope)?;
        let image = screenshot::capture_image_cancellable(
            self.desktop.socket.as_deref(),
            &scope,
            &self.cancelled,
        )?;
        let after = screenshot::current_frame(self.desktop.socket.as_deref(), &scope)?;
        ensure!(
            before == after,
            "capture identity, geometry or ownership changed; observe again"
        );
        if let Some(frame) = &image.frame {
            ensure!(
                *frame == after,
                "captured scene no longer matches target identity or geometry"
            );
        }
        ensure!(
            !self.cancelled.load(Ordering::Acquire),
            "desktop session disconnected"
        );
        let encoded = STANDARD.encode(&image.png);
        let observation =
            self.captures
                .publish(scope, after, image.png, image.width, image.height)?;
        self.last_observation = Some(Instant::now());
        let mut value = serde_json::to_value(observation)?;
        value["png_base64"] = encoded.into();
        value["scene_frame_serial"] = image.scene_serial.into();
        Ok(value)
    }

    fn capture_action(&mut self, action: CaptureAction) -> Result<Value> {
        action.validate()?;
        let observe_after = action.observe_after;
        let scope = self.captures.scope(&action.capture_id)?;
        let current = screenshot::current_frame(self.desktop.socket.as_deref(), &scope)?;
        ensure!(
            !self.cancelled.load(Ordering::Acquire),
            "desktop session disconnected"
        );
        let admitted = self.captures.admit(action, &current)?;

        // Supersede every other observation in this session before any possible effect.
        self.captures.retire()?;
        self.last_observation = None;
        let cancelled = self.cancelled.clone();
        let mut completed_prefix = 0usize;
        let mut handled_effects = 0usize;
        let result = (|| -> Result<()> {
            ensure!(
                !cancelled.load(Ordering::Acquire),
                "desktop session disconnected"
            );
            self.ensure_controller()?;
            let controller = self
                .desktop
                .controller
                .as_mut()
                .context("agent seat is unavailable")?;
            // Never adopt a newer ownership revision merely because seat creation
            // happened after the observation. Niri checks this revision at delivery.
            controller.use_coordination_revision(admitted.coordination_revision);
            controller.use_capture_geometry(captured_geometry(&admitted.frame)?);
            controller.move_pointer_for_window(
                admitted.window_id,
                &admitted.output,
                admitted.x,
                admitted.y,
            )?;
            ensure!(
                !cancelled.load(Ordering::Acquire),
                "desktop session disconnected during movement"
            );
            let after_move = screenshot::current_frame(self.desktop.socket.as_deref(), &scope)?;
            ensure!(
                after_move == current,
                "target changed during movement; observe again"
            );
            if matches!(
                &admitted.input,
                CaptureInput::Key { .. }
                    | CaptureInput::Chord { .. }
                    | CaptureInput::Text { .. }
                    | CaptureInput::KeyHold { .. }
            ) {
                controller.focus_window(admitted.window_id)?;
            }
            execute_capture_input(
                &mut CountedInput {
                    inner: controller,
                    handled: &mut handled_effects,
                },
                &admitted,
                &cancelled,
                &mut completed_prefix,
            )
        })();
        self.last_action = Instant::now();
        if result.is_ok() {}
        let seat = self
            .desktop
            .controller
            .as_ref()
            .map(|c| c.seat_name().to_owned());
        let receipt_result: Result<Value> = match result {
            Ok(()) => Ok(
                json!({"contract_version":dojo_computer::CONTRACT_VERSION,"capture_id":admitted.capture_id,"capture_consumed":true,
                "effect":"handled","handled":true,"completed_prefix":completed_prefix,"verified":false,"requires_fresh_observation":true,"seat_name":seat}),
            ),
            Err(error) => {
                if let Some(paused) = error.downcast_ref::<dojo::agent_input::InputPaused>() {
                    Ok(
                        json!({"contract_version":dojo_computer::CONTRACT_VERSION,"capture_id":admitted.capture_id,"capture_consumed":true,
                        "effect":if handled_effects == 0 {"paused"} else {"partial"},"handled":false,"paused":true,"completed_prefix":completed_prefix,"handled_effects":handled_effects,"reason":paused.reason,
                        "coordination":paused.coordination,"verified":false,"requires_fresh_observation":true,"seat_name":seat}),
                    )
                } else {
                    self.release_controller();
                    Ok(
                        json!({"contract_version":dojo_computer::CONTRACT_VERSION,"capture_id":admitted.capture_id,"capture_consumed":true,
                    "effect":"unknown","handled":false,"completed_prefix":completed_prefix,"handled_effects":handled_effects,"reason":format!("{error:#}"),"verified":false,
                    "requires_fresh_observation":true,"seat_name":null}),
                    )
                }
            }
        };
        let mut receipt = receipt_result?;
        if observe_after {
            match self.observe(scope) {
                Ok(observation) => receipt["post_observation"] = observation,
                Err(error) => {
                    receipt["post_observation_error"] = format!("{error:#}").into();
                    receipt["requires_fresh_observation"] = true.into();
                }
            }
        }
        Ok(receipt)
    }

    fn handle(&mut self, message: Message) -> Result<Value> {
        self.semantics.poll();
        if matches!(
            &message,
            Message::Action { .. }
                | Message::CaptureAction { .. }
                | Message::DriverCall { .. }
                | Message::Release { .. }
                | Message::Observe { .. }
        ) {
            self.semantics.retire();
        }
        match message {
            Message::DriverTools {} => {
                self.ensure_driver()?;
                self.last_action = Instant::now();
                self.driver.as_ref().unwrap().tools()
            }
            Message::DriverCall { name, arguments } => {
                self.ensure_driver()?;
                self.last_action = Instant::now();
                self.driver.as_ref().unwrap().call(name, arguments)
            }
            Message::Hello { .. } => bail!("session is already initialized"),
            Message::Status {} => Ok(
                json!({"seat_name": self.desktop.controller.as_ref().map(|controller| controller.seat_name()),
                    "driver_worker_pid": self.driver.as_ref().map(super::driver::Driver::pid),
                    "session_active": self.active()}),
            ),
            Message::Release {} => {
                self.release_controller();
                self.captures.retire()?;
                self.last_observation = None;
                Ok(json!({"released": true}))
            }
            Message::Observe { scope } => self.observe(scope),
            Message::CaptureAction { action } => self.capture_action(action),
            Message::Verify { input } => {
                input.validate()?;
                let output = super::verification::verify(
                    self.desktop.socket.as_deref(),
                    &input,
                    &self.cancelled,
                )?;
                Ok(serde_json::to_value(output)?)
            }
            Message::DecodeModel { input } => self.captures.decode_model(input),
            Message::ParseVisualRegions { .. } => {
                bail!("perception must execute outside the desktop service loop")
            }

            Message::AccessibilityAction { input } => {
                let attempt = (|| -> Result<Value> {
                    let reference = self.semantics.consume(&input.element_ref)?;
                    self.captures.retire()?;
                    self.last_observation = None;
                    self.ensure_controller()?;
                    let peer = self
                        .desktop
                        .controller
                        .as_ref()
                        .context("agent seat unavailable")?
                        .peer_id();
                    self.last_action = Instant::now();
                    self.semantics.perform(
                        self.desktop.socket.as_deref(),
                        peer,
                        reference,
                        input.action.into(),
                        &self.cancelled,
                    )
                })();
                // perform returns errors only before a possible FD handoff. Once
                // transferred, every failure is a receipt with unknown effect.
                Ok(attempt.unwrap_or_else(|error| json!({"effect":"refused","reference_consumed":true,"reason":format!("{error:#}"),"verified":false,"requires_fresh_observation":true})))
            }
            Message::Accessibility { input } => {
                ensure!(
                    !self.cancelled.load(Ordering::Acquire),
                    "desktop session disconnected"
                );
                ensure!(
                    input.window_id > 0 && input.query.as_ref().is_none_or(|q| q.len() <= 1024),
                    "invalid window/query"
                );
                let mut socket = super::desktop_socket(self.desktop.socket.as_deref())?;
                self.semantics.retire();
                let mut observation = super::accessibility::observe(
                    &mut socket,
                    input.window_id,
                    input.query,
                    Instant::now() + Duration::from_secs(5),
                    &self.cancelled,
                )?;
                ensure!(
                    !self.cancelled.load(Ordering::Acquire),
                    "desktop session disconnected"
                );
                self.semantics.issue(&mut observation);
                let mut value = serde_json::to_value(observation)?;
                value["semantic_receipts"] = serde_json::to_value(self.semantics.take_receipts())?;
                ensure!(
                    serde_json::to_vec(&value)?.len() <= 16 * 1024 * 1024,
                    "accessibility observation exceeds byte budget"
                );
                Ok(value)
            }
            Message::Action { action } => {
                action.validate()?;
                self.captures.retire()?;
                self.last_observation = None;
                ensure!(
                    !self.cancelled.load(Ordering::Acquire),
                    "desktop session disconnected"
                );
                if !matches!(&action, DesktopAction::Reset {}) || self.desktop.controller.is_some()
                {
                    self.ensure_controller()?;
                }
                let result = self.desktop.action(action);
                if self.desktop.controller.is_none() {
                    self.release_controller();
                }
                self.last_action = Instant::now();
                result
            }
        }
    }
}

fn captured_geometry(
    frame: &dojo_computer::CaptureFrame,
) -> Result<niri_ipc::CapturedInputGeometry> {
    let window = frame
        .window
        .as_ref()
        .context("capture has no window identity")?;
    let compositor = frame
        .output
        .compositor
        .as_ref()
        .context("capture has no compositor identity")?;
    let transform = match frame.output.transform.as_str() {
        "Normal" => niri_ipc::Transform::Normal,
        "_90" => niri_ipc::Transform::_90,
        "_180" => niri_ipc::Transform::_180,
        "_270" => niri_ipc::Transform::_270,
        "Flipped" => niri_ipc::Transform::Flipped,
        "Flipped90" => niri_ipc::Transform::Flipped90,
        "Flipped180" => niri_ipc::Transform::Flipped180,
        "Flipped270" => niri_ipc::Transform::Flipped270,
        value => bail!("unknown captured output transform: {value}"),
    };
    Ok(niri_ipc::CapturedInputGeometry {
        window_id: window.window_id,
        pid: window.pid,
        compositor_pid: compositor.pid,
        compositor_start_time: compositor.process_start_time,
        coordination_revision: frame.coordination_revision,
        output: frame.output.name.clone(),
        output_geometry: niri_ipc::LogicalOutput {
            x: frame.output.x,
            y: frame.output.y,
            width: frame.output.width,
            height: frame.output.height,
            scale: frame.output.scale,
            transform,
        },
        rect: window.rendered_rect,
    })
}

/// Execute only the capture-bound action families implemented here. Every key
/// and button press is targeted at the exact observed window and carries the
/// coordination revision pinned by that observation. Any failed sequence
/// releases the held suffix before returning its error.
enum HeldInput {
    Button(u32),
    Key(u32),
}

trait CaptureInputSink {
    fn move_pointer(&mut self, window: u64, output: &str, x: f64, y: f64) -> Result<()>;
    fn text_key(&mut self, window: u64, code: u32, state: super::InputState) -> Result<()>;
    fn reset(&mut self) -> Result<()> {
        Ok(())
    }
    fn button(&mut self, window_id: u64, code: u32, state: super::InputState) -> Result<()>;
    fn key(&mut self, window_id: u64, code: u32, state: super::InputState) -> Result<()>;
    fn scroll(&mut self, window_id: u64, horizontal: f64, vertical: f64) -> Result<()>;
}

struct CountedInput<'a, T> {
    inner: &'a mut T,
    handled: &'a mut usize,
}
impl<T: CaptureInputSink> CaptureInputSink for CountedInput<'_, T> {
    fn move_pointer(&mut self, window: u64, output: &str, x: f64, y: f64) -> Result<()> {
        self.inner.move_pointer(window, output, x, y)
    }
    fn text_key(&mut self, window: u64, code: u32, state: super::InputState) -> Result<()> {
        self.inner.text_key(window, code, state)?;
        if state == super::InputState::Pressed {
            *self.handled += 1;
        }
        Ok(())
    }
    fn reset(&mut self) -> Result<()> {
        self.inner.reset()
    }

    fn button(&mut self, window: u64, code: u32, state: super::InputState) -> Result<()> {
        self.inner.button(window, code, state)?;
        if state == super::InputState::Pressed {
            *self.handled += 1;
        }
        Ok(())
    }
    fn key(&mut self, window: u64, code: u32, state: super::InputState) -> Result<()> {
        self.inner.key(window, code, state)?;
        if state == super::InputState::Pressed {
            *self.handled += 1;
        }
        Ok(())
    }
    fn scroll(&mut self, window: u64, horizontal: f64, vertical: f64) -> Result<()> {
        self.inner.scroll(window, horizontal, vertical)?;
        *self.handled += 1;
        Ok(())
    }
}

impl CaptureInputSink for super::AgentController {
    fn move_pointer(&mut self, window: u64, output: &str, x: f64, y: f64) -> Result<()> {
        self.move_pointer_for_window(window, output, x, y)
    }
    fn text_key(&mut self, window: u64, code: u32, state: super::InputState) -> Result<()> {
        self.text_key_for_window(window, code, state)
    }
    fn reset(&mut self) -> Result<()> {
        super::AgentController::reset(self)
    }

    fn button(&mut self, window_id: u64, code: u32, state: super::InputState) -> Result<()> {
        self.button_for_window(window_id, code, state)
    }
    fn key(&mut self, window_id: u64, code: u32, state: super::InputState) -> Result<()> {
        self.key_for_window(window_id, code, state)
    }
    fn scroll(&mut self, window_id: u64, horizontal: f64, vertical: f64) -> Result<()> {
        self.scroll_for_window(window_id, horizontal, vertical)
    }
}

fn execute_capture_input(
    controller: &mut impl CaptureInputSink,
    action: &dojo_computer::AdmittedAction,
    cancelled: &AtomicBool,
    completed_prefix: &mut usize,
) -> Result<()> {
    let mut held = Vec::<HeldInput>::new();
    let result = (|| -> Result<()> {
        match &action.input {
            CaptureInput::Move => Ok(()),
            CaptureInput::Drag { button, .. } => {
                targeted_button(
                    controller,
                    action.window_id,
                    button.evdev(),
                    super::InputState::Pressed,
                    cancelled,
                )?;
                held.push(HeldInput::Button(button.evdev()));
                for point in &action.drag_path {
                    ensure!(
                        !cancelled.load(Ordering::Acquire),
                        "desktop session disconnected during drag"
                    );
                    controller.move_pointer(action.window_id, &action.output, point.x, point.y)?;
                    *completed_prefix += 1;
                }
                controller.button(
                    action.window_id,
                    button.evdev(),
                    super::InputState::Released,
                )?;
                held.pop();
                Ok(())
            }
            CaptureInput::KeyHold { key, duration_ms } => {
                let (_, code) = normalize_key(key)?;
                ensure!(
                    !cancelled.load(Ordering::Acquire),
                    "desktop session disconnected before key hold"
                );
                controller.key(action.window_id, code, super::InputState::Pressed)?;
                held.push(HeldInput::Key(code));
                cancellable_delay(std::time::Duration::from_millis(*duration_ms), cancelled)?;
                controller.key(action.window_id, code, super::InputState::Released)?;
                held.pop();
                *completed_prefix += 1;
                Ok(())
            }
            CaptureInput::Text { text } => {
                for (code, shifted) in physical_text(text)? {
                    ensure!(
                        !cancelled.load(Ordering::Acquire),
                        "desktop session disconnected during text"
                    );
                    if shifted {
                        controller.text_key(action.window_id, 42, super::InputState::Pressed)?;
                        held.push(HeldInput::Key(42));
                    }
                    controller.text_key(action.window_id, code, super::InputState::Pressed)?;
                    held.push(HeldInput::Key(code));
                    controller.text_key(action.window_id, code, super::InputState::Released)?;
                    held.pop();
                    if shifted {
                        controller.text_key(action.window_id, 42, super::InputState::Released)?;
                        held.pop();
                    }
                    *completed_prefix += 1;
                    cancellable_delay(std::time::Duration::from_millis(8), cancelled)?;
                }
                Ok(())
            }
            CaptureInput::Click { button } => {
                targeted_button(
                    controller,
                    action.window_id,
                    button.evdev(),
                    super::InputState::Pressed,
                    cancelled,
                )?;
                held.push(HeldInput::Button(button.evdev()));
                targeted_button(
                    controller,
                    action.window_id,
                    button.evdev(),
                    super::InputState::Released,
                    cancelled,
                )?;
                held.pop();
                *completed_prefix += 1;
                Ok(())
            }
            CaptureInput::DoubleClick { button } => {
                for index in 0..2 {
                    targeted_button(
                        controller,
                        action.window_id,
                        button.evdev(),
                        super::InputState::Pressed,
                        cancelled,
                    )?;
                    held.push(HeldInput::Button(button.evdev()));
                    targeted_button(
                        controller,
                        action.window_id,
                        button.evdev(),
                        super::InputState::Released,
                        cancelled,
                    )?;
                    held.pop();
                    *completed_prefix += 1;
                    if index == 0 {
                        cancellable_delay(DOUBLE_CLICK_GAP, cancelled)?;
                        ensure!(
                            !cancelled.load(Ordering::Acquire),
                            "desktop session disconnected during double-click"
                        );
                    }
                }
                Ok(())
            }
            CaptureInput::Key { key } => {
                let (_, code) = normalize_key(key)?;
                key_tap(controller, action.window_id, code, cancelled, &mut held)?;
                *completed_prefix += 1;
                Ok(())
            }
            CaptureInput::Chord { keys } => {
                let codes = keys
                    .iter()
                    .map(|key| normalize_key(key).map(|(_, code)| code))
                    .collect::<Result<Vec<_>>>()?;
                for code in &codes {
                    ensure!(
                        !cancelled.load(Ordering::Acquire),
                        "desktop session disconnected during chord"
                    );
                    controller.key(action.window_id, *code, super::InputState::Pressed)?;
                    held.push(HeldInput::Key(*code));
                }
                for code in codes.iter().rev() {
                    ensure!(
                        !cancelled.load(Ordering::Acquire),
                        "desktop session disconnected during chord"
                    );
                    controller.key(action.window_id, *code, super::InputState::Released)?;
                    held.pop();
                }
                *completed_prefix += 1;
                Ok(())
            }
            CaptureInput::Scroll {
                horizontal,
                vertical,
            } => {
                ensure!(
                    !cancelled.load(Ordering::Acquire),
                    "desktop session disconnected before scroll"
                );
                controller.scroll(action.window_id, *horizontal, *vertical)
            }
        }
    })();
    if result.is_err() && !held.is_empty() {
        let mut cleanup_error = None;
        for input in held.into_iter().rev() {
            let release = match input {
                HeldInput::Button(code) => {
                    controller.button(action.window_id, code, super::InputState::Released)
                }
                HeldInput::Key(code) => {
                    controller.key(action.window_id, code, super::InputState::Released)
                }
            };
            if let Err(error) = release {
                cleanup_error.get_or_insert(error);
            }
        }
        // Reset bypasses stale coordination and clears held input after human takeover.
        let reset_result = controller.reset();
        if let Some(error) = cleanup_error {
            reset_result.context("reset after failed held-input release")?;
            return Err(error).context("action failed and held input cleanup also failed");
        }
        reset_result.context("reset after interrupted input")?;
    }
    result
}

fn cancellable_delay(duration: std::time::Duration, cancelled: &AtomicBool) -> Result<()> {
    let deadline = Instant::now() + duration;
    loop {
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "desktop session disconnected during input pacing"
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        std::thread::sleep(remaining.min(std::time::Duration::from_millis(5)));
    }
}

fn targeted_button(
    controller: &mut impl CaptureInputSink,
    window_id: u64,
    button: u32,
    state: super::InputState,
    cancelled: &AtomicBool,
) -> Result<()> {
    if state == super::InputState::Pressed {
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "desktop session disconnected before button press"
        );
    }
    controller.button(window_id, button, state)
}

fn key_tap(
    controller: &mut impl CaptureInputSink,
    window_id: u64,
    key: u32,
    cancelled: &AtomicBool,
    held: &mut Vec<HeldInput>,
) -> Result<()> {
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "desktop session disconnected before key press"
    );
    controller.key(window_id, key, super::InputState::Pressed)?;
    held.push(HeldInput::Key(key));
    controller.key(window_id, key, super::InputState::Released)?;
    held.pop();
    Ok(())
}

enum Event {
    Request {
        id: u64,
        message: Message,
        reply: mpsc::SyncSender<Reply>,
        cancelled: Arc<AtomicBool>,
        native_retirement: NativeRetirement,
    },
    Disconnect(u64),
    PerceptionComplete {
        id: u64,
        read: Arc<dojo_computer::PerceptionRead>,
        reply: mpsc::SyncSender<Reply>,
        result: Reply,
        arguments: Value,
    },
}

#[derive(Default)]
struct Retirements {
    native: Option<niri_ipc::socket::RetirementHandle>,
    driver: Arc<Mutex<Option<dojo_cua_worker::WorkerRetirement>>>,
}
type NativeRetirement = Arc<Mutex<Retirements>>;
fn cancel_native(cancelled: &AtomicBool, retirement: &NativeRetirement) {
    cancelled.store(true, Ordering::Release);
    let (handle, driver) = {
        let mut slots = retirement
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        (slots.native.take(), slots.driver.clone())
    };
    if let Some(handle) = handle {
        handle.retire();
    }
    if let Some(handle) = driver
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take()
    {
        let _ = handle.retire();
    }
}

fn connection(stream: UnixStream, id: u64, sender: mpsc::SyncSender<Event>) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let native_retirement: NativeRetirement = Arc::new(Mutex::new(Retirements::default()));
    let writing_cancelled = cancelled.clone();
    let writing_retirement = native_retirement.clone();
    let (outbound, responses) = mpsc::sync_channel::<mpsc::Receiver<Reply>>(1);
    // Keep reading while an effect runs, so EOF cancels dependent steps without
    // waiting for the serialized service loop to handle Disconnect.
    std::thread::spawn(move || {
        for response in responses {
            let Ok(value) = response.recv() else {
                break;
            };
            if writing_cancelled.load(Ordering::Acquire)
                || write_message(&mut writer, &value).is_err()
            {
                break;
            }
        }
        cancel_native(&writing_cancelled, &writing_retirement);
        let _ = writer.shutdown(std::net::Shutdown::Both);
    });
    let mut reader = BufReader::new(stream);
    let result = (|| -> Result<()> {
        while let Some(message) = read_message::<Message>(&mut reader)? {
            let (reply, response) = mpsc::sync_channel(1);
            outbound
                .send(response)
                .map_err(|_| anyhow::anyhow!("desktop response writer stopped"))?;
            sender.send(Event::Request {
                id,
                message,
                reply,
                cancelled: cancelled.clone(),
                native_retirement: native_retirement.clone(),
            })?;
        }
        Ok(())
    })();
    cancel_native(&cancelled, &native_retirement);
    if result.is_err() {
        // Only the response thread writes this stream. A parser-error reply
        // here could interleave with a pending image or effect receipt.
        let _ = reader.get_mut().shutdown(std::net::Shutdown::Both);
    }
    drop(outbound);
    let _ = sender.send(Event::Disconnect(id));
}

struct SocketFile(PathBuf);
impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(super) fn run(socket: PathBuf) -> Result<()> {
    let policy = super::policy::DesktopPolicy::load()?;
    let (path, lock_path) = paths(&socket)?;
    let lock: File = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let listener = UnixListener::bind(&path)?;
    let _socket_file = SocketFile(path.clone());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    // Only the lock owner verifies the installation. Bind first so startup
    // probes join this process instead of launching competing hash work.
    let perception = std::env::var_os("DOJO_PERCEPTION_INSTALLATION")
        .map(|path| {
            dojo_cua_core::capture_perception::CapturePerception::open(Path::new(&path))
                .map(Arc::new)
        })
        .transpose()?;
    let inference_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let (sender, receiver) = mpsc::sync_channel(64);
    let listener_sender = sender.clone();
    std::thread::spawn(move || {
        for (id, stream) in listener.incoming().enumerate() {
            let Ok(stream) = stream else {
                break;
            };
            let sender = listener_sender.clone();
            std::thread::spawn(move || connection(stream, id as u64, sender));
        }
    });
    let captures = Arc::new(CaptureAuthority::new()?);
    let mut sessions: HashMap<u64, Session> = HashMap::new();
    let mut empty_since = Instant::now();
    let mut pending_perception = 0usize;
    loop {
        let now = Instant::now();
        for session in sessions.values_mut() {
            session.expire(now)?;
        }
        captures.prune();
        if !sessions.is_empty() {
            empty_since = now;
        }
        if sessions.is_empty()
            && pending_perception == 0
            && now.duration_since(empty_since) >= EMPTY_GRACE
        {
            break;
        }
        let deadline = sessions
            .values()
            .filter_map(Session::next_expiry)
            .min()
            .or_else(|| {
                (sessions.is_empty() && pending_perception == 0)
                    .then_some(empty_since + EMPTY_GRACE)
            });
        let event = match deadline {
            Some(deadline) => {
                match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(event) => event,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match receiver.recv() {
                Ok(event) => event,
                Err(_) => break,
            },
        };
        match event {
            Event::PerceptionComplete {
                id,
                read,
                reply,
                result,
                arguments,
            } => {
                pending_perception = pending_perception.saturating_sub(1);
                let result = (|| -> Result<Value> {
                    let session = sessions.get(&id).context("perception session closed")?;
                    ensure!(
                        !session.cancelled.load(Ordering::Acquire),
                        "perception request cancelled"
                    );
                    session.captures.complete_perception_read(&read)?;
                    if let Some(policy) = &policy {
                        policy.authorize("parse_visual_regions", &arguments)?;
                    }
                    result.map_err(anyhow::Error::msg)
                })();
                let _ = reply.send(result.map_err(|error| format!("{error:#}")));
            }
            Event::Disconnect(id) => {
                sessions.remove(&id);
            }
            Event::Request {
                id,
                message,
                reply,
                cancelled,
                native_retirement,
            } => {
                if cancelled.load(Ordering::Acquire) {
                    sessions.remove(&id);
                    let _ = reply.send(Err("desktop session disconnected".into()));
                    continue;
                }
                if let Message::ParseVisualRegions { input } = message {
                    let prepared = (|| -> Result<_> {
                        ensure!(pending_perception == 0, "perception backend busy");
                        let session = sessions
                            .get(&id)
                            .context("initialize the session with hello first")?;
                        let arguments = serde_json::to_value(&input)?;
                        if let Some(policy) = &policy {
                            policy.authorize("parse_visual_regions", &arguments)?;
                        }
                        input
                            .0
                            .validate()
                            .map_err(|error| anyhow::anyhow!("{error}"))?;
                        let backend = perception
                            .clone()
                            .context("perception worker not installed")?;
                        let read = Arc::new(
                            session
                                .captures
                                .begin_perception_read(&input.0.capture_id)?,
                        );
                        Ok((backend, read, session.cancelled.clone(), arguments))
                    })();
                    match prepared {
                        Err(error) => {
                            let _ = reply.send(Err(format!("{error:#}")));
                        }
                        Ok((backend, read, cancellation, arguments)) => {
                            pending_perception += 1;
                            let work_read = read.clone();
                            let task = inference_runtime.spawn(async move {
                                backend
                                    .parse(&work_read, input.0, cancellation)
                                    .await
                                    .and_then(|output| {
                                        let mut value = serde_json::to_value(output)?;
                                        value["dojo_provenance"] =
                                            serde_json::to_value(work_read.frame())?;
                                        value["authorized"] = json!(false);
                                        value["capture_consumed"] = json!(false);
                                        Ok(value)
                                    })
                                    .map_err(|error| format!("{error:#}"))
                            });
                            let completion = sender.clone();
                            inference_runtime.spawn(async move {
                                let result = task.await.unwrap_or_else(|error| {
                                    Err(format!("perception executor failed: {error}"))
                                });
                                let _ = completion.send(Event::PerceptionComplete {
                                    id,
                                    read,
                                    reply,
                                    result,
                                    arguments,
                                });
                            });
                        }
                    }
                    continue;
                }
                let result = if let Some(session) = sessions.get_mut(&id) {
                    (|| -> Result<Value> {
                        if let Some(policy) = &policy {
                            if let Err(error) = authorize_message(policy, &message) {
                                return Err(error);
                            }
                        }
                        let mut result = session.handle(message)?;
                        if result.get("session_active").is_some() {
                            result["operator_policy_sha256"] =
                                json!(policy.as_ref().map(|policy| &policy.digest));
                        }
                        Ok(result)
                    })()
                } else {
                    match message {
                        Message::Hello {
                            version,
                            policy_sha256,
                            name,
                            idle_timeout_seconds,
                        } => (|| -> Result<Value> {
                            if version != VERSION {
                                bail!("unsupported desktop protocol version");
                            }
                            ensure!(
                                policy_sha256.as_ref()
                                    == policy.as_ref().map(|policy| &policy.digest),
                                "desktop policy changed during connection; reconnect"
                            );
                            validate_name(&name)?;
                            if !(1..=86400).contains(&idle_timeout_seconds) {
                                bail!("idle timeout must be between 1 and 86400 seconds");
                            }
                            ensure!(sessions.len() < 64, "desktop session limit reached");
                            sessions.insert(
                                id,
                                Session {
                                    desktop: DesktopSession {
                                        socket: Some(socket.clone()),
                                        name,
                                        controller: None,
                                    },
                                    captures: captures.session(format!("connection-{id}"))?,
                                    semantics: Default::default(),
                                    cancelled,
                                    native_retirement,
                                    driver: None,
                                    last_observation: None,
                                    idle_timeout: Duration::from_secs(idle_timeout_seconds),
                                    last_action: Instant::now(),
                                },
                            );
                            Ok(json!({"version": VERSION, "service_pid": std::process::id()}))
                        })(),
                        _ => Err(anyhow::anyhow!("initialize the session with hello first")),
                    }
                };
                let _ = reply.send(result.map_err(|error| format!("{error:#}")));
            }
        }
    }
    drop(sessions);
    Ok(())
}

#[cfg(test)]
mod capture_input_tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    enum Event {
        Button(u64, u32, dojo::agent_input::InputState),
        Key(u64, u32, dojo::agent_input::InputState),
    }

    struct FakeInput {
        events: Vec<Event>,
        fail_calls: Vec<usize>,
        calls: usize,
    }
    impl CaptureInputSink for FakeInput {
        fn move_pointer(&mut self, _window: u64, _output: &str, _x: f64, _y: f64) -> Result<()> {
            self.calls += 1;
            if self.fail_calls.contains(&self.calls) {
                bail!("injected movement failure");
            }
            Ok(())
        }
        fn text_key(
            &mut self,
            window: u64,
            code: u32,
            state: super::super::InputState,
        ) -> Result<()> {
            self.key(window, code, state)
        }

        fn button(
            &mut self,
            window: u64,
            code: u32,
            state: dojo::agent_input::InputState,
        ) -> Result<()> {
            self.record(Event::Button(window, code, state))
        }
        fn key(
            &mut self,
            window: u64,
            code: u32,
            state: dojo::agent_input::InputState,
        ) -> Result<()> {
            self.record(Event::Key(window, code, state))
        }
        fn scroll(&mut self, _window: u64, _horizontal: f64, _vertical: f64) -> Result<()> {
            Ok(())
        }
    }
    impl FakeInput {
        fn record(&mut self, event: Event) -> Result<()> {
            self.calls += 1;
            if self.fail_calls.contains(&self.calls) {
                anyhow::bail!("injected dispatch failure");
            }
            self.events.push(event);
            Ok(())
        }
    }
    fn admitted(input: CaptureInput) -> dojo_computer::AdmittedAction {
        dojo_computer::AdmittedAction {
            capture_id: "test".into(),
            output: "out".into(),
            window_id: 42,
            x: 1.,
            y: 2.,
            input,
            coordination_revision: 9,
            drag_path: Vec::new(),
            frame: serde_json::from_value(json!({"output":{"name":"out","x":0,"y":0,"width":100,"height":100,"scale":1.,"transform":"Normal","compositor":null},"logical_x":0,"logical_y":0,"logical_width":100,"logical_height":100,"window":null,"coordination_revision":9})).unwrap(),
        }
    }
    fn fake(fail_calls: impl Into<Vec<usize>>) -> FakeInput {
        FakeInput {
            events: Vec::new(),
            fail_calls: fail_calls.into(),
            calls: 0,
        }
    }

    #[test]
    fn text_failure_releases_shift_and_key_and_retains_completed_prefix() {
        use dojo::agent_input::InputState::{Pressed as P, Released as R};
        let mut input = fake(vec![3]);
        let mut completed = 0;
        assert!(
            execute_capture_input(
                &mut input,
                &admitted(CaptureInput::Text { text: "A".into() }),
                &AtomicBool::new(false),
                &mut completed
            )
            .is_err()
        );
        assert_eq!(
            input.events,
            vec![
                Event::Key(42, 42, P),
                Event::Key(42, 30, P),
                Event::Key(42, 30, R),
                Event::Key(42, 42, R)
            ]
        );
        assert_eq!(completed, 0);
    }

    #[test]
    fn failed_key_hold_release_retries_held_cleanup() {
        use dojo::agent_input::InputState::{Pressed as P, Released as R};
        let cancelled = AtomicBool::new(false);
        let mut input = fake(vec![2]);
        let mut completed = 0;
        assert!(
            execute_capture_input(
                &mut input,
                &admitted(CaptureInput::KeyHold {
                    key: "a".into(),
                    duration_ms: 1
                }),
                &cancelled,
                &mut completed
            )
            .is_err()
        );
        assert_eq!(
            input.events,
            vec![Event::Key(42, 30, P), Event::Key(42, 30, R)]
        );
        assert_eq!(completed, 0);
    }

    #[test]
    fn drag_failed_motion_releases_button() {
        use dojo::agent_input::InputState::{Pressed as P, Released as R};
        let mut input = fake(vec![2]);
        let mut action = admitted(CaptureInput::Drag {
            button: dojo_computer::MouseButton::Left,
            path: vec![],
        });
        action
            .drag_path
            .push(dojo_computer::ImagePoint { x: 10., y: 20. });
        let mut completed = 0;
        assert!(
            execute_capture_input(&mut input, &action, &AtomicBool::new(false), &mut completed)
                .is_err()
        );
        assert_eq!(
            input.events,
            vec![Event::Button(42, 272, P), Event::Button(42, 272, R)]
        );
        assert_eq!(completed, 0);
    }

    #[test]
    fn chord_targets_every_transition_and_releases_held_prefix_after_failure() {
        use dojo::agent_input::InputState::{Pressed as P, Released as R};
        let mut input = fake(vec![3, 4]);
        let mut completed = 0;
        assert!(
            execute_capture_input(
                &mut input,
                &admitted(CaptureInput::Chord {
                    keys: vec!["ctrl".into(), "shift".into(), "c".into()]
                }),
                &AtomicBool::new(false),
                &mut completed
            )
            .is_err()
        );
        assert_eq!(
            input.events,
            vec![
                Event::Key(42, 29, P),
                Event::Key(42, 42, P),
                Event::Key(42, 29, R)
            ]
        );
        assert_eq!(
            input.calls, 5,
            "cleanup must attempt every held key release"
        );
        assert_eq!(completed, 0);
    }

    #[test]
    fn handled_press_is_recorded_even_when_no_whole_chord_completed() {
        let mut input = fake(vec![2]);
        let mut completed = 0;
        let mut handled = 0;
        assert!(
            execute_capture_input(
                &mut CountedInput {
                    inner: &mut input,
                    handled: &mut handled
                },
                &admitted(CaptureInput::Chord {
                    keys: vec!["ctrl".into(), "a".into()]
                }),
                &AtomicBool::new(false),
                &mut completed
            )
            .is_err()
        );
        assert_eq!(completed, 0);
        assert_eq!(handled, 1);
        assert_eq!(
            input.events.last(),
            Some(&Event::Key(42, 29, super::super::InputState::Released))
        );
    }

    #[test]
    fn double_click_is_two_complete_targeted_clicks() {
        use dojo::agent_input::InputState::{Pressed as P, Released as R};
        let mut input = fake(Vec::<usize>::new());
        let mut completed = 0;
        execute_capture_input(
            &mut input,
            &admitted(CaptureInput::DoubleClick {
                button: dojo_computer::MouseButton::Left,
            }),
            &AtomicBool::new(false),
            &mut completed,
        )
        .unwrap();
        assert_eq!(
            input.events,
            vec![
                Event::Button(42, 272, P),
                Event::Button(42, 272, R),
                Event::Button(42, 272, P),
                Event::Button(42, 272, R)
            ]
        );
        assert_eq!(completed, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_request_guard_disconnects_while_completed_guard_preserves_stream() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut guard = CancelConnection(Some(client.try_clone().unwrap()));
        guard.complete();
        drop(guard);
        client.write_all(b"a").unwrap();
        let mut byte = [0];
        peer.read_exact(&mut byte).unwrap();
        assert_eq!(byte, *b"a");
        drop(CancelConnection(Some(client.try_clone().unwrap())));
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn socket_protocol_requires_bounded_complete_messages() {
        let valid = b"{\"operation\":\"status\"}\n";
        assert!(matches!(
            read_message::<Message>(&mut &valid[..]).unwrap(),
            Some(Message::Status {})
        ));
        assert!(read_message::<Message>(&mut &valid[..valid.len() - 1]).is_err());
        assert!(
            read_message::<Message>(&mut &b"{\"operation\":\"status\",\"extra\":1}\n"[..]).is_err()
        );
        assert!(read_message::<Message>(&mut &vec![b' '; MAX_LINE as usize + 1][..]).is_err());
        assert!(read_message::<Message>(&mut &b""[..]).unwrap().is_none());
    }

    #[test]
    fn image_replies_have_a_separate_bounded_budget() {
        let value = json!({"image": "x".repeat(MAX_LINE as usize + 10)});
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        assert!(read_message::<Value>(&mut &bytes[..]).is_err());
        assert_eq!(
            read_bounded::<Value>(&mut &bytes[..], MAX_REPLY).unwrap(),
            Some(value)
        );
        assert!(read_bounded::<Value>(&mut &bytes[..], 20).is_err());
    }

    #[test]
    fn disconnect_cancels_in_flight_work_before_serialized_disconnect_handling() {
        let (client, server) = UnixStream::pair().unwrap();
        let (sender, receiver) = mpsc::sync_channel(4);
        let thread = std::thread::spawn(move || connection(server, 1, sender));
        let mut client = client;
        write_message(&mut client, &Message::Status {}).unwrap();
        let Event::Request {
            cancelled,
            reply,
            native_retirement,
            ..
        } = receiver.recv_timeout(Duration::from_secs(1)).unwrap()
        else {
            panic!("missing request")
        };
        assert!(!cancelled.load(Ordering::Acquire));
        let path = std::env::temp_dir().join(format!(
            "dojo-retirement-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let native = niri_ipc::socket::Socket::connect_to(&path).unwrap();
        let (mut native_peer, _) = listener.accept().unwrap();
        std::fs::remove_file(path).unwrap();
        native_peer
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        native_retirement.lock().unwrap().native = Some(native.retirement_handle().unwrap());
        drop(client);
        let deadline = Instant::now() + Duration::from_secs(1);
        while !cancelled.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(cancelled.load(Ordering::Acquire));
        assert_eq!(native_peer.read(&mut [0]).unwrap(), 0);
        // The caller disappeared while the service still owns this pending reply.
        drop(reply);
        thread.join().unwrap();
    }

    #[test]
    fn capture_lease_keeps_service_alive_without_renewing_input_lease() {
        let last_action = Instant::now() - Duration::from_secs(5);
        let mut session = Session {
            desktop: DesktopSession {
                socket: None,
                name: "test".into(),
                controller: None,
            },
            captures: Arc::new(CaptureAuthority::new().unwrap())
                .session("lease".into())
                .unwrap(),
            cancelled: Arc::new(AtomicBool::new(false)),
            native_retirement: Arc::new(Mutex::new(Retirements::default())),
            driver: None,
            semantics: Default::default(),
            last_observation: Some(Instant::now()),
            idle_timeout: Duration::from_secs(2),
            last_action,
        };
        session.expire(Instant::now()).unwrap();
        assert!(session.active());
        assert!(session.desktop.controller.is_none());
        assert_eq!(session.last_action, last_action);
        session.last_observation =
            Some(Instant::now() - Duration::from_secs(CAPTURE_TTL_SECONDS + 1));
        session.expire(Instant::now()).unwrap();
        assert!(!session.active());
    }

    #[test]
    fn status_release_and_invalid_actions_do_not_renew_session() {
        let last_action = Instant::now() - Duration::from_secs(5);
        let mut session = Session {
            desktop: DesktopSession {
                socket: Some(PathBuf::from("/missing.sock")),
                name: "test".into(),
                controller: None,
            },
            captures: Arc::new(CaptureAuthority::new().unwrap())
                .session("test".into())
                .unwrap(),
            cancelled: Arc::new(AtomicBool::new(false)),
            native_retirement: Arc::new(Mutex::new(Retirements::default())),
            driver: None,
            semantics: Default::default(),
            last_observation: None,
            idle_timeout: Duration::from_secs(2),
            last_action,
        };
        assert_eq!(
            session.handle(Message::Status {}).unwrap(),
            json!({"seat_name":null,"driver_worker_pid":null,"session_active":false})
        );
        assert_eq!(
            session.handle(Message::Release {}).unwrap(),
            json!({"released":true})
        );
        assert!(
            session
                .handle(Message::Action {
                    action: DesktopAction::Key {
                        evdev: 0,
                        state: super::super::KeyState::Press
                    }
                })
                .is_err()
        );
        assert_eq!(session.last_action, last_action);
        assert!(session.desktop.controller.is_none());
    }
}
