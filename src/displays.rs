use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use niri_ipc::{
    Action, ConfiguredMode, ConfiguredPosition, Event, ModeToSet, OutputAction,
    OutputConfigChanged, Request, Response, ScaleToSet, socket::Socket,
};
use serde::{Deserialize, Serialize};

const PREVIEW_TIMEOUT: Duration = Duration::from_secs(15);
const APPLY_TIMEOUT: Duration = Duration::from_secs(5);
const IPC_TIMEOUT: Duration = Duration::from_secs(2);
const APPLY_POLL_INTERVAL: Duration = Duration::from_millis(100);
const SNAPSHOT_NOTIFY_CAPACITY: usize = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct OutputSettings {
    pub name: String,
    pub enabled: bool,
    pub position: (i32, i32),
    pub scale: f64,
    pub mode: Option<niri_ipc::Mode>,
    pub transform: niri_ipc::Transform,
}

#[derive(Clone, Debug)]
pub struct Display {
    pub output: niri_ipc::Output,
    pub settings: OutputSettings,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub displays: Vec<Display>,
    pub preview_remaining: Option<u64>,
    pub busy: bool,
    pub error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            displays: Vec::new(),
            preview_remaining: None,
            busy: false,
            error: None,
        }
    }
}

#[derive(Clone)]
pub struct Controller {
    inner: Arc<ControllerInner>,
}

struct ControllerInner {
    commands: std::sync::mpsc::Sender<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    event_stream: Option<niri_ipc::socket::RetirementHandle>,
}

impl Drop for ControllerInner {
    fn drop(&mut self) {
        if let Some(event_stream) = &self.event_stream {
            event_stream.retire();
        }
        let _ = self.commands.send(Command::Shutdown);
    }
}

impl Controller {
    pub fn start() -> anyhow::Result<(Self, async_channel::Receiver<()>)> {
        let socket = Socket::connect()?;
        socket.set_read_timeout(Some(IPC_TIMEOUT))?;
        socket.set_write_timeout(Some(IPC_TIMEOUT))?;
        let mut event_socket = Socket::connect()?;
        event_socket.set_read_timeout(None)?;
        event_socket.set_write_timeout(Some(IPC_TIMEOUT))?;
        let event_retirement = event_socket.retirement_handle()?;
        match event_socket
            .send(Request::EventStream)
            .map_err(anyhow::Error::from)?
            .map_err(anyhow::Error::msg)?
        {
            Response::Handled => {}
            _ => anyhow::bail!("Niri returned an unexpected response to EventStream"),
        }
        let config_events = config_event_stream(event_socket)?;
        Self::start_with_config_events(
            socket,
            Some(config_events),
            Some(event_retirement),
            PREVIEW_TIMEOUT,
        )
    }

    #[cfg(test)]
    fn start_with_socket(
        socket: Socket,
        preview_timeout: Duration,
    ) -> anyhow::Result<(Self, async_channel::Receiver<()>)> {
        Self::start_with_config_events(socket, None, None, preview_timeout)
    }

    fn start_with_config_events(
        socket: Socket,
        config_events: Option<std::sync::mpsc::Receiver<bool>>,
        event_stream: Option<niri_ipc::socket::RetirementHandle>,
        preview_timeout: Duration,
    ) -> anyhow::Result<(Self, async_channel::Receiver<()>)> {
        let (commands_tx, commands_rx) = std::sync::mpsc::channel();
        let worker_command_sender = commands_tx.clone();
        let (updates_tx, updates_rx) = async_channel::bounded(SNAPSHOT_NOTIFY_CAPACITY);
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let worker_snapshot = shared.clone();
        thread::Builder::new()
            .name("dojo-displays".into())
            .spawn(move || {
                Worker {
                    socket,
                    commands: commands_rx,
                    command_sender: worker_command_sender,
                    config_events,
                    updates: updates_tx,
                    snapshot: worker_snapshot,
                    preview_timeout,
                    preview: None,
                    next_preview_id: 0,
                }
                .run();
            })
            .context("starting display controller")?;
        let controller = Self {
            inner: Arc::new(ControllerInner {
                commands: commands_tx,
                snapshot: shared,
                event_stream,
            }),
        };
        controller.refresh().map_err(anyhow::Error::msg)?;
        Ok((controller, updates_rx))
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::new(
            self.inner
                .snapshot
                .lock()
                .expect("display snapshot lock poisoned")
                .clone(),
        )
    }

    pub fn refresh(&self) -> Result<(), String> {
        self.send(Command::Refresh)
    }

    pub fn preview(&self, settings: Vec<OutputSettings>) -> Result<(), String> {
        self.send(Command::Preview(settings))
    }

    pub fn keep(&self) -> Result<(), String> {
        self.send(Command::Keep)
    }

    pub fn revert(&self) -> Result<(), String> {
        self.send(Command::Revert)
    }

    pub fn close(&self) -> Result<(), String> {
        self.send(Command::Close)
    }

    fn send(&self, command: Command) -> Result<(), String> {
        self.inner
            .commands
            .send(command)
            .map_err(|_| "display controller has stopped".to_owned())
    }
}

#[derive(Debug)]
enum Command {
    Refresh,
    Preview(Vec<OutputSettings>),
    Keep,
    Revert,
    RevertExpired(u64),
    Tick,
    Close,
    Shutdown,
}

#[derive(Clone)]
struct Preview {
    id: u64,
    original: Vec<OutputSettings>,
    deadline: Instant,
}

struct Worker {
    socket: Socket,
    commands: std::sync::mpsc::Receiver<Command>,
    command_sender: std::sync::mpsc::Sender<Command>,
    config_events: Option<std::sync::mpsc::Receiver<bool>>,
    updates: async_channel::Sender<()>,
    snapshot: Arc<Mutex<Snapshot>>,
    preview_timeout: Duration,
    preview: Option<Preview>,
    next_preview_id: u64,
}

impl Worker {
    fn run(mut self) {
        loop {
            let command = match self.preview.as_ref() {
                Some(preview) => {
                    let wait = preview
                        .deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(1));
                    match self.commands.recv_timeout(wait) {
                        Ok(command) => command,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                            if Instant::now() >= preview.deadline =>
                        {
                            Command::RevertExpired(preview.id)
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Command::Tick,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Command::Close,
                    }
                }
                None => match self.commands.recv() {
                    Ok(command) => command,
                    Err(_) => Command::Close,
                },
            };
            if matches!(command, Command::Shutdown) {
                if self.preview.is_some() {
                    let _ = self.rollback_preview();
                }
                break;
            }
            let mutating = !matches!(command, Command::Refresh | Command::Tick);
            if mutating {
                self.set_busy(true);
            }
            let result = match command {
                Command::Refresh => self.refresh_snapshot(),
                Command::Preview(settings) => self.start_preview(settings),
                Command::Keep => self.keep_preview(),
                Command::Revert => self.rollback_preview(),
                Command::RevertExpired(id) => {
                    if self
                        .preview
                        .as_ref()
                        .is_some_and(|preview| preview.id == id)
                    {
                        self.rollback_preview()
                    } else {
                        Ok(())
                    }
                }
                Command::Tick => Ok(()),
                Command::Close => self.rollback_preview(),
                Command::Shutdown => unreachable!(),
            };
            if let Err(error) = result {
                self.set_error(Some(error));
            }
            if mutating {
                self.set_busy(false);
            }
            self.publish();
        }
        self.publish();
    }

    fn start_preview(&mut self, settings: Vec<OutputSettings>) -> Result<(), String> {
        if self
            .preview
            .as_ref()
            .is_some_and(|preview| Instant::now() >= preview.deadline)
        {
            self.rollback_preview()?;
            return Err("display preview expired and was reverted".into());
        }
        let current = self.query_outputs()?;
        validate_against_outputs(&settings, &current)?;
        let original = self.preview.as_ref().map_or_else(
            || current.iter().map(output_settings).collect::<Vec<_>>(),
            |preview| preview.original.clone(),
        );
        self.next_preview_id = self.next_preview_id.wrapping_add(1).max(1);
        let mut preview = Preview {
            id: self.next_preview_id,
            original,
            deadline: Instant::now() + APPLY_TIMEOUT,
        };
        self.preview = Some(preview.clone());
        self.set_preview_remaining(None);

        if let Err(error) = self
            .apply_settings(&settings, preview.deadline)
            .and_then(|()| self.refresh_snapshot())
        {
            let rollback = self.restore(&preview.original);
            self.preview = None;
            self.set_preview_remaining(None);
            return match rollback {
                Ok(()) => Err(format!("{error}; original display settings restored")),
                Err(rollback_error) => Err(format!(
                    "{error}; restoring original display settings also failed: {rollback_error}"
                )),
            };
        }
        preview.deadline = Instant::now() + self.preview_timeout;
        self.preview = Some(preview.clone());
        self.set_preview_remaining(Some(self.preview_timeout.as_secs()));
        arm_timeout(
            self.command_sender.clone(),
            preview.id,
            self.preview_timeout,
        );
        Ok(())
    }

    fn keep_preview(&mut self) -> Result<(), String> {
        let Some(preview) = self.preview.clone() else {
            return Err("there is no display preview to keep".into());
        };
        if Instant::now() >= preview.deadline {
            self.rollback_preview()?;
            return Err("display preview expired and was reverted".into());
        }
        let current = match self.query_outputs() {
            Ok(current) => current,
            Err(error) => return self.abort_keep(error),
        };
        let live_settings = current.iter().map(output_settings).collect::<Vec<_>>();
        let path = match settings_path() {
            Ok(path) => path,
            Err(error) => return self.abort_keep(error.to_string()),
        };
        let json_path = path.with_extension("json");
        let old_kdl = match read_optional_bytes(&path) {
            Ok(bytes) => bytes,
            Err(error) => return self.abort_keep(error.to_string()),
        };
        let old_json = match read_optional_bytes(&json_path) {
            Ok(bytes) => bytes,
            Err(error) => return self.abort_keep(error.to_string()),
        };
        let saved_settings = match read_saved_settings() {
            Ok(settings) => settings,
            Err(error) => return self.abort_keep(error.to_string()),
        };
        let saved = merge_saved_settings(saved_settings, &live_settings);
        let keep_result = persist_settings(&saved)
            .map_err(|error| format!("saving display settings failed: {error}"))
            .and_then(|()| self.reload_config(preview.deadline))
            .and_then(|()| self.wait_for_settings(&live_settings, preview.deadline));
        if let Err(error) = keep_result {
            let restore_kdl = restore_file(&path, old_kdl.as_deref()).err();
            let restore_json = restore_file(&json_path, old_json.as_deref()).err();
            let reload_error = self
                .reload_config(Instant::now() + self.preview_timeout)
                .err();
            let rollback_error = self.restore(&preview.original).err();
            self.preview = None;
            self.set_preview_remaining(None);
            let recovery_errors = restore_kdl
                .into_iter()
                .chain(restore_json)
                .map(|error| format!("restoring prior saved files failed: {error}"))
                .chain(
                    reload_error.map(|error| format!("reloading prior settings failed: {error}")),
                )
                .chain(
                    rollback_error.map(|error| {
                        format!("restoring the original display layout failed: {error}")
                    }),
                )
                .collect::<Vec<_>>();
            return Err(if recovery_errors.is_empty() {
                error
            } else {
                format!("{error}; {}", recovery_errors.join("; "))
            });
        }
        self.preview = None;
        self.set_preview_remaining(None);
        self.refresh_snapshot()
    }

    fn abort_keep(&mut self, error: String) -> Result<(), String> {
        match self.rollback_preview() {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(format!(
                "{error}; reverting the display preview also failed: {rollback_error}"
            )),
        }
    }

    fn rollback_preview(&mut self) -> Result<(), String> {
        let Some(preview) = self.preview.take() else {
            self.set_preview_remaining(None);
            return self.refresh_snapshot();
        };
        let result = self.restore(&preview.original);
        self.set_preview_remaining(None);
        let refresh = self.refresh_snapshot();
        result.and(refresh)
    }

    fn restore(&mut self, settings: &[OutputSettings]) -> Result<(), String> {
        self.apply_settings(settings, Instant::now() + self.preview_timeout)
    }

    fn apply_settings(
        &mut self,
        settings: &[OutputSettings],
        deadline: Instant,
    ) -> Result<(), String> {
        let outputs = self.query_outputs()?;
        let connected_settings = settings_for_connected_outputs(settings, &outputs);
        if !connected_settings.iter().any(|setting| setting.enabled) {
            return Err("at least one connected display must remain enabled".into());
        }
        let (pre_position, positions) = staged_actions(&connected_settings, &outputs)?;
        // Niri recomputes logical positions after each output mutation. Stage
        // all non-position changes first, then explicitly place every output.
        for (name, action) in pre_position {
            if Instant::now() >= deadline {
                return Err("display preview timed out during apply".into());
            }
            self.send_output_action(&name, action)?;
        }
        for (name, action) in positions {
            if Instant::now() >= deadline {
                return Err("display preview timed out during apply".into());
            }
            self.send_output_action(&name, action)?;
        }
        self.wait_for_settings(&connected_settings, deadline)
    }

    fn wait_for_settings(
        &mut self,
        settings: &[OutputSettings],
        deadline: Instant,
    ) -> Result<(), String> {
        loop {
            let outputs = self.query_outputs()?;
            if settings.iter().all(|setting| {
                outputs
                    .iter()
                    .find(|output| output.name == setting.name)
                    .is_some_and(|output| settings_match_output(setting, output))
            }) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(
                    "Niri did not apply the requested display settings before timeout".into(),
                );
            }
            thread::sleep(
                APPLY_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }

    fn send_output_action(&mut self, name: &str, action: OutputAction) -> Result<(), String> {
        match self.send(Request::Output {
            output: name.to_owned(),
            action,
        })? {
            Response::OutputConfigChanged(OutputConfigChanged::Applied) => Ok(()),
            Response::OutputConfigChanged(OutputConfigChanged::OutputWasMissing) => {
                Err(format!("display {name} is no longer connected"))
            }
            _ => Err(format!(
                "Niri returned an unexpected response for display {name}"
            )),
        }
    }

    fn reload_config(&mut self, deadline: Instant) -> Result<(), String> {
        let events = self
            .config_events
            .take()
            .ok_or_else(|| "Niri config event stream is unavailable".to_owned())?;
        while events.try_recv().is_ok() {}
        let result = (|| match self.send(Request::Action(Action::LoadConfigFile { path: None }))? {
            Response::Handled => {
                let timeout = deadline.saturating_duration_since(Instant::now());
                if timeout.is_zero() {
                    return Err("display preview expired before config reload completed".into());
                }
                match events.recv_timeout(timeout) {
                    Ok(false) => Ok(()),
                    Ok(true) => Err("Niri rejected the saved display configuration".into()),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        Err("timed out waiting for Niri config reload".into())
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        Err("Niri config event stream disconnected".into())
                    }
                }
            }
            _ => Err("Niri returned an unexpected response to config reload".into()),
        })();
        self.config_events = Some(events);
        result
    }

    fn refresh_snapshot(&mut self) -> Result<(), String> {
        let outputs = self.query_outputs()?;
        let displays = outputs
            .into_iter()
            .map(|output| {
                let settings = output_settings(&output);
                Display { output, settings }
            })
            .collect();
        self.snapshot
            .lock()
            .map_err(|_| "display snapshot lock poisoned".to_owned())?
            .displays = displays;
        self.set_error(None);
        Ok(())
    }

    fn query_outputs(&mut self) -> Result<Vec<niri_ipc::Output>, String> {
        match self.send(Request::Outputs)? {
            Response::Outputs(outputs) => {
                let mut outputs: Vec<_> = outputs.into_values().collect();
                outputs.sort_by(|left, right| left.name.cmp(&right.name));
                Ok(outputs)
            }
            _ => Err("Niri returned an unexpected response to Outputs".into()),
        }
    }

    fn send(&mut self, request: Request) -> Result<Response, String> {
        self.socket
            .send(request)
            .map_err(|error| format!("Niri IPC failed: {error}"))?
            .map_err(|error| format!("Niri rejected display request: {error}"))
    }

    fn set_busy(&mut self, busy: bool) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            snapshot.busy = busy;
        }
        self.notify();
    }

    fn set_error(&mut self, error: Option<String>) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            snapshot.error = error;
        }
        self.notify();
    }

    fn set_preview_remaining(&mut self, remaining: Option<u64>) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            snapshot.preview_remaining = remaining;
        }
        self.notify();
    }

    fn publish(&mut self) {
        if let (Some(preview), Ok(mut snapshot)) = (&self.preview, self.snapshot.lock()) {
            snapshot.preview_remaining = Some(
                preview
                    .deadline
                    .saturating_duration_since(Instant::now())
                    .as_secs(),
            );
        }
        self.notify();
    }

    fn notify(&self) {
        let _ = self.updates.try_send(());
    }
}

fn staged_actions(
    settings: &[OutputSettings],
    outputs: &[niri_ipc::Output],
) -> Result<(Vec<(String, OutputAction)>, Vec<(String, OutputAction)>), String> {
    let mut pre_position = Vec::new();
    let mut positions = Vec::new();
    for setting in settings {
        let output = outputs
            .iter()
            .find(|output| output.name == setting.name)
            .ok_or_else(|| format!("display {} disconnected during apply", setting.name))?;
        if !setting.enabled {
            if output.current_mode.is_some() {
                pre_position.push((setting.name.clone(), OutputAction::Off));
            }
            continue;
        }
        if output.current_mode.is_none() {
            pre_position.push((setting.name.clone(), OutputAction::On));
        }
        if !same_mode(setting.mode, current_mode(output)) {
            let mode = setting
                .mode
                .ok_or_else(|| format!("display {} has no selected mode", setting.name))?;
            pre_position.push((
                setting.name.clone(),
                OutputAction::Mode {
                    mode: ModeToSet::Specific(ConfiguredMode {
                        width: mode.width,
                        height: mode.height,
                        refresh: Some(f64::from(mode.refresh_rate) / 1000.0),
                    }),
                },
            ));
        }
        if output
            .logical
            .as_ref()
            .is_none_or(|logical| (logical.scale - setting.scale).abs() > f64::EPSILON)
        {
            pre_position.push((
                setting.name.clone(),
                OutputAction::Scale {
                    scale: ScaleToSet::Specific(setting.scale),
                },
            ));
        }
        if output
            .logical
            .as_ref()
            .is_none_or(|logical| logical.transform != setting.transform)
        {
            pre_position.push((
                setting.name.clone(),
                OutputAction::Transform {
                    transform: setting.transform,
                },
            ));
        }
        positions.push((
            setting.name.clone(),
            OutputAction::Position {
                position: niri_ipc::PositionToSet::Specific(ConfiguredPosition {
                    x: setting.position.0,
                    y: setting.position.1,
                }),
            },
        ));
    }
    Ok((pre_position, positions))
}

fn arm_timeout(commands: std::sync::mpsc::Sender<Command>, preview_id: u64, timeout: Duration) {
    thread::spawn(move || {
        thread::sleep(timeout);
        let _ = commands.send(Command::RevertExpired(preview_id));
    });
}

fn config_event_stream(socket: Socket) -> anyhow::Result<std::sync::mpsc::Receiver<bool>> {
    let (config_tx, config_rx) = std::sync::mpsc::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    thread::Builder::new()
        .name("dojo-display-config-events".into())
        .spawn(move || {
            let mut read_event = socket.read_events();
            let mut initialized = false;
            loop {
                match read_event() {
                    Ok(Event::ConfigLoaded { .. }) if !initialized => {
                        initialized = true;
                        let _ = ready_tx.send(Ok(()));
                    }
                    Ok(Event::ConfigLoaded { failed }) => {
                        if config_tx.send(failed).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if !initialized {
                            let _ = ready_tx.send(Err(error.to_string()));
                        }
                        break;
                    }
                }
            }
        })
        .context("starting Niri config event listener")?;
    match ready_rx.recv_timeout(IPC_TIMEOUT) {
        Ok(Ok(())) => Ok(config_rx),
        Ok(Err(error)) => anyhow::bail!("reading initial Niri config state: {error}"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            anyhow::bail!("timed out waiting for Niri initial config state")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("Niri config event listener stopped during startup")
        }
    }
}

fn output_settings(output: &niri_ipc::Output) -> OutputSettings {
    let logical = output.logical;
    OutputSettings {
        name: output.name.clone(),
        enabled: output.current_mode.is_some(),
        position: logical.map_or((0, 0), |logical| (logical.x, logical.y)),
        scale: logical.map_or(1.0, |logical| logical.scale),
        mode: current_mode(output),
        transform: logical.map_or(niri_ipc::Transform::Normal, |logical| logical.transform),
    }
}

fn current_mode(output: &niri_ipc::Output) -> Option<niri_ipc::Mode> {
    output
        .current_mode
        .and_then(|index| output.modes.get(index))
        .copied()
}

fn settings_for_connected_outputs(
    settings: &[OutputSettings],
    outputs: &[niri_ipc::Output],
) -> Vec<OutputSettings> {
    outputs
        .iter()
        .map(|output| {
            settings
                .iter()
                .find(|setting| setting.name == output.name)
                .cloned()
                .unwrap_or_else(|| output_settings(output))
        })
        .collect()
}

fn settings_match_output(settings: &OutputSettings, output: &niri_ipc::Output) -> bool {
    if !settings.enabled {
        return output.current_mode.is_none();
    }
    let Some(logical) = output.logical else {
        return false;
    };
    same_mode(settings.mode, current_mode(output))
        && (logical.x, logical.y) == settings.position
        && (logical.scale - settings.scale).abs() < 0.001
        && logical.transform == settings.transform
}

fn validate_against_outputs(
    settings: &[OutputSettings],
    outputs: &[niri_ipc::Output],
) -> Result<(), String> {
    validate(settings)?;
    for setting in settings {
        let output = outputs
            .iter()
            .find(|output| output.name == setting.name)
            .ok_or_else(|| format!("unknown display {}", setting.name))?;
        if setting.enabled {
            let mode = setting
                .mode
                .ok_or_else(|| format!("display {} has no selected mode", setting.name))?;
            if !output
                .modes
                .iter()
                .any(|supported| same_mode(Some(*supported), Some(mode)))
            {
                return Err(format!(
                    "display {} does not support that mode",
                    setting.name
                ));
            }
        }
    }
    if outputs
        .iter()
        .any(|output| !settings.iter().any(|setting| setting.name == output.name))
    {
        return Err("draft does not include every connected display".into());
    }
    Ok(())
}

fn same_mode(left: Option<niri_ipc::Mode>, right: Option<niri_ipc::Mode>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.width == right.width
                && left.height == right.height
                && left.refresh_rate == right.refresh_rate
        }
        (None, None) => true,
        _ => false,
    }
}

pub fn validate(settings: &[OutputSettings]) -> Result<(), String> {
    if settings.is_empty() || settings.iter().all(|setting| !setting.enabled) {
        return Err("at least one display must remain enabled".into());
    }
    let mut names = std::collections::HashSet::new();
    for setting in settings {
        if !names.insert(setting.name.as_str()) {
            return Err(format!("duplicate display {}", setting.name));
        }
        if !setting.scale.is_finite() || setting.scale <= 0.0 {
            return Err(format!("display {} has an invalid scale", setting.name));
        }
        if setting.enabled && setting.mode.is_none() {
            return Err(format!("display {} has no selected mode", setting.name));
        }
    }
    for left_index in 0..settings.len() {
        let left = &settings[left_index];
        if !left.enabled {
            continue;
        }
        let (left_width, left_height) = logical_size(left);
        for right in settings
            .iter()
            .skip(left_index + 1)
            .filter(|right| right.enabled)
        {
            let (right_width, right_height) = logical_size(right);
            let (left_x, left_y) = (f64::from(left.position.0), f64::from(left.position.1));
            let (right_x, right_y) = (f64::from(right.position.0), f64::from(right.position.1));
            if left_x < right_x + right_width
                && right_x < left_x + left_width
                && left_y < right_y + right_height
                && right_y < left_y + left_height
            {
                return Err(format!("displays {} and {} overlap", left.name, right.name));
            }
        }
    }
    Ok(())
}

/// The next transform when turning the monitor a quarter turn. Niri counts
/// rotation counter-clockwise, so `clockwise` steps back through that order. A
/// mirrored monitor stays mirrored.
pub fn rotated(transform: niri_ipc::Transform, clockwise: bool) -> niri_ipc::Transform {
    use niri_ipc::Transform::*;
    const NORMAL: [niri_ipc::Transform; 4] = [Normal, _90, _180, _270];
    const FLIPPED: [niri_ipc::Transform; 4] = [Flipped, Flipped90, Flipped180, Flipped270];
    let (ring, index) = match NORMAL.iter().position(|t| *t == transform) {
        Some(index) => (NORMAL, index),
        None => (
            FLIPPED,
            FLIPPED.iter().position(|t| *t == transform).unwrap_or(0),
        ),
    };
    ring[(index + if clockwise { 3 } else { 1 }) % 4]
}

/// How a monitor is turned, for display: degrees counter-clockwise, as Niri names them.
pub fn rotation_label(transform: niri_ipc::Transform) -> &'static str {
    use niri_ipc::Transform::*;
    match transform {
        Normal => "0°",
        _90 => "90°",
        _180 => "180°",
        _270 => "270°",
        Flipped => "0° mirrored",
        Flipped90 => "90° mirrored",
        Flipped180 => "180° mirrored",
        Flipped270 => "270° mirrored",
    }
}

/// Niri places outputs using rounded logical dimensions; IPC dimensions truncate.
pub fn logical_size(settings: &OutputSettings) -> (f64, f64) {
    let Some(mode) = settings.mode else {
        return (0.0, 0.0);
    };
    let (width, height) = match settings.transform {
        niri_ipc::Transform::_90
        | niri_ipc::Transform::_270
        | niri_ipc::Transform::Flipped90
        | niri_ipc::Transform::Flipped270 => (mode.height, mode.width),
        _ => (mode.width, mode.height),
    };
    (
        (f64::from(width) / settings.scale).round(),
        (f64::from(height) / settings.scale).round(),
    )
}

pub fn snapped_position(
    outputs: &[OutputSettings],
    selected: usize,
    candidate: (i32, i32),
    threshold: i32,
) -> (i32, i32) {
    let Some(selected_output) = outputs.get(selected) else {
        return candidate;
    };
    let (width, height) = logical_size(selected_output);
    let threshold = f64::from(threshold.max(0));
    let mut x = f64::from(candidate.0);
    let mut y = f64::from(candidate.1);
    let mut best_x = threshold + 1.0;
    let mut best_y = threshold + 1.0;
    for (index, output) in outputs.iter().enumerate() {
        if index == selected || !output.enabled {
            continue;
        }
        let (other_width, other_height) = logical_size(output);
        let left = f64::from(output.position.0);
        let top = f64::from(output.position.1);
        for edge in [left, left + other_width] {
            let distance = (x - edge).abs();
            if distance < best_x && distance <= threshold {
                best_x = distance;
                x = edge;
            }
        }
        for edge in [top, top + other_height] {
            let distance = (y - edge).abs();
            if distance < best_y && distance <= threshold {
                best_y = distance;
                y = edge;
            }
        }
        // Aligning opposing edges is useful for a clean monitor grid.
        for edge in [left - width, left + other_width] {
            let distance = (x - edge).abs();
            if distance < best_x && distance <= threshold {
                best_x = distance;
                x = edge;
            }
        }
        for edge in [top - height, top + other_height] {
            let distance = (y - edge).abs();
            if distance < best_y && distance <= threshold {
                best_y = distance;
                y = edge;
            }
        }
    }
    (x.round() as i32, y.round() as i32)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedSettings {
    outputs: Vec<SavedOutput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedOutput {
    name: String,
    enabled: bool,
    position: (i32, i32),
    scale: f64,
    mode: Option<SavedMode>,
    transform: niri_ipc::Transform,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedMode {
    width: u16,
    height: u16,
    refresh_rate: u32,
}

impl From<&OutputSettings> for SavedOutput {
    fn from(settings: &OutputSettings) -> Self {
        Self {
            name: settings.name.clone(),
            enabled: settings.enabled,
            position: settings.position,
            scale: settings.scale,
            mode: settings.mode.map(|mode| SavedMode {
                width: mode.width,
                height: mode.height,
                refresh_rate: mode.refresh_rate,
            }),
            transform: settings.transform,
        }
    }
}

fn saved_to_settings(saved: Vec<SavedOutput>) -> Vec<OutputSettings> {
    saved
        .into_iter()
        .map(|saved| OutputSettings {
            name: saved.name,
            enabled: saved.enabled,
            position: saved.position,
            scale: saved.scale,
            mode: saved.mode.map(|mode| niri_ipc::Mode {
                width: mode.width,
                height: mode.height,
                refresh_rate: mode.refresh_rate,
                is_preferred: false,
            }),
            transform: saved.transform,
        })
        .collect()
}

fn merge_saved_settings(
    existing: Vec<OutputSettings>,
    current: &[OutputSettings],
) -> Vec<OutputSettings> {
    let live_names = current
        .iter()
        .map(|settings| settings.name.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut merged = existing
        .into_iter()
        .filter(|settings| !live_names.contains(settings.name.as_str()))
        .collect::<Vec<_>>();
    merged.extend_from_slice(current);
    merged.sort_by(|left, right| left.name.cmp(&right.name));
    merged
}

fn settings_path() -> anyhow::Result<PathBuf> {
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .context("neither XDG_STATE_HOME nor HOME is set")?;
    anyhow::ensure!(state_home.is_absolute(), "XDG_STATE_HOME must be absolute");
    Ok(state_home.join("dojo/displays/niri.kdl"))
}

fn read_saved_settings() -> anyhow::Result<Vec<OutputSettings>> {
    let json_path = settings_path()?.with_extension("json");
    match fs::read(&json_path) {
        Ok(bytes) => {
            let saved: SavedSettings = serde_json::from_slice(&bytes).with_context(|| {
                format!("parsing saved display settings at {}", json_path.display())
            })?;
            Ok(saved_to_settings(saved.outputs))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", json_path.display())),
    }
}

fn read_optional_bytes(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match fs::read(&path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn restore_file(path: &Path, bytes: Option<&[u8]>) -> anyhow::Result<()> {
    match bytes {
        Some(bytes) => write_atomic(path, bytes),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
        },
    }
}

fn persist_settings(settings: &[OutputSettings]) -> anyhow::Result<()> {
    let path = settings_path()?;
    let json_path = path.with_extension("json");
    let saved = SavedSettings {
        outputs: settings.iter().map(SavedOutput::from).collect(),
    };
    let json = serde_json::to_vec_pretty(&saved).context("serializing saved display settings")?;
    write_atomic(&json_path, &json)?;
    let kdl = serialize_kdl(settings);
    write_atomic(&path, kdl.as_bytes())
}

fn serialize_kdl(settings: &[OutputSettings]) -> String {
    let mut out = String::from("// Generated by Dojo display settings.\n");
    for setting in settings {
        out.push_str(&format!("\noutput {} {{\n", kdl_string(&setting.name)));
        if !setting.enabled {
            out.push_str("    off\n");
        } else if let Some(mode) = setting.mode {
            out.push_str(&format!(
                "    mode {}\n",
                kdl_string(&format!(
                    "{}x{}@{:.3}",
                    mode.width,
                    mode.height,
                    f64::from(mode.refresh_rate) / 1000.0
                ))
            ));
        }
        out.push_str(&format!("    scale {}\n", setting.scale));
        out.push_str(&format!(
            "    transform {}\n",
            kdl_string(transform_kdl(setting.transform))
        ));
        out.push_str(&format!(
            "    position x={} y={}\n",
            setting.position.0, setting.position.1
        ));
        out.push_str("}\n");
    }
    out
}

fn transform_kdl(transform: niri_ipc::Transform) -> &'static str {
    match transform {
        niri_ipc::Transform::Normal => "normal",
        niri_ipc::Transform::_90 => "90",
        niri_ipc::Transform::_180 => "180",
        niri_ipc::Transform::_270 => "270",
        niri_ipc::Transform::Flipped => "flipped",
        niri_ipc::Transform::Flipped90 => "flipped-90",
        niri_ipc::Transform::Flipped180 => "flipped-180",
        niri_ipc::Transform::Flipped270 => "flipped-270",
    }
}

fn kdl_string(value: &str) -> String {
    let mut encoded = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => encoded.push_str("\\\\"),
            '"' => encoded.push_str("\\\""),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            character if character.is_control() => {
                encoded.push_str(&format!("\\u{{{:x}}}", character as u32))
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
    encoded
}

fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("display state path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)
        .with_context(|| format!("creating {}", temporary.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("writing {}", temporary.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::BufRead as _;
    use std::os::unix::net::UnixListener;

    fn setting(name: &str, position: (i32, i32), transform: niri_ipc::Transform) -> OutputSettings {
        OutputSettings {
            name: name.into(),
            enabled: true,
            position,
            scale: 1.0,
            mode: Some(niri_ipc::Mode {
                width: 1920,
                height: 1080,
                refresh_rate: 60_000,
                is_preferred: false,
            }),
            transform,
        }
    }

    #[test]
    fn scaled_drag_snaps_to_niris_rounded_placement_geometry() {
        let mut laptop = setting("eDP-1", (0, 0), niri_ipc::Transform::Normal);
        laptop.mode.as_mut().unwrap().width = 2880;
        laptop.mode.as_mut().unwrap().height = 1800;
        laptop.scale = 1.75;
        let monitor = setting("HDMI-A-1", (2880, 0), niri_ipc::Transform::Normal);
        let mut layout = vec![laptop, monitor];
        layout[1].position = snapped_position(&layout, 1, (1648, 0), 10);
        assert_eq!(layout[1].position, (1646, 0));
        assert!(validate(&layout).is_ok());
        layout[1].position.0 = 1645;
        assert!(
            validate(&layout).is_err(),
            "Niri rejects this one-pixel overlap"
        );
    }

    #[test]
    fn logical_size_accounts_for_fractional_scale_and_rotation() {
        let mut output = setting("DP-1", (0, 0), niri_ipc::Transform::_90);
        output.scale = 1.5;
        assert_eq!(logical_size(&output), (720.0, 1280.0));
        output.mode = Some(niri_ipc::Mode {
            width: 2880,
            height: 1800,
            refresh_rate: 60_000,
            is_preferred: false,
        });
        output.transform = niri_ipc::Transform::Normal;
        output.scale = 1.75;
        assert_eq!(logical_size(&output), (1646.0, 1029.0));
    }

    #[test]
    fn mode_identity_ignores_preferred_flag_for_duplicate_modes() {
        let output = test_output();
        assert!(same_mode(Some(output.modes[0]), Some(output.modes[2])));
        let mut desired = output_settings(&output);
        desired.mode = Some(output.modes[0]);
        assert!(settings_match_output(&desired, &output));
    }

    #[test]
    fn validation_requires_a_setting_for_each_connected_output() {
        let output = test_output();
        let mut second = test_output();
        second.name = "HDMI-1".into();
        let error =
            validate_against_outputs(&[output_settings(&output)], &[output, second]).unwrap_err();
        assert!(error.contains("every connected display"));
    }

    #[test]
    fn restore_ignores_disconnected_outputs_and_preserves_connected_baseline() {
        let connected = test_output();
        let mut disconnected = test_output();
        disconnected.name = "HDMI-1".into();
        disconnected.current_mode = None;
        disconnected.logical = None;
        let restored = settings_for_connected_outputs(
            &[output_settings(&connected), output_settings(&disconnected)],
            &[connected.clone()],
        );
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].name, connected.name);
        assert!(restored[0].enabled);
    }

    #[test]
    fn staging_places_all_outputs_after_every_mode_change() {
        let first = test_output();
        let mut second = test_output();
        second.name = "HDMI-1".into();
        let mut first_settings = output_settings(&first);
        let mut second_settings = output_settings(&second);
        first_settings.mode = Some(first.modes[1]);
        first_settings.position = (-1920, 0);
        second_settings.mode = Some(second.modes[1]);
        second_settings.position = (0, 0);
        let (before_positions, positions) =
            staged_actions(&[first_settings, second_settings], &[first, second]).unwrap();
        assert!(
            before_positions
                .iter()
                .all(|(_, action)| !matches!(action, OutputAction::Position { .. }))
        );
        assert_eq!(positions.len(), 2);
        assert!(
            positions
                .iter()
                .all(|(_, action)| matches!(action, OutputAction::Position { .. }))
        );
    }

    #[test]
    fn snapping_aligns_monitor_edges_within_threshold() {
        let outputs = [
            setting("DP-1", (0, 0), niri_ipc::Transform::Normal),
            setting("HDMI-1", (0, 0), niri_ipc::Transform::Normal),
        ];
        assert_eq!(snapped_position(&outputs, 1, (1926, 4), 10), (1920, 0));
    }

    #[test]
    fn validation_rejects_all_off_duplicate_and_overlapping_layouts() {
        let mut first = setting("DP-1", (0, 0), niri_ipc::Transform::Normal);
        let mut second = setting("HDMI-1", (0, 0), niri_ipc::Transform::Normal);
        assert!(
            validate(&[OutputSettings {
                enabled: false,
                ..first.clone()
            }])
            .is_err()
        );
        assert!(
            validate(&[first.clone(), first.clone()])
                .unwrap_err()
                .contains("duplicate")
        );
        assert!(
            validate(&[first.clone(), second.clone()])
                .unwrap_err()
                .contains("overlap")
        );
        second.position = (1920, 0);
        assert!(validate(&[first.clone(), second]).is_ok());
        first.scale = f64::NAN;
        assert!(validate(&[first]).unwrap_err().contains("scale"));
    }

    #[test]
    fn kdl_serialization_escapes_connector_and_keeps_refresh_scale_transform() {
        let mut output = setting(
            "DP \"one\"\\two",
            (-1920, 24),
            niri_ipc::Transform::Flipped90,
        );
        output.scale = 1.25;
        let kdl = serialize_kdl(&[output]);
        assert!(kdl.contains("output \"DP \\\"one\\\"\\\\two\""));
        assert!(kdl.contains("mode \"1920x1080@60.000\""));
        assert!(kdl.contains("scale 1.25"));
        assert!(kdl.contains("transform \"flipped-90\""));
        assert!(kdl.contains("position x=-1920 y=24"));
    }

    #[test]
    fn saved_merge_retains_disconnected_outputs() {
        let disconnected = setting("HDMI-9", (2000, 0), niri_ipc::Transform::Normal);
        let connected = setting("DP-1", (0, 0), niri_ipc::Transform::Normal);
        let merged = merge_saved_settings(vec![disconnected.clone()], &[connected.clone()]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].name, "DP-1");
        assert_eq!(merged[1].name, "HDMI-9");
    }

    fn test_output() -> niri_ipc::Output {
        niri_ipc::Output {
            name: "DP-1".into(),
            make: "Example".into(),
            model: "Panel".into(),
            serial: None,
            physical_size: None,
            modes: vec![
                niri_ipc::Mode {
                    width: 1920,
                    height: 1080,
                    refresh_rate: 60_000,
                    is_preferred: true,
                },
                niri_ipc::Mode {
                    width: 2560,
                    height: 1440,
                    refresh_rate: 144_000,
                    is_preferred: false,
                },
                niri_ipc::Mode {
                    width: 1920,
                    height: 1080,
                    refresh_rate: 60_000,
                    is_preferred: false,
                },
            ],
            current_mode: Some(2),
            is_custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(niri_ipc::LogicalOutput {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale: 1.0,
                transform: niri_ipc::Transform::Normal,
            }),
            max_bpc: Some(8),
        }
    }

    fn fake_socket(handle: impl FnOnce(std::os::unix::net::UnixStream) + Send + 'static) -> Socket {
        static NEXT_SOCKET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "dojo-display-ipc-{}-{}.sock",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let path_for_thread = path.clone();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle(stream);
            let _ = fs::remove_file(path_for_thread);
        });
        let socket = Socket::connect_to(&path).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
    }

    #[test]
    fn ipc_mode_request_preserves_resolution_and_refresh_rate() {
        let request_slot = Arc::new(Mutex::new(None));
        let request_for_server = request_slot.clone();
        let mut socket = fake_socket(move |stream| {
            let mut reader = std::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            *request_for_server.lock().unwrap() =
                Some(serde_json::from_str::<Request>(&line).unwrap());
            let reply = serde_json::to_vec(&Result::<Response, String>::Ok(
                Response::OutputConfigChanged(OutputConfigChanged::Applied),
            ))
            .unwrap();
            reader.get_mut().write_all(&reply).unwrap();
            reader.get_mut().write_all(b"\n").unwrap();
        });
        let reply = socket
            .send(Request::Output {
                output: "DP-1".into(),
                action: OutputAction::Mode {
                    mode: ModeToSet::Specific(ConfiguredMode {
                        width: 2560,
                        height: 1440,
                        refresh: Some(143.999),
                    }),
                },
            })
            .unwrap()
            .unwrap();
        assert!(matches!(
            reply,
            Response::OutputConfigChanged(OutputConfigChanged::Applied)
        ));
        let request = request_slot.lock().unwrap().take().unwrap();
        assert!(
            matches!(request, Request::Output { output, action: OutputAction::Mode { mode: ModeToSet::Specific(ConfiguredMode { width: 2560, height: 1440, refresh: Some(refresh) }) } } if output == "DP-1" && (refresh - 143.999).abs() < 0.0001)
        );
    }

    #[test]
    fn config_reload_failure_is_confirmed_by_event_stream() {
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let socket = fake_socket(move |stream| {
            let mut reader = std::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: Request = serde_json::from_str(&line).unwrap();
            assert!(matches!(
                request,
                Request::Action(Action::LoadConfigFile { path: None })
            ));
            event_tx.send(true).unwrap();
            let reply =
                serde_json::to_vec(&Result::<Response, String>::Ok(Response::Handled)).unwrap();
            reader.get_mut().write_all(&reply).unwrap();
            reader.get_mut().write_all(b"\n").unwrap();
        });
        let (command_sender, commands) = std::sync::mpsc::channel();
        let (updates, _receiver) = async_channel::bounded(1);
        let mut worker = Worker {
            socket,
            commands,
            command_sender,
            config_events: Some(event_rx),
            updates,
            snapshot: Arc::new(Mutex::new(Snapshot::default())),
            preview_timeout: PREVIEW_TIMEOUT,
            preview: None,
            next_preview_id: 0,
        };
        let error = worker
            .reload_config(Instant::now() + Duration::from_secs(1))
            .unwrap_err();
        assert!(error.contains("rejected the saved display configuration"));
    }

    #[test]
    fn confirmation_countdown_starts_after_apply_readback() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let socket = fake_socket(move |stream| {
            let mut reader = std::io::BufReader::new(stream);
            let mut queries = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let response = match serde_json::from_str::<Request>(&line).unwrap() {
                    Request::Outputs => {
                        queries += 1;
                        if queries == 4 {
                            started_tx.send(()).unwrap();
                            resume_rx.recv().unwrap();
                        }
                        Response::Outputs(HashMap::from([("DP-1".into(), test_output())]))
                    }
                    Request::Output { .. } => {
                        Response::OutputConfigChanged(OutputConfigChanged::Applied)
                    }
                    request => panic!("unexpected request {request:?}"),
                };
                reader
                    .get_mut()
                    .write_all(
                        &serde_json::to_vec(&Result::<Response, String>::Ok(response)).unwrap(),
                    )
                    .unwrap();
                reader.get_mut().write_all(b"\n").unwrap();
            }
        });
        let (controller, _updates) =
            Controller::start_with_socket(socket, PREVIEW_TIMEOUT).unwrap();
        for _ in 0..100 {
            if !controller.snapshot().displays.is_empty() && !controller.snapshot().busy {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        controller
            .preview(vec![output_settings(&test_output())])
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let applying = controller.snapshot();
        resume_tx.send(()).unwrap();
        assert!(applying.busy);
        assert_eq!(
            applying.preview_remaining, None,
            "Keep countdown is shown while Apply still blocks all buttons"
        );
        for _ in 0..100 {
            let ready = controller.snapshot();
            if !ready.busy && ready.preview_remaining.is_some() {
                assert!(
                    ready.preview_remaining.unwrap() >= 14,
                    "apply consumed the confirmation period"
                );
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("confirmation controls never became available");
    }

    #[test]
    fn background_refresh_keeps_editor_controls_available() {
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let socket = fake_socket(move |stream| {
            let mut reader = std::io::BufReader::new(stream);
            let mut count = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                assert!(matches!(
                    serde_json::from_str::<Request>(&line).unwrap(),
                    Request::Outputs
                ));
                count += 1;
                if count == 2 {
                    started_tx.send(()).unwrap();
                    resume_rx.recv().unwrap();
                }
                let reply = serde_json::to_vec(&Result::<Response, String>::Ok(Response::Outputs(
                    HashMap::from([("DP-1".into(), test_output())]),
                )))
                .unwrap();
                reader.get_mut().write_all(&reply).unwrap();
                reader.get_mut().write_all(b"\n").unwrap();
            }
        });
        let (controller, _updates) =
            Controller::start_with_socket(socket, PREVIEW_TIMEOUT).unwrap();
        for _ in 0..100 {
            if !controller.snapshot().displays.is_empty() && !controller.snapshot().busy {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        controller.refresh().unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let busy = controller.snapshot().busy;
        resume_tx.send(()).unwrap();
        assert!(
            !busy,
            "polling disables the controls and flashes the Applying label"
        );
    }

    #[test]
    fn configurable_preview_deadline_reverts_through_fake_ipc() {
        let socket = fake_socket(|stream| {
            let mut reader = std::io::BufReader::new(stream);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let request: Request = serde_json::from_str(&line).unwrap();
                let response = match request {
                    Request::Outputs => {
                        Response::Outputs(HashMap::from([("DP-1".into(), test_output())]))
                    }
                    Request::Output { .. } => {
                        Response::OutputConfigChanged(OutputConfigChanged::Applied)
                    }
                    _ => panic!("unexpected request"),
                };
                let reply = serde_json::to_vec(&Result::<Response, String>::Ok(response)).unwrap();
                reader.get_mut().write_all(&reply).unwrap();
                reader.get_mut().write_all(b"\n").unwrap();
            }
        });
        let (controller, updates) =
            Controller::start_with_socket(socket, Duration::from_millis(120)).unwrap();
        drop(updates);
        for _ in 0..50 {
            if !controller.snapshot().displays.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        controller
            .preview(vec![output_settings(&test_output())])
            .unwrap();
        let mut saw_preview = false;
        for _ in 0..50 {
            if controller.snapshot().preview_remaining.is_some() {
                saw_preview = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            saw_preview,
            "preview deadline should be visible in snapshot"
        );
        thread::sleep(Duration::from_millis(250));
        assert_eq!(controller.snapshot().preview_remaining, None);
        assert!(controller.snapshot().error.is_none());
        controller.close().unwrap();
    }

    #[test]
    fn failed_mode_apply_restores_the_original_snapshot() {
        let socket = fake_socket(|stream| {
            let mut reader = std::io::BufReader::new(stream);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let request: Request = serde_json::from_str(&line).unwrap();
                let reply: Result<Response, String> = match request {
                    Request::Outputs => Ok(Response::Outputs(HashMap::from([(
                        "DP-1".into(),
                        test_output(),
                    )]))),
                    Request::Output {
                        action: OutputAction::Mode { .. },
                        ..
                    } => Err("simulated mode rejection".into()),
                    Request::Output { .. } => {
                        Ok(Response::OutputConfigChanged(OutputConfigChanged::Applied))
                    }
                    _ => panic!("unexpected request"),
                };
                let reply = serde_json::to_vec(&reply).unwrap();
                reader.get_mut().write_all(&reply).unwrap();
                reader.get_mut().write_all(b"\n").unwrap();
            }
        });
        let (controller, updates) =
            Controller::start_with_socket(socket, Duration::from_secs(1)).unwrap();
        drop(updates);
        for _ in 0..50 {
            if !controller.snapshot().displays.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let mut requested = output_settings(&test_output());
        requested.mode = Some(test_output().modes[1]);
        controller.preview(vec![requested]).unwrap();
        for _ in 0..50 {
            if controller.snapshot().error.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let snapshot = controller.snapshot();
        assert!(
            snapshot
                .error
                .as_deref()
                .is_some_and(|error| error.contains("simulated mode rejection"))
        );
        assert_eq!(snapshot.preview_remaining, None);
        assert_eq!(
            snapshot.displays[0].settings.mode,
            Some(test_output().modes[2])
        );
        controller.close().unwrap();
    }

    fn wait_controller_settings(
        controller: &Controller,
        settings: &[OutputSettings],
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let snapshot = controller.snapshot();
            if let Some(error) = &snapshot.error {
                anyhow::bail!("display controller reported an error: {error}");
            }
            if settings.iter().all(|setting| {
                snapshot
                    .displays
                    .iter()
                    .find(|display| display.settings.name == setting.name)
                    .is_some_and(|display| settings_match_output(setting, &display.output))
            }) && !snapshot.busy
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for live display settings");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn restore_live_config_and_layout(
        controller: &Controller,
        baseline: &[OutputSettings],
        kdl_path: &Path,
        kdl_bytes: Option<&[u8]>,
        json_path: &Path,
        json_bytes: Option<&[u8]>,
    ) -> anyhow::Result<()> {
        let mut cleanup_errors = Vec::new();
        if let Err(error) = restore_file(kdl_path, kdl_bytes) {
            cleanup_errors.push(format!("restoring {}: {error:#}", kdl_path.display()));
        }
        if let Err(error) = restore_file(json_path, json_bytes) {
            cleanup_errors.push(format!("restoring {}: {error:#}", json_path.display()));
        }

        let mut event_socket = Socket::connect()?;
        event_socket.set_read_timeout(None)?;
        event_socket.set_write_timeout(Some(IPC_TIMEOUT))?;
        let retirement = event_socket.retirement_handle()?;
        match event_socket
            .send(Request::EventStream)
            .map_err(anyhow::Error::from)?
            .map_err(anyhow::Error::msg)?
        {
            Response::Handled => {}
            _ => anyhow::bail!("Niri returned an unexpected response to EventStream"),
        }
        let config_events = config_event_stream(event_socket)?;

        let mut socket = Socket::connect()?;
        socket.set_read_timeout(Some(IPC_TIMEOUT))?;
        socket.set_write_timeout(Some(IPC_TIMEOUT))?;
        match socket
            .send(Request::Action(Action::LoadConfigFile { path: None }))
            .map_err(anyhow::Error::from)?
            .map_err(anyhow::Error::msg)?
        {
            Response::Handled => {}
            _ => anyhow::bail!("Niri returned an unexpected response to config reload"),
        }
        let failed = config_events.recv_timeout(Duration::from_secs(3))?;
        anyhow::ensure!(!failed, "restored Niri config did not load");
        // Restoring an included file also triggers the watcher. Let any duplicate
        // watcher reload finish before the explicit runtime baseline is applied.
        while let Ok(failed) = config_events.recv_timeout(Duration::from_millis(750)) {
            anyhow::ensure!(!failed, "restored Niri config failed to reload");
        }

        let (command_sender, commands) = std::sync::mpsc::channel();
        let (updates, _updates_rx) = async_channel::bounded(1);
        let mut worker = Worker {
            socket,
            commands,
            command_sender,
            config_events: None,
            updates,
            snapshot: Arc::new(Mutex::new(Snapshot::default())),
            preview_timeout: PREVIEW_TIMEOUT,
            preview: None,
            next_preview_id: 0,
        };
        if let Err(error) =
            worker.apply_settings(baseline, Instant::now() + Duration::from_secs(20))
        {
            cleanup_errors.push(format!("restoring original runtime layout: {error}"));
        }
        drop(retirement);
        if let Err(error) = controller
            .refresh()
            .map_err(anyhow::Error::msg)
            .and_then(|()| wait_controller_settings(controller, baseline, Duration::from_secs(10)))
        {
            cleanup_errors.push(format!("verifying restored runtime layout: {error:#}"));
        }
        if cleanup_errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(cleanup_errors.join("; "))
        }
    }

    fn wait_persisted_settings(
        controller: &Controller,
        json_path: &Path,
        expected: &[OutputSettings],
        timeout: Duration,
    ) -> anyhow::Result<SavedSettings> {
        let deadline = Instant::now() + timeout;
        loop {
            let snapshot = controller.snapshot();
            if let Some(error) = &snapshot.error {
                anyhow::bail!("display controller reported an error: {error}");
            }
            if let Ok(bytes) = fs::read(json_path) {
                if let Ok(saved) = serde_json::from_slice::<SavedSettings>(&bytes) {
                    let settings = saved_to_settings(saved.outputs.clone());
                    let matches = expected.iter().all(|expected| {
                        settings.iter().any(|actual| {
                            actual.name == expected.name
                                && actual.enabled == expected.enabled
                                && actual.position == expected.position
                                && actual.scale == expected.scale
                                && same_mode(actual.mode, expected.mode)
                                && actual.transform == expected.transform
                        })
                    });
                    if matches && !snapshot.busy {
                        return Ok(saved);
                    }
                }
            }
            if Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for saved settings and config reload");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    #[ignore = "requires a live Niri session; changes only one output position by one logical pixel"]
    fn live_controller_preview_revert_keep_and_restore() -> anyhow::Result<()> {
        live_controller_roundtrip(false, false)
    }

    #[test]
    #[ignore = "requires live Niri; previews scaling and restores original files and layout"]
    fn live_controller_scaling_keep_and_restore() -> anyhow::Result<()> {
        live_controller_roundtrip(true, false)
    }

    #[test]
    #[ignore = "requires active laptop and HDMI outputs; previews scaled edge alignment and restores layout"]
    fn live_scaled_laptop_alignment_keep_and_restore() -> anyhow::Result<()> {
        live_controller_roundtrip(true, true)
    }

    fn live_controller_roundtrip(change_scale: bool, align_monitors: bool) -> anyhow::Result<()> {
        let (controller, _updates) = Controller::start()?;
        controller.refresh().map_err(anyhow::Error::msg)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while controller.snapshot().displays.is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        let baseline = controller
            .snapshot()
            .displays
            .iter()
            .map(|display| display.settings.clone())
            .collect::<Vec<_>>();
        anyhow::ensure!(!baseline.is_empty(), "Niri reported no connected displays");

        let path = settings_path()?;
        let json_path = path.with_extension("json");
        let original_kdl = read_optional_bytes(&path)?;
        let original_json = read_optional_bytes(&json_path)?;
        let mut candidate = None;
        if align_monitors {
            let laptop = baseline
                .iter()
                .position(|output| output.name == "eDP-1" && output.enabled)
                .context("the laptop output must be enabled")?;
            let monitor = baseline
                .iter()
                .position(|output| output.name == "HDMI-A-1" && output.enabled)
                .context("the HDMI output must be enabled")?;
            let mut layout = baseline.clone();
            layout[laptop].scale = 1.75;
            let width = logical_size(&layout[laptop]).0 as i32;
            let edge = (layout[laptop].position.0 + width, layout[laptop].position.1);
            layout[monitor].position = snapped_position(&layout, monitor, edge, 10);
            validate(&layout).map_err(anyhow::Error::msg)?;
            candidate = Some(layout);
        }
        for index in 0..baseline.len() {
            if candidate.is_some() {
                break;
            }
            if change_scale {
                if !baseline[index].enabled {
                    continue;
                }
                let mut layout = baseline.clone();
                layout[index].scale += 0.25;
                if validate(&layout).is_ok() {
                    candidate = Some(layout);
                    break;
                }
                continue;
            }
            for delta in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let mut layout = baseline.clone();
                layout[index].position.0 = layout[index].position.0.saturating_add(delta.0);
                layout[index].position.1 = layout[index].position.1.saturating_add(delta.1);
                if layout[index].position != baseline[index].position && validate(&layout).is_ok() {
                    candidate = Some(layout);
                    break;
                }
            }
            if candidate.is_some() {
                break;
            }
        }
        let target = candidate.context("no safe one-pixel output move is available")?;
        for (before, after) in baseline.iter().zip(&target) {
            anyhow::ensure!(
                before.name == after.name
                    && before.enabled == after.enabled
                    && (change_scale || before.scale == after.scale)
                    && same_mode(before.mode, after.mode)
                    && before.transform == after.transform,
                "live probe unexpectedly changed a non-position setting"
            );
        }

        let probe = (|| -> anyhow::Result<()> {
            controller
                .preview(target.clone())
                .map_err(anyhow::Error::msg)?;
            wait_controller_settings(&controller, &target, Duration::from_secs(8))?;
            controller.revert().map_err(anyhow::Error::msg)?;
            wait_controller_settings(&controller, &baseline, Duration::from_secs(8))?;

            controller
                .preview(target.clone())
                .map_err(anyhow::Error::msg)?;
            wait_controller_settings(&controller, &target, Duration::from_secs(8))?;
            controller.keep().map_err(anyhow::Error::msg)?;
            let saved =
                wait_persisted_settings(&controller, &json_path, &target, Duration::from_secs(12))?;
            let saved = saved_to_settings(saved.outputs);
            for expected in &target {
                let actual = saved
                    .iter()
                    .find(|setting| setting.name == expected.name)
                    .context("persisted output is missing")?;
                anyhow::ensure!(
                    actual.position == expected.position
                        && actual.enabled == expected.enabled
                        && actual.scale == expected.scale
                        && same_mode(actual.mode, expected.mode)
                        && actual.transform == expected.transform,
                    "persisted output settings differ from the confirmed preview"
                );
            }
            let kdl = fs::read_to_string(&path).context("reading persisted Niri output KDL")?;
            for expected in &target {
                anyhow::ensure!(
                    kdl.contains(&format!("output {} {{", kdl_string(&expected.name)))
                        && kdl.contains(&format!(
                            "position x={} y={}",
                            expected.position.0, expected.position.1
                        )),
                    "persisted KDL omitted an output's confirmed position"
                );
            }
            Ok(())
        })();

        let _ = controller.revert();
        let cleanup = restore_live_config_and_layout(
            &controller,
            &baseline,
            &path,
            original_kdl.as_deref(),
            &json_path,
            original_json.as_deref(),
        );
        let cleanup = cleanup.and_then(|()| {
            anyhow::ensure!(
                read_optional_bytes(&path)? == original_kdl,
                "cleanup did not restore the original KDL bytes"
            );
            anyhow::ensure!(
                read_optional_bytes(&json_path)? == original_json,
                "cleanup did not restore the original JSON bytes"
            );
            Ok(())
        });
        match (probe, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(probe_error), Ok(())) => Err(probe_error),
            (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
            (Err(probe_error), Err(cleanup_error)) => Err(anyhow::anyhow!(
                "live probe failed: {probe_error:#}; cleanup also failed: {cleanup_error:#}"
            )),
        }
    }

    #[test]
    fn rotating_steps_quarter_turns_and_wraps_in_both_directions() {
        use niri_ipc::Transform::*;
        assert_eq!(rotated(Normal, false), _90);
        assert_eq!(rotated(_270, false), Normal);
        assert_eq!(rotated(Normal, true), _270);
        assert_eq!(rotated(_90, true), Normal);
        let mut transform = Normal;
        for _ in 0..4 {
            transform = rotated(transform, true);
        }
        assert_eq!(transform, Normal);
    }

    #[test]
    fn rotating_a_mirrored_monitor_keeps_it_mirrored() {
        use niri_ipc::Transform::*;
        assert_eq!(rotated(Flipped, false), Flipped90);
        assert_eq!(rotated(Flipped, true), Flipped270);
        assert_eq!(rotated(Flipped270, false), Flipped);
        assert_eq!(rotation_label(Flipped90), "90° mirrored");
    }

    #[test]
    fn a_quarter_turn_swaps_the_logical_size() {
        let mut output = setting("DP-1", (0, 0), niri_ipc::Transform::Normal);
        let (width, height) = logical_size(&output);
        output.transform = rotated(output.transform, false);
        assert_eq!(logical_size(&output), (height, width));
        output.transform = rotated(output.transform, false);
        assert_eq!(logical_size(&output), (width, height));
    }
}
