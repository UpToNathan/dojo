//! Capture-correlated Niri scene reads behind the original Linux image tools.
use super::{connection, process_start, Connection};
use crate::x11::WindowInfo;
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use niri_ipc::{CapturedScene, LogicalOutput, Request, Response};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::time::{Duration, Instant};

const MAX_PNG_BYTES: usize = 32 * 1024 * 1024;
const MAX_PIXELS: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct CaptureEvidence {
    pub coordination_revision: Option<u64>,
    pub compositor_pid: u32,
    pub compositor_uid: u32,
    pub compositor_start_ticks: u64,
    pub scene_frame_serial: u64,
    pub output: String,
    pub logical_output: LogicalOutput,
    pub source_width: u32,
    pub source_height: u32,
    pub source_png_sha256: String,
    pub image_semantic_atomic: bool,
    pub image_geometry_atomic: bool,
    pub capture_semantics: &'static str,
    pub rendered_windows: Vec<niri_ipc::CapturedWindow>,
    pub window: Option<WindowEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowEvidence {
    pub window_id: u64,
    pub pid: u32,
    pub process_start_ticks: u64,
    pub rendered_rect: (f64, f64, f64, f64),
}

pub struct CapturedFrame {
    pub png: Vec<u8>,
    pub evidence: CaptureEvidence,
    pub windows: Vec<WindowInfo>,
}

pub fn window(window_id: u64, pid: Option<u32>) -> Result<CapturedFrame> {
    capture_from_connection(&mut connection()?, Some((window_id, pid)))
}

pub fn desktop() -> Result<CapturedFrame> {
    capture_from_connection(&mut connection()?, None)
}

pub fn window_for_input(window_id: u64, pid: Option<u32>) -> Result<CapturedFrame> {
    capture_scoped(&mut connection()?, Some((window_id, pid)), true)
}

pub fn desktop_for_input() -> Result<CapturedFrame> {
    capture_scoped(&mut connection()?, None, true)
}

fn capture_from_connection(
    connection: &mut Connection,
    target: Option<(u64, Option<u32>)>,
) -> Result<CapturedFrame> {
    capture_scoped(connection, target, false)
}

fn capture_scoped(
    connection: &mut Connection,
    target: Option<(u64, Option<u32>)>,
    for_input: bool,
) -> Result<CapturedFrame> {
    connection.deadline = Instant::now() + Duration::from_secs(5);
    let Response::Outputs(outputs) = connection.query(&Request::Outputs)? else {
        bail!("unexpected Niri outputs reply")
    };
    let enabled: Vec<_> = outputs
        .into_iter()
        .filter(|(_, output)| output.logical.is_some())
        .collect();
    let mut selected_inventory = None;
    let selected = if enabled.len() == 1 { 0 } else {
        let (id, _) = target.context("facility_unavailable: Niri desktop capture currently requires exactly one active output")?;
        let Response::Windows(windows) = connection.query(&Request::Windows)? else { bail!("unexpected Niri windows reply") };
        let window = windows.iter().find(|window| window.id == id).context("surface_identity_unproven: Niri target window is absent")?;
        let Response::Workspaces(workspaces) = connection.query(&Request::Workspaces)? else { bail!("unexpected Niri workspaces reply") };
        let name = workspaces.iter().find(|workspace| Some(workspace.id) == window.workspace_id)
            .and_then(|workspace| workspace.output.as_deref()).context("target workspace has no output")?;
        let index = enabled.iter().position(|(output, _)| output == name).context("target output is disabled")?;
        selected_inventory = Some(windows);
        index
    };
    let (output_name, output) = &enabled[selected];
    ensure!(
        output_name == &output.name && output.current_mode.is_some(),
        "invalid Niri capture output identity"
    );
    let logical = output.logical.context("capture output is disabled")?;
    ensure!(
        logical.width > 0 && logical.height > 0 && logical.scale.is_finite() && logical.scale > 0.0,
        "invalid Niri capture output geometry"
    );
    ensure!(
        logical.width <= 32768
            && logical.height <= 32768
            && u64::from(logical.width) * u64::from(logical.height) <= MAX_PIXELS,
        "logical capture frame exceeds image budget"
    );
    let inventory = match selected_inventory {
        Some(windows) => windows,
        None => { let Response::Windows(windows) = connection.query(&Request::Windows)? else { bail!("unexpected Niri windows reply") }; windows }
    };
    let target_identity = if let Some((id, requested_pid)) = target {
        let matches: Vec<_> = inventory.iter().filter(|window| window.id == id).collect();
        ensure!(
            matches.len() == 1,
            "surface_identity_unproven: Niri target window is absent or ambiguous"
        );
        let native = matches[0];
        let pid = u32::try_from(
            native
                .pid
                .context("surface_identity_unproven: Niri target pid is unavailable")?,
        )?;
        ensure!(
            pid > 0 && requested_pid.is_none_or(|expected| expected == pid),
            "surface_identity_unproven: Niri target process mismatch"
        );
        Some((id, pid, process_start(pid)?))
    } else {
        None
    };
    let coordination_revision = if for_input {
        let Response::InputCoordination(state) = connection.query(&Request::InputCoordination)?
        else {
            bail!("facility_unavailable: native input coordination is unavailable");
        };
        Some(state.revision)
    } else {
        None
    };
    let Response::CapturedScene(scene) = connection.query(&Request::CaptureScene {
        output: output_name.clone(),
    })?
    else {
        bail!("compositor does not provide capture-correlated scene geometry")
    };
    if let Some(revision) = coordination_revision {
        let Response::InputCoordination(state) = connection.query(&Request::InputCoordination)?
        else {
            bail!("facility_unavailable: native input coordination is unavailable");
        };
        ensure!(
            state.revision == revision,
            "capture ownership changed during observation"
        );
    }
    validate_scene(&scene, connection, output_name, logical)?;
    let png = STANDARD.decode(&scene.png_base64)?;
    ensure!(
        png.len() <= MAX_PNG_BYTES,
        "scene exceeds encoded image budget"
    );
    ensure!(
        png_dimensions(&png)? == (scene.width, scene.height),
        "scene PNG dimensions mismatch"
    );
    let mut seen = std::collections::HashSet::new();
    let mut windows = Vec::new();
    for placement in &scene.windows {
        if target.is_some_and(|(id, _)| id != placement.id) {
            continue;
        }
        ensure!(
            seen.insert(placement.id),
            "duplicate captured Niri window identity"
        );
        let matches: Vec<_> = inventory
            .iter()
            .filter(|window| window.id == placement.id && window.pid == placement.pid)
            .collect();
        ensure!(matches.len() == 1, "captured Niri window identity changed");
        let native = matches[0];
        let (x, y, width, height) = placement.rect;
        windows.push(WindowInfo {
            xid: placement.id,
            pid: native.pid.map(u32::try_from).transpose()?,
            app_name: native.app_id.clone().unwrap_or_default(),
            title: native.title.clone().unwrap_or_default(),
            is_on_screen: true,
            z_index: None,
            x: super::exact_integer(x.floor())?,
            y: super::exact_integer(y.floor())?,
            width: u32::try_from(super::exact_integer(width.ceil())?)?,
            height: u32::try_from(super::exact_integer(height.ceil())?)?,
        });
    }
    let window = if let Some((id, pid, start)) = target_identity {
        let placement = scene
            .windows
            .iter()
            .find(|window| window.id == id && window.pid == Some(pid as i32))
            .context("surface_identity_unproven: target is not visible in the captured scene")?;
        ensure!(
            !placement.blocked_out,
            "surface_identity_unproven: window is excluded by screen-capture privacy rules"
        );
        let (x, y, width, height) = placement.rect;
        ensure!(
            placement.rect.0 == placement.output_clip.0
                && placement.rect.1 == placement.output_clip.1
                && ((placement.rect.0 + placement.rect.2) - (placement.output_clip.0 + placement.output_clip.2)).abs() <= 1e-9
                && ((placement.rect.1 + placement.rect.3) - (placement.output_clip.1 + placement.output_clip.3)).abs() <= 1e-9
                && x >= 0.0
                && y >= 0.0
                && x + width <= f64::from(logical.width)
                && y + height <= f64::from(logical.height),
            "facility_unavailable: partially clipped Niri window capture is unsupported"
        );
        ensure!(
            process_start(pid)? == start,
            "surface_identity_unproven: Niri target process generation changed"
        );
        Some(WindowEvidence {
            window_id: id,
            pid,
            process_start_ticks: start,
            rendered_rect: placement.rect,
        })
    } else {
        None
    };
    let evidence = CaptureEvidence {
        coordination_revision,
        compositor_pid: connection.peer.pid,
        compositor_uid: connection.peer.uid,
        compositor_start_ticks: connection.peer.start_ticks,
        scene_frame_serial: scene.frame_serial,
        output: scene.output.clone(),
        logical_output: logical,
        source_width: scene.width,
        source_height: scene.height,
        source_png_sha256: hex_digest(&png),
        image_semantic_atomic: false,
        image_geometry_atomic: true,
        capture_semantics: "visible output pixels; window crops may include occluding surfaces",
        rendered_windows: scene
            .windows
            .iter()
            .filter(|placement| target.is_none_or(|(id, _)| id == placement.id))
            .cloned()
            .collect(),
        window,
    };
    let png = if let Some(window) = &evidence.window {
        let (x, y, width, height) = window.rendered_rect;
        let rect = (x, y, width, height);
        resample_png(
            &png,
            scene.width,
            scene.height,
            &logical,
            rect,
            connection.deadline,
        )?
    } else {
        png
    };
    ensure!(
        process_start(connection.peer.pid)? == connection.peer.start_ticks,
        "Niri compositor generation changed during capture"
    );
    Ok(CapturedFrame {
        png,
        evidence,
        windows,
    })
}

fn validate_scene(
    scene: &CapturedScene,
    connection: &Connection,
    output: &str,
    logical: LogicalOutput,
) -> Result<()> {
    ensure!(
        scene.output == output
            && scene.compositor_pid == connection.peer.pid
            && scene.frame_serial > 0,
        "invalid capture scene identity"
    );
    ensure!(
        scene.logical_output == logical,
        "capture output geometry changed"
    );
    ensure!(
        scene.png_base64.len() <= 44 * 1024 * 1024
            && scene.width > 0
            && scene.height > 0
            && u64::from(scene.width) * u64::from(scene.height) <= MAX_PIXELS,
        "scene exceeds image budget"
    );
    ensure!(
        scene.windows.len() <= 8192,
        "scene exceeds window inventory budget"
    );
    for window in &scene.windows {
        let (x, y, width, height) = window.rect;
        ensure!(
            [x, y, width, height].iter().all(|value| value.is_finite())
                && width > 0.0
                && height > 0.0,
            "invalid captured window geometry"
        );
    }
    Ok(())
}

pub fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn png_dimensions(png: &[u8]) -> Result<(u32, u32)> {
    let mut reader = image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(MAX_PNG_BYTES as u64);
    reader.limits(limits);
    let decoded = reader.decode().context("decode captured PNG")?;
    let dimensions = (decoded.width(), decoded.height());
    ensure!(
        dimensions.0 > 0
            && dimensions.1 > 0
            && u64::from(dimensions.0) * u64::from(dimensions.1) <= MAX_PIXELS,
        "screenshot exceeds pixel budget"
    );
    Ok(dimensions)
}

// These clipping and exact logical sampling operations are copied from Dojo's screenshot.rs.
fn resample_png(
    source_png: &[u8],
    source_width: u32,
    source_height: u32,
    output: &LogicalOutput,
    rect: (f64, f64, f64, f64),
    deadline: Instant,
) -> Result<Vec<u8>> {
    let mut reader =
        image::ImageReader::with_format(Cursor::new(source_png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_PNG_BYTES as u64);
    reader.limits(limits);
    let image = reader.decode()?.to_rgba8();
    let (x, y, logical_width, logical_height) = rect;
    let width = u32::try_from(super::exact_integer(logical_width.ceil())?)?;
    let height = u32::try_from(super::exact_integer(logical_height.ceil())?)?;
    let sx = f64::from(source_width) / f64::from(output.width);
    let sy = f64::from(source_height) / f64::from(output.height);
    let left = (f64::from(x) * sx).floor() as u32;
    let top = (f64::from(y) * sy).floor() as u32;
    let right = ((x + logical_width) * sx).ceil() as u32;
    let bottom = ((y + logical_height) * sy).ceil() as u32;
    ensure!(
        right <= source_width && bottom <= source_height && right > left && bottom > top,
        "native crop exceeds captured output"
    );
    ensure!(
        u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "resampled image exceeds pixel budget"
    );
    let mut delivered = image::RgbaImage::new(width, height);
    for row in 0..height {
        ensure!(Instant::now() < deadline, "capture deadline exceeded");
        let source_y =
            ((f64::from(y) + (f64::from(row) + 0.5) * logical_height / f64::from(height)) * sy
                - 0.5)
                .clamp(0.0, f64::from(source_height - 1)) as f32;
        for column in 0..width {
            let source_x = ((f64::from(x)
                + (f64::from(column) + 0.5) * logical_width / f64::from(width))
                * sx
                - 0.5)
                .clamp(0.0, f64::from(source_width - 1)) as f32;
            let pixel = image::imageops::interpolate_bilinear(&image, source_x, source_y)
                .context("native sampling exceeds image")?;
            delivered.put_pixel(column, row, pixel);
        }
    }
    let mut png = Cursor::new(Vec::new());
    delivered.write_to(&mut png, image::ImageFormat::Png)?;
    ensure!(
        png.get_ref().len() <= MAX_PNG_BYTES,
        "screenshot exceeds image byte budget"
    );
    Ok(png.into_inner())
}

#[cfg(test)]
pub(crate) mod tests;
