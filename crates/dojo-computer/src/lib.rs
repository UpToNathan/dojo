//! Session-owned screenshot references and one-use action admission.
//!
//! CUA owns immutable PNG storage, limits, expiry and coordinate admission.
//! Dojo supplements its target with named-output geometry, process start time
//! and coordination evidence. This module performs no host input or model I/O.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use dojo_cua_primitives::capture_registry::{
    CaptureActionAdmission, CaptureActionRequest, CaptureBinding, CaptureId, CapturePublication,
    CaptureRegistryConfig, CaptureService, CaptureTarget, EncodedScreenshotDimensions,
    NativeActionDimensions, ScreenshotToActionTransform,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CONTRACT_VERSION: u32 = 3;
mod model;
pub use model::{ModelDecode, ModelProfile, ModelSource};
pub const CAPTURE_TTL_SECONDS: u64 = 30;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureScope {
    pub output: String,
    pub region: Option<Region>,
    pub window_id: Option<u64>,
    pub scale: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl CaptureScope {
    pub fn validate(&self) -> Result<f64> {
        ensure!(
            !self.output.is_empty() && self.output.len() <= 256,
            "invalid output name"
        );
        ensure!(
            self.region.is_none() || self.window_id.is_none(),
            "choose region or window_id, not both"
        );
        let scale = self.scale.unwrap_or(1.);
        ensure!(
            scale.is_finite() && (0.1..=1.).contains(&scale),
            "scale must be between 0.1 and 1"
        );
        Ok(scale)
    }
}

/// Identity evidence from the compositor and /proc, never supplied by a model.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WindowIdentity {
    pub window_id: u64,
    pub pid: u32,
    pub process_start_time: u64,
    pub app_id: Option<String>,
    pub workspace_id: u64,
    /// Full window geometry from the same rendered scene, before clipping.
    pub rendered_rect: (f64, f64, f64, f64),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OutputFrame {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub transform: String,
    pub compositor: Option<CompositorIdentity>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompositorIdentity {
    pub pid: u32,
    pub process_start_time: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CaptureFrame {
    pub output: OutputFrame,
    pub logical_x: i32,
    pub logical_y: i32,
    pub logical_width: u32,
    pub logical_height: u32,
    pub window: Option<WindowIdentity>,
    pub coordination_revision: u64,
}

impl CaptureFrame {
    fn target(&self) -> CaptureTarget {
        self.window
            .as_ref()
            .map_or(CaptureTarget::PrimaryDesktop, |window| {
                CaptureTarget::Window {
                    pid: window.pid,
                    window_id: window.window_id,
                }
            })
    }

    fn validate(&self) -> Result<()> {
        ensure!(!self.output.name.is_empty(), "capture output is missing");
        ensure!(
            self.output.scale.is_finite() && self.output.scale > 0.,
            "invalid output scale"
        );
        ensure!(
            self.logical_x >= 0
                && self.logical_y >= 0
                && self.logical_width > 0
                && self.logical_height > 0,
            "invalid capture geometry"
        );
        ensure!(
            u64::from(self.logical_x as u32) + u64::from(self.logical_width)
                <= u64::from(self.output.width)
                && u64::from(self.logical_y as u32) + u64::from(self.logical_height)
                    <= u64::from(self.output.height),
            "capture is outside output"
        );
        if let Some(window) = &self.window {
            let (x, y, w, h) = window.rendered_rect;
            ensure!(
                [x, y, w, h].iter().all(|v| v.is_finite()) && w > 0. && h > 0.,
                "rendered window geometry is invalid"
            );
            ensure!(
                self.output
                    .compositor
                    .as_ref()
                    .is_some_and(|c| c.pid > 0 && c.process_start_time > 0),
                "window capture has no compositor generation"
            );
            ensure!(
                window.pid > 0 && window.window_id > 0 && window.process_start_time > 0,
                "window identity is unavailable"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureAction {
    pub capture_id: String,
    /// Pixels in the delivered PNG, not logical desktop coordinates.
    pub x: f64,
    pub y: f64,
    pub input: CaptureInput,
    /// Request a fresh observation after the admitted action without replaying it.
    #[serde(default)]
    pub observe_after: bool,
    /// Original model prediction for provenance; never confers authority.
    #[serde(default)]
    pub model_source: Option<ModelSource>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CaptureInput {
    Move,
    /// Printable ASCII through a verified unmodified US keyboard layout.
    Text {
        text: String,
    },
    /// Hold one physical key for a bounded duration, then release it.
    KeyHold {
        key: String,
        duration_ms: u64,
    },
    /// In-window waypoints in pixels of this exact delivered image.
    Drag {
        button: MouseButton,
        path: Vec<ImagePoint>,
    },
    Click {
        button: MouseButton,
    },
    /// Two clicks with a fixed, bounded inter-click interval.
    DoubleClick {
        button: MouseButton,
    },
    /// A single physical key tap. `key` accepts CUA-style key names and aliases.
    Key {
        key: String,
    },
    /// Press modifiers and key in order, then release in reverse order.
    Chord {
        keys: Vec<String>,
    },
    Scroll {
        horizontal: f64,
        vertical: f64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImagePoint {
    pub x: f64,
    pub y: f64,
}

/// Physical US-layout typing. Control characters are deliberately excluded.
pub fn physical_text(text: &str) -> Result<Vec<(u32, bool)>> {
    ensure!(
        !text.is_empty() && text.len() <= 512,
        "text must contain 1 to 512 printable ASCII bytes"
    );
    text.bytes()
        .map(|byte| {
            ensure!(
                (32..=126).contains(&byte),
                "text requires printable ASCII; Unicode and control characters are unsupported"
            );
            let shifted = b"~!@#$%^&*()_+{}|:\"<>?";
            let plain = b"`1234567890-=[]\\;'.,/";
            let (key, shift) = if byte.is_ascii_uppercase() {
                (byte.to_ascii_lowercase(), true)
            } else if let Some(index) = shifted.iter().position(|v| *v == byte) {
                (plain[index], true)
            } else {
                (byte, false)
            };
            let code = match key {
                b' ' => 57,
                b'`' => 41,
                b'-' => 12,
                b'=' => 13,
                b'[' => 26,
                b']' => 27,
                b'\\' => 43,
                b';' => 39,
                b'\'' => 40,
                b',' => 51,
                b'.' => 52,
                b'/' => 53,
                _ => normalize_key(&(key as char).to_string())?.1,
            };
            Ok((code, shift))
        })
        .collect()
}

const MAX_CHORD_KEYS: usize = 5;

/// Resolve the deliberately small, platform-independent key-name vocabulary
/// into Linux evdev codes. Text insertion and arbitrary Unicode are not keys.
pub fn normalize_key(name: &str) -> Result<(String, u32)> {
    let normalized = name
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "");
    let canonical = match normalized.as_str() {
        "ctrl" | "control" | "ctrlleft" | "controlleft" => ("ctrl", 29),
        "shift" | "shiftleft" => ("shift", 42),
        "alt" | "option" | "altleft" | "optionleft" => ("alt", 56),
        "cmd" | "command" | "meta" | "super" | "win" | "windows" => ("super", 125),
        "enter" | "return" => ("enter", 28),
        "esc" | "escape" => ("escape", 1),
        "space" | "spacebar" => ("space", 57),
        "tab" => ("tab", 15),
        "backspace" | "back" => ("backspace", 14),
        "delete" | "del" => ("delete", 111),
        "insert" | "ins" => ("insert", 110),
        "home" => ("home", 102),
        "end" => ("end", 107),
        "pageup" | "prior" => ("pageup", 104),
        "pagedown" | "next" => ("pagedown", 109),
        "up" | "arrowup" => ("up", 103),
        "down" | "arrowdown" => ("down", 108),
        "left" | "arrowleft" => ("left", 105),
        "right" | "arrowright" => ("right", 106),
        "capslock" => ("capslock", 58),
        "f1" => ("f1", 59),
        "f2" => ("f2", 60),
        "f3" => ("f3", 61),
        "f4" => ("f4", 62),
        "f5" => ("f5", 63),
        "f6" => ("f6", 64),
        "f7" => ("f7", 65),
        "f8" => ("f8", 66),
        "f9" => ("f9", 67),
        "f10" => ("f10", 68),
        "f11" => ("f11", 87),
        "f12" => ("f12", 88),
        other if other.len() == 1 && other.as_bytes()[0].is_ascii_lowercase() => {
            let code = match other.as_bytes()[0] {
                b'a' => 30,
                b'b' => 48,
                b'c' => 46,
                b'd' => 32,
                b'e' => 18,
                b'f' => 33,
                b'g' => 34,
                b'h' => 35,
                b'i' => 23,
                b'j' => 36,
                b'k' => 37,
                b'l' => 38,
                b'm' => 50,
                b'n' => 49,
                b'o' => 24,
                b'p' => 25,
                b'q' => 16,
                b'r' => 19,
                b's' => 31,
                b't' => 20,
                b'u' => 22,
                b'v' => 47,
                b'w' => 17,
                b'x' => 45,
                b'y' => 21,
                b'z' => 44,
                _ => unreachable!(),
            };
            (other, code)
        }
        other if other.len() == 1 && other.as_bytes()[0].is_ascii_digit() => {
            let code = match other.as_bytes()[0] {
                b'1' => 2,
                b'2' => 3,
                b'3' => 4,
                b'4' => 5,
                b'5' => 6,
                b'6' => 7,
                b'7' => 8,
                b'8' => 9,
                b'9' => 10,
                b'0' => 11,
                _ => unreachable!(),
            };
            (other, code)
        }
        _ => anyhow::bail!("unsupported key name: {name}"),
    };
    Ok((canonical.0.to_owned(), canonical.1))
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    pub fn evdev(self) -> u32 {
        match self {
            Self::Left => 272,
            Self::Right => 273,
            Self::Middle => 274,
        }
    }
}

impl CaptureAction {
    pub fn validate(&self) -> Result<()> {
        if let Some(source) = &self.model_source {
            source.validate()?;
        }
        ensure!(self.capture_id.len() <= 128, "invalid capture id");
        self.capture_id
            .parse::<CaptureId>()
            .context("invalid capture id")?;
        ensure!(
            self.x.is_finite() && self.y.is_finite() && self.x >= 0. && self.y >= 0.,
            "image coordinates must be finite and nonnegative"
        );
        if let CaptureInput::Scroll {
            horizontal,
            vertical,
        } = self.input
        {
            ensure!(
                horizontal.is_finite()
                    && vertical.is_finite()
                    && horizontal.abs() <= 10000.
                    && vertical.abs() <= 10000.,
                "invalid scroll amount"
            );
        }
        match &self.input {
            CaptureInput::Text { text } => {
                physical_text(text)?;
            }
            CaptureInput::KeyHold { key, duration_ms } => {
                normalize_key(key)?;
                ensure!(
                    (1..=1000).contains(duration_ms),
                    "key hold must be 1 to 1000 milliseconds"
                );
            }
            CaptureInput::Drag { path, .. } => {
                ensure!(
                    (1..=16).contains(&path.len()),
                    "drag requires 1 to 16 waypoints"
                );
                ensure!(
                    path.iter()
                        .all(|p| p.x.is_finite() && p.y.is_finite() && p.x >= 0. && p.y >= 0.),
                    "drag coordinates must be finite and nonnegative"
                );
            }
            CaptureInput::Key { key } => {
                normalize_key(key)?;
            }
            CaptureInput::Chord { keys } => {
                ensure!(
                    (2..=MAX_CHORD_KEYS).contains(&keys.len()),
                    "chord must contain 2 to 5 keys"
                );
                let normalized = keys
                    .iter()
                    .map(|key| normalize_key(key).map(|v| v.0))
                    .collect::<Result<Vec<_>>>()?;
                let mut unique = std::collections::HashSet::new();
                ensure!(
                    normalized.iter().all(|key| unique.insert(key)),
                    "chord keys must be distinct"
                );
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Observation {
    pub contract_version: u32,
    pub capture_id: String,
    pub digest: String,
    pub acquired_at_ms: u64,
    pub expires_after_seconds: u64,
    pub image_width: u32,
    pub image_height: u32,
    pub frame: CaptureFrame,
    pub logical_x_per_image_pixel: f64,
    pub logical_y_per_image_pixel: f64,
    pub actionable: bool,
    pub coverage: String,
}

#[derive(Clone)]
struct Entry {
    binding: CaptureBinding,
    scope: CaptureScope,
    frame: CaptureFrame,
    image_width: u32,
    image_height: u32,
}

/// Shared by the actual Rust desktop process, not by disposable MCP clients.
pub struct CaptureAuthority {
    captures: CaptureService,
    entries: Mutex<HashMap<CaptureId, Entry>>,
}

impl CaptureAuthority {
    pub fn new() -> Result<Self> {
        let config = CaptureRegistryConfig {
            ttl: Duration::from_secs(CAPTURE_TTL_SECONDS),
            max_captures: 32,
            max_total_bytes: 64 * 1024 * 1024,
            max_encoded_width: 32768,
            max_encoded_height: 32768,
            max_decoded_bytes: 128 * 1024 * 1024,
        };
        Ok(Self {
            captures: CaptureService::new(config)?,
            entries: Mutex::new(HashMap::new()),
        })
    }

    pub fn session(self: &Arc<Self>, private_session_id: String) -> Result<CaptureSession> {
        let binding = self.captures.current_binding(private_session_id)?;
        Ok(CaptureSession {
            authority: self.clone(),
            binding,
        })
    }

    pub fn prune(&self) {
        self.captures.prune_expired();
        let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        entries.retain(|id, entry| {
            self.captures
                .read_for_perception(*id, &entry.binding)
                .is_ok()
        });
    }
}

pub struct CaptureSession {
    authority: Arc<CaptureAuthority>,
    binding: CaptureBinding,
}

/// Immutable pixels resolved by their owning session. This carries no input grant.
pub struct PerceptionRead {
    binding: CaptureBinding,
    id: CaptureId,
    digest: String,
    png: Arc<[u8]>,
    frame: CaptureFrame,
    width: u32,
    height: u32,
    deadline: Instant,
}

impl PerceptionRead {
    pub fn capture_id(&self) -> String {
        self.id.to_string()
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn png(&self) -> Arc<[u8]> {
        self.png.clone()
    }
    pub fn frame(&self) -> &CaptureFrame {
        &self.frame
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

pub struct AdmittedAction {
    pub capture_id: String,
    pub output: String,
    pub window_id: u64,
    pub x: f64,
    pub y: f64,
    pub input: CaptureInput,
    pub coordination_revision: u64,
    /// Validated output-relative logical drag waypoints.
    pub drag_path: Vec<ImagePoint>,
    pub frame: CaptureFrame,
}

impl CaptureSession {
    pub fn begin_perception_read(&self, capture_id: &str) -> Result<PerceptionRead> {
        let started = Instant::now();
        let id: CaptureId = capture_id.parse()?;
        let (capture, remaining) = self
            .authority
            .captures
            .read_for_perception_with_lifetime(id, &self.binding)?;
        let entries = self
            .authority
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let entry = entries.get(&id).context("capture has no target evidence")?;
        ensure!(
            entry.binding == self.binding,
            "capture belongs to another session"
        );
        Ok(PerceptionRead {
            binding: self.binding.clone(),
            id,
            digest: capture.digest().hex(),
            png: capture.png_bytes(),
            frame: entry.frame.clone(),
            width: entry.image_width,
            height: entry.image_height,
            deadline: started + remaining,
        })
    }

    /// Suppress results for a capture consumed, retired or expired during inference.
    pub fn complete_perception_read(&self, read: &PerceptionRead) -> Result<()> {
        ensure!(
            read.binding == self.binding,
            "perception belongs to another session"
        );
        ensure!(Instant::now() < read.deadline, "perception capture expired");
        let capture = self
            .authority
            .captures
            .read_for_perception(read.id, &self.binding)?;
        ensure!(
            capture.digest().hex() == read.digest,
            "perception pixels changed"
        );
        let entries = self
            .authority
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let entry = entries
            .get(&read.id)
            .context("capture has no target evidence")?;
        ensure!(
            entry.binding == self.binding && entry.frame == read.frame,
            "perception provenance changed"
        );
        Ok(())
    }

    pub fn publish(
        &self,
        scope: CaptureScope,
        frame: CaptureFrame,
        png: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Result<Observation> {
        scope.validate()?;
        frame.validate()?;
        ensure!(scope.output == frame.output.name, "capture output mismatch");
        ensure!(
            scope.window_id == frame.window.as_ref().map(|w| w.window_id),
            "capture window mismatch"
        );
        let transform = ScreenshotToActionTransform::new(
            f64::from(frame.logical_width) / f64::from(width),
            0.,
            0.,
            f64::from(frame.logical_height) / f64::from(height),
            f64::from(frame.logical_x),
            f64::from(frame.logical_y),
        )?;
        self.authority.prune();
        let id = self.authority.captures.publish(CapturePublication {
            png_bytes: png,
            target: frame.target(),
            encoded_dimensions: EncodedScreenshotDimensions::new(width, height)?,
            native_action_dimensions: NativeActionDimensions::new(
                frame.output.width,
                frame.output.height,
            )?,
            screenshot_to_action: transform,
            session_id: self.binding.session_id().into(),
            session_generation: self.binding.session_generation(),
        })?;
        let digest = self
            .authority
            .captures
            .read_for_perception(id, &self.binding)?
            .digest()
            .hex();
        let observation = Observation {
            contract_version: CONTRACT_VERSION,
            capture_id: id.to_string(),
            digest,
            acquired_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_millis()
                .try_into()?,
            expires_after_seconds: CAPTURE_TTL_SECONDS,
            image_width: width,
            image_height: height,
            logical_x_per_image_pixel: f64::from(frame.logical_width) / f64::from(width),
            logical_y_per_image_pixel: f64::from(frame.logical_height) / f64::from(height),
            actionable: frame.window.is_some(),
            frame: frame.clone(),
            coverage: "visible composited pixels; window crops may include occluding surfaces"
                .into(),
        };
        self.authority
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                id,
                Entry {
                    binding: self.binding.clone(),
                    scope,
                    frame,
                    image_width: width,
                    image_height: height,
                },
            );
        Ok(observation)
    }

    /// Non-consuming lookup used to obtain the server-owned scope for revalidation.
    pub fn scope(&self, id: &str) -> Result<CaptureScope> {
        let id: CaptureId = id.parse()?;
        self.authority
            .captures
            .read_for_perception(id, &self.binding)?;
        let entries = self
            .authority
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let entry = entries.get(&id).context("capture has no target evidence")?;
        ensure!(
            entry.binding == self.binding,
            "capture belongs to another session"
        );
        Ok(entry.scope.clone())
    }

    /// Consume an exact window capture for a non-pointer operation. This reserves
    /// target authority only; it dispatches no move or other desktop effect.
    pub fn admit_target(
        &self,
        capture_id: String,
        current: &CaptureFrame,
    ) -> Result<AdmittedAction> {
        self.admit(
            CaptureAction {
                capture_id,
                x: 0.0,
                y: 0.0,
                input: CaptureInput::Move,
                observe_after: false,
                model_source: None,
            },
            current,
        )
    }

    /// Consumes only after all static/live checks pass. The admitted reference is
    /// never restored after a pause, cancellation or uncertain native effect.
    pub fn admit(&self, action: CaptureAction, current: &CaptureFrame) -> Result<AdmittedAction> {
        action.validate()?;
        current.validate()?;
        let id: CaptureId = action.capture_id.parse()?;
        let capture = self
            .authority
            .captures
            .read_for_perception(id, &self.binding)?;
        if let Some(source) = &action.model_source {
            ensure!(
                source.capture_digest == capture.digest().hex(),
                "prediction refers to different captured pixels"
            );
            ensure!(
                source.parser_version == dojo_computer_models::PARSER_VERSION,
                "prediction parser version changed"
            );
        }
        let entry = self
            .authority
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .cloned()
            .context("capture has no target evidence")?;
        ensure!(
            entry.binding == self.binding,
            "capture belongs to another session"
        );
        ensure!(
            entry.frame == *current,
            "capture geometry, identity or ownership changed; observe again"
        );
        let window = current
            .window
            .as_ref()
            .context("capture is read-only: observe an exact positioned window before acting")?;
        let drag_path = if let CaptureInput::Drag { path, .. } = &action.input {
            path.iter()
                .map(|point| {
                    ensure!(
                        point.x < f64::from(entry.image_width)
                            && point.y < f64::from(entry.image_height),
                        "drag point lies outside captured image"
                    );
                    Ok(ImagePoint {
                        x: f64::from(current.logical_x)
                            + point.x * f64::from(current.logical_width)
                                / f64::from(entry.image_width),
                        y: f64::from(current.logical_y)
                            + point.y * f64::from(current.logical_height)
                                / f64::from(entry.image_height),
                    })
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let CaptureActionAdmission {
            action_x, action_y, ..
        } = self.authority.captures.admit_action(CaptureActionRequest {
            capture_id: id,
            binding: self.binding.clone(),
            target: current.target(),
            current_native_action_dimensions: NativeActionDimensions::new(
                current.output.width,
                current.output.height,
            )?,
            screenshot_x: action.x,
            screenshot_y: action.y,
        })?;
        self.authority
            .entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
        Ok(AdmittedAction {
            capture_id: action.capture_id,
            output: current.output.name.clone(),
            window_id: window.window_id,
            x: action_x,
            y: action_y,
            input: action.input,
            coordination_revision: current.coordination_revision,
            drag_path,
            frame: current.clone(),
        })
    }

    pub fn retire(&mut self) -> Result<()> {
        self.authority.captures.retire_session(&self.binding);
        self.authority.prune();
        self.binding = self
            .authority
            .captures
            .current_binding(self.binding.session_id())?;
        Ok(())
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        self.authority.captures.retire_session(&self.binding);
        self.authority.prune();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn frame() -> CaptureFrame {
        CaptureFrame {
            output: OutputFrame {
                name: "left".into(),
                x: -800,
                y: 100,
                width: 800,
                height: 600,
                scale: 1.75,
                transform: "Normal".into(),
                compositor: Some(CompositorIdentity {
                    pid: 10,
                    process_start_time: 123,
                }),
            },
            logical_x: 100,
            logical_y: 80,
            logical_width: 400,
            logical_height: 200,
            window: Some(WindowIdentity {
                window_id: 42,
                pid: 100,
                process_start_time: 123,
                app_id: Some("fixture".into()),
                workspace_id: 1,
                rendered_rect: (100., 80., 400., 200.),
            }),
            coordination_revision: 7,
        }
    }
    fn scope() -> CaptureScope {
        CaptureScope {
            output: "left".into(),
            region: None,
            window_id: Some(42),
            scale: Some(0.5),
        }
    }
    fn png() -> Vec<u8> {
        let mut result = Vec::new();
        image::codecs::png::PngEncoder::new(&mut result)
            .write_image(
                &vec![255; 200 * 100 * 4],
                200,
                100,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        result
    }
    fn action(id: String) -> CaptureAction {
        CaptureAction {
            capture_id: id,
            observe_after: false,
            model_source: None,
            x: 25.,
            y: 30.,
            input: CaptureInput::Click {
                button: MouseButton::Left,
            },
        }
    }
    #[test]
    fn capture_maps_image_pixels_and_is_consumed_once() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let session = authority.session("one".into()).unwrap();
        let observation = session.publish(scope(), frame(), png(), 200, 100).unwrap();
        assert!(observation.actionable);
        assert_eq!(observation.digest.len(), 64);
        let admitted = session
            .admit(action(observation.capture_id.clone()), &frame())
            .unwrap();
        assert_eq!((admitted.x, admitted.y), (150., 140.));
        assert!(
            session
                .admit(action(observation.capture_id), &frame())
                .is_err()
        );
    }

    #[test]
    fn perception_reads_preserve_capture_and_refuse_late_completion() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let session = authority.session("one".into()).unwrap();
        let other = authority.session("two".into()).unwrap();
        let observation = session.publish(scope(), frame(), png(), 200, 100).unwrap();
        let read = session
            .begin_perception_read(&observation.capture_id)
            .unwrap();
        let again = session
            .begin_perception_read(&observation.capture_id)
            .unwrap();
        assert!(Arc::ptr_eq(&read.png(), &again.png()));
        assert_eq!(read.digest(), observation.digest);
        assert_eq!(read.dimensions(), (200, 100));
        assert_eq!(read.frame(), &frame());
        assert!(again.deadline() <= read.deadline() + Duration::from_millis(1));
        assert!(
            other
                .begin_perception_read(&observation.capture_id)
                .is_err()
        );
        assert!(other.complete_perception_read(&read).is_err());
        session.complete_perception_read(&read).unwrap();
        session
            .admit(action(observation.capture_id), &frame())
            .unwrap();
        assert!(session.complete_perception_read(&read).is_err());
        assert!(!read.png().is_empty());
    }

    #[test]
    fn perception_retirement_and_deadline_suppress_results() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let mut session = authority.session("one".into()).unwrap();
        let observation = session.publish(scope(), frame(), png(), 200, 100).unwrap();
        let mut read = session
            .begin_perception_read(&observation.capture_id)
            .unwrap();
        read.deadline = Instant::now();
        assert!(session.complete_perception_read(&read).is_err());
        read.deadline = Instant::now() + Duration::from_secs(10);
        session.retire().unwrap();
        assert!(session.complete_perception_read(&read).is_err());
    }

    #[test]
    fn model_provenance_binds_pixels_and_parser_before_capture_consumption() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let session = authority.session("one".into()).unwrap();
        let observation = session.publish(scope(), frame(), png(), 200, 100).unwrap();
        let mut input = action(observation.capture_id.clone());
        input.model_source = Some(ModelSource {
            profile: ModelProfile::UiTars,
            parser_version: dojo_computer_models::PARSER_VERSION.into(),
            raw: "click()".into(),
            capture_digest: "0".repeat(64),
            call_id: None,
        });
        assert!(
            session
                .admit(input.clone(), &frame())
                .err()
                .unwrap()
                .to_string()
                .contains("different captured pixels")
        );
        input.model_source.as_mut().unwrap().capture_digest = observation.digest;
        input.model_source.as_mut().unwrap().parser_version = "old".into();
        assert!(
            session
                .admit(input.clone(), &frame())
                .err()
                .unwrap()
                .to_string()
                .contains("parser version changed")
        );
        input.model_source.as_mut().unwrap().parser_version =
            dojo_computer_models::PARSER_VERSION.into();
        assert!(session.admit(input, &frame()).is_ok());
    }

    #[test]
    fn model_decode_uses_retained_view_and_does_not_grant_or_consume_input() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let one = authority.session("one".into()).unwrap();
        let two = authority.session("two".into()).unwrap();
        let observation = one.publish(scope(), frame(), png(), 200, 100).unwrap();
        let request = || ModelDecode {
            capture_id: observation.capture_id.clone(),
            profile: ModelProfile::UiTars,
            raw: "Action: click(start_box='(500,500)')".into(),
        };
        let decoded = one.decode_model(request()).unwrap();
        assert_eq!(
            decoded["outcome"]["Action"]["Click"]["at"]["delivered_x"],
            100.0
        );
        assert_eq!(
            decoded["outcome"]["Action"]["Click"]["at"]["delivered_y"],
            50.0
        );
        assert_eq!(decoded["authorized"], false);
        assert!(two.decode_model(request()).is_err());
        one.admit(action(observation.capture_id.clone()), &frame())
            .unwrap();
        assert!(one.decode_model(request()).is_err());
    }
    #[test]
    fn failed_validation_preserves_valid_capture_but_never_dispatches() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let session = authority.session("one".into()).unwrap();
        let observation = session.publish(scope(), frame(), png(), 200, 100).unwrap();
        let mut invalid = action(observation.capture_id.clone());
        invalid.x = 200.;
        assert!(session.admit(invalid, &frame()).is_err());
        let mut changed = frame();
        changed.coordination_revision += 1;
        assert!(
            session
                .admit(action(observation.capture_id.clone()), &changed)
                .is_err()
        );
        assert!(
            session
                .admit(action(observation.capture_id), &frame())
                .is_ok()
        );
    }
    #[test]
    fn output_crop_process_identity_and_geometry_are_not_primary_desktop_aliases() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let session = authority.session("one".into()).unwrap();
        let observation = session.publish(scope(), frame(), png(), 200, 100).unwrap();
        for changed in [
            {
                let mut f = frame();
                f.output.name = "right".into();
                f
            },
            {
                let mut f = frame();
                f.output.x += 1;
                f
            },
            {
                let mut f = frame();
                f.logical_x += 1;
                f
            },
            {
                let mut f = frame();
                f.window.as_mut().unwrap().process_start_time += 1;
                f
            },
        ] {
            assert!(
                session
                    .admit(action(observation.capture_id.clone()), &changed)
                    .is_err()
            );
        }
    }
    #[test]
    fn another_session_release_and_restart_cannot_reuse_capture() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let mut one = authority.session("one".into()).unwrap();
        let two = authority.session("two".into()).unwrap();
        let observation = one.publish(scope(), frame(), png(), 200, 100).unwrap();
        assert!(two.scope(&observation.capture_id).is_err());
        assert!(
            two.admit(action(observation.capture_id.clone()), &frame())
                .is_err()
        );
        one.retire().unwrap();
        assert!(one.scope(&observation.capture_id).is_err());
        let restarted = Arc::new(CaptureAuthority::new().unwrap());
        assert!(
            restarted
                .session("one".into())
                .unwrap()
                .scope(&observation.capture_id)
                .is_err()
        );
    }
    #[test]
    fn unbound_output_observation_cannot_authorize_an_input_target() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let session = authority.session("one".into()).unwrap();
        let mut unbound = frame();
        unbound.window = None;
        let mut scope = scope();
        scope.window_id = None;
        let observation = session
            .publish(scope, unbound.clone(), png(), 200, 100)
            .unwrap();
        assert!(!observation.actionable);
        assert!(
            session
                .admit(action(observation.capture_id), &unbound)
                .is_err()
        );
    }
    #[test]
    fn contract_rejects_extra_fields_and_nonfinite_or_missing_points() {
        assert!(serde_json::from_str::<CaptureAction>(r#"{"capture_id":"x","x":0,"y":0,"input":{"kind":"click","button":"left","extra":1}}"#).is_err());
        assert!(
            serde_json::from_str::<CaptureAction>(
                r#"{"capture_id":"x","y":0,"input":{"kind":"move"}}"#
            )
            .is_err()
        );
        let mut invalid = action("x".into());
        invalid.x = f64::NAN;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn key_names_normalize_cua_aliases_without_unicode_typing() {
        assert_eq!(normalize_key("Control").unwrap(), ("ctrl".into(), 29));
        assert_eq!(
            normalize_key("Page_Down").unwrap(),
            ("pagedown".into(), 109)
        );
        assert_eq!(normalize_key("ArrowLeft").unwrap(), ("left".into(), 105));
        assert_eq!(normalize_key("A").unwrap(), ("a".into(), 30));
        assert!(normalize_key("é").is_err());
        assert!(normalize_key("unmapped-key").is_err());
    }

    #[test]
    fn physical_text_maps_shift_and_punctuation_and_refuses_control_or_unicode() {
        assert_eq!(
            physical_text("Az!? ").unwrap(),
            vec![(30, true), (44, false), (2, true), (53, true), (57, false)]
        );
        for text in ["", "é", "a\n", "\t", "\0"] {
            assert!(physical_text(text).is_err());
        }
        assert!(physical_text(&"a".repeat(513)).is_err());
    }

    #[test]
    fn drag_maps_every_point_and_rejects_outside_without_consuming_capture() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let session = authority.session("one".into()).unwrap();
        let observation = session.publish(scope(), frame(), png(), 200, 100).unwrap();
        let mut drag = action(observation.capture_id.clone());
        drag.input = CaptureInput::Drag {
            button: MouseButton::Left,
            path: vec![ImagePoint { x: 200., y: 0. }],
        };
        assert!(session.admit(drag.clone(), &frame()).is_err());
        drag.input = CaptureInput::Drag {
            button: MouseButton::Left,
            path: vec![ImagePoint { x: 150., y: 50. }],
        };
        let admitted = session.admit(drag, &frame()).unwrap();
        assert_eq!(
            (admitted.drag_path[0].x, admitted.drag_path[0].y),
            (400., 180.)
        );
        assert!(
            session
                .admit(action(observation.capture_id), &frame())
                .is_err()
        );
    }

    #[test]
    fn chord_contract_is_bounded_and_rejects_duplicate_keys() {
        let authority = Arc::new(CaptureAuthority::new().unwrap());
        let session = authority.session("one".into()).unwrap();
        let observation = session.publish(scope(), frame(), png(), 200, 100).unwrap();
        let mut chord = action(observation.capture_id.clone());
        chord.input = CaptureInput::Chord {
            keys: vec!["ctrl".into(), "c".into()],
        };
        assert!(chord.validate().is_ok());
        chord.input = CaptureInput::Chord {
            keys: vec!["ctrl".into(), "control".into()],
        };
        assert!(chord.validate().is_err());
        chord.input = CaptureInput::Chord {
            keys: vec!["ctrl".into(); MAX_CHORD_KEYS + 1],
        };
        assert!(chord.validate().is_err());
        chord.input = CaptureInput::Key {
            key: "command".into(),
        };
        assert!(chord.validate().is_ok());
    }
}
