use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use dojo_computer::{CaptureFrame, CaptureScope, OutputFrame, WindowIdentity};
use niri_ipc::{LogicalOutput, Request, Response, socket::Socket};
use rmcp::model::ContentBlock;
use serde_json::json;

pub(super) type CaptureRequest = CaptureScope;

pub(super) struct CapturedImage {
    pub(super) png: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) metadata: serde_json::Value,
    pub(super) frame: Option<CaptureFrame>,
    pub(super) scene_serial: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
struct CapturePlan {
    output: LogicalOutput,
    rect: (i32, i32, u32, u32),
    scale: f64,
}

fn clipped_rect(
    output: &LogicalOutput,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(i32, i32, u32, u32)> {
    ensure!(
        [x, y, width, height].iter().all(|v| v.is_finite()) && width > 0. && height > 0.,
        "capture rectangle must be finite and nonempty"
    );
    let left = x.max(0.).floor();
    let top = y.max(0.).floor();
    let right = (x + width).min(f64::from(output.width)).ceil();
    let bottom = (y + height).min(f64::from(output.height)).ceil();
    ensure!(
        right > left && bottom > top,
        "capture rectangle is outside the selected output"
    );
    ensure!(
        left <= f64::from(i32::MAX) && top <= f64::from(i32::MAX),
        "capture origin exceeds coordinate limits"
    );
    Ok((
        left as i32,
        top as i32,
        (right - left) as u32,
        (bottom - top) as u32,
    ))
}

fn resolve(socket: &mut Socket, request: &CaptureRequest, scale: f64) -> Result<CapturePlan> {
    let Response::Outputs(outputs) = socket.send(Request::Outputs)?.map_err(anyhow::Error::msg)?
    else {
        bail!("unexpected outputs response");
    };
    let output = outputs
        .get(&request.output)
        .context("unknown output; inspect desktop_context")?
        .logical
        .context("output is disabled")?;
    let rect = if let Some(region) = &request.region {
        clipped_rect(
            &output,
            f64::from(region.x),
            f64::from(region.y),
            f64::from(region.width),
            f64::from(region.height),
        )?
    } else if let Some(id) = request.window_id {
        let Response::Windows(windows) =
            socket.send(Request::Windows)?.map_err(anyhow::Error::msg)?
        else {
            bail!("unexpected windows response");
        };
        let window = windows
            .iter()
            .find(|w| w.id == id)
            .context("unknown window_id; inspect desktop_context")?;
        let Response::Workspaces(workspaces) = socket
            .send(Request::Workspaces)?
            .map_err(anyhow::Error::msg)?
        else {
            bail!("unexpected workspaces response");
        };
        let workspace = workspaces
            .iter()
            .find(|w| Some(w.id) == window.workspace_id)
            .context("window has no workspace")?;
        ensure!(
            workspace.is_active && workspace.output.as_deref() == Some(request.output.as_str()),
            "window is not on the selected output's active workspace"
        );
        let (x, y) = window.layout.tile_pos_in_workspace_view
            .context("compositor does not supply this window's position; use an explicit region or full output")?;
        let (width, height) = window.layout.tile_size;
        clipped_rect(&output, x, y, width, height)?
    } else {
        clipped_rect(
            &output,
            0.,
            0.,
            f64::from(output.width),
            f64::from(output.height),
        )?
    };
    ensure!(
        f64::from(rect.2) * scale <= 32768.
            && f64::from(rect.3) * scale <= 32768.
            && f64::from(rect.2) * f64::from(rect.3) * scale * scale <= 128. * 1024. * 1024.,
        "capture exceeds the pixel budget"
    );
    Ok(CapturePlan {
        output,
        rect,
        scale,
    })
}

fn metadata(
    request: &CaptureRequest,
    plan: &CapturePlan,
    width: u32,
    height: u32,
) -> serde_json::Value {
    let (x, y, logical_width, logical_height) = plan.rect;
    json!({
        "output": request.output, "window_id": request.window_id,
        "image_width": width, "image_height": height,
        "logical_x": x, "logical_y": y,
        "logical_width": logical_width, "logical_height": logical_height,
        "logical_x_per_image_pixel": f64::from(logical_width) / f64::from(width),
        "logical_y_per_image_pixel": f64::from(logical_height) / f64::from(height),
        "capture": "visible output pixels; window crops may include occluding surfaces",
        "coordinate_system": "output-relative logical pixels; desktop x/y = logical_x/y + image x/y * logical_x/y_per_image_pixel"
    })
}

pub(super) fn capture(
    socket_path: Option<&Path>,
    request: &CaptureRequest,
) -> Result<Vec<ContentBlock>> {
    let image = capture_image(socket_path, request)?;
    Ok(vec![
        ContentBlock::text(image.metadata.to_string()),
        ContentBlock::image(STANDARD.encode(image.png), "image/png"),
    ])
}

pub(super) fn capture_image(
    socket_path: Option<&Path>,
    request: &CaptureRequest,
) -> Result<CapturedImage> {
    capture_image_cancellable(
        socket_path,
        request,
        &std::sync::atomic::AtomicBool::new(false),
    )
}

pub(super) fn capture_image_cancellable(
    socket_path: Option<&Path>,
    request: &CaptureRequest,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<CapturedImage> {
    let scale = request.validate()?;
    let mut socket = super::desktop_socket(socket_path)?;
    if request.window_id.is_some() {
        return capture_window_scene(request, scale, &mut socket, cancelled);
    }
    let plan = resolve(&mut socket, request, scale)?;
    match std::env::var("DOJO_CAPTURE_BACKEND").as_deref() {
        Ok("grim") => {}
        Ok("native") | Err(_) => {
            return native_capture(socket_path, request, &plan, &mut socket, cancelled);
        }
        Ok(_) => bail!("DOJO_CAPTURE_BACKEND must be native or grim"),
    }
    let mut image_file = tempfile::tempfile()?;
    let mut error_file = tempfile::tempfile()?;
    let mut command = Command::new("grim");
    command.args(["-s", &scale.to_string()]);
    if request.region.is_some() || request.window_id.is_some() {
        let (x, y, width, height) = plan.rect;
        command.args([
            "-g",
            &format!(
                "{},{} {}x{}",
                i64::from(plan.output.x) + i64::from(x),
                i64::from(plan.output.y) + i64::from(y),
                width,
                height
            ),
        ]);
    } else {
        command.args(["-o", request.output.as_str()]);
    }
    let mut child = command
        .arg("-")
        .stdin(Stdio::null())
        .stdout(image_file.try_clone()?)
        .stderr(error_file.try_clone()?)
        .spawn()
        .context("start screenshot capture; install grim and expose it on PATH")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None)
                if Instant::now() < deadline
                    && !cancelled.load(std::sync::atomic::Ordering::Acquire) =>
            {
                std::thread::sleep(Duration::from_millis(20))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                if let Err(error) = result {
                    return Err(error.into());
                }
                bail!("desktop capture timed out after five seconds");
            }
        }
    };
    error_file.seek(SeekFrom::Start(0))?;
    let mut errors = String::new();
    error_file.take(4096).read_to_string(&mut errors)?;
    ensure!(
        status.success(),
        "desktop capture failed: {}",
        errors.trim()
    );
    ensure!(
        image_file.metadata()?.len() <= 32 * 1024 * 1024,
        "screenshot exceeds 32 MiB"
    );
    image_file.seek(SeekFrom::Start(0))?;
    let mut png = Vec::new();
    image_file.read_to_end(&mut png)?;
    let (width, height) = png_dimensions(&png)?;
    ensure!(
        resolve(&mut socket, request, scale)? == plan,
        "capture geometry changed; inspect desktop_context and capture again"
    );
    Ok(CapturedImage {
        png,
        width,
        height,
        metadata: metadata(request, &plan, width, height),
        frame: None,
        scene_serial: None,
    })
}

fn native_capture(
    socket_path: Option<&Path>,
    request: &CaptureRequest,
    plan: &CapturePlan,
    socket: &mut Socket,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<CapturedImage> {
    let display = std::env::var_os("WAYLAND_DISPLAY")
        .context("WAYLAND_DISPLAY is required for native capture")?;
    let display = std::path::PathBuf::from(display);
    let wayland_socket = if display.is_absolute() {
        display
    } else {
        std::path::PathBuf::from(
            std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is required")?,
        )
        .join(display)
    };
    let niri_socket = super::niri_socket(socket_path)?;
    let output = dojo_capture_wayland::capture_output(&dojo_capture_wayland::CaptureRequest {
        wayland_socket: &wayland_socket,
        niri_socket: &niri_socket,
        output: &request.output,
        deadline: Instant::now() + Duration::from_secs(5),
        cancelled,
        limits: dojo_capture_wayland::Limits::default(),
    })?;
    ensure!(
        resolve(socket, request, plan.scale)? == *plan,
        "capture geometry changed; observe again"
    );
    let mut image = resample_png(
        &output.png,
        output.width,
        output.height,
        request,
        plan,
        cancelled,
    )?;
    image.metadata["backend"] = "native_ext_image_copy_capture".into();
    image.metadata["compositor_pid"] = output.compositor_pid.into();
    image.metadata["buffer_transform"] = format!("{:?}", output.buffer_transform).into();
    Ok(image)
}

fn resample_png(
    source_png: &[u8],
    source_width: u32,
    source_height: u32,
    request: &CaptureRequest,
    plan: &CapturePlan,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<CapturedImage> {
    let image =
        image::load_from_memory_with_format(&source_png, image::ImageFormat::Png)?.to_rgba8();
    let (x, y, width, height) = plan.rect;
    let sx = f64::from(source_width) / f64::from(plan.output.width);
    let sy = f64::from(source_height) / f64::from(plan.output.height);
    let left = (f64::from(x) * sx).floor() as u32;
    let top = (f64::from(y) * sy).floor() as u32;
    let right = (f64::from(x + i32::try_from(width)?) * sx).ceil() as u32;
    let bottom = (f64::from(y + i32::try_from(height)?) * sy).ceil() as u32;
    ensure!(
        right <= source_width && bottom <= source_height && right > left && bottom > top,
        "native crop exceeds captured output"
    );
    let image_width = (f64::from(width) * plan.scale).round().max(1.) as u32;
    let image_height = (f64::from(height) * plan.scale).round().max(1.) as u32;
    // Sample the exact logical rectangle. Rounding a physical crop outward
    // would shift the published image-to-logical mapping on fractional scales.
    let mut delivered = image::RgbaImage::new(image_width, image_height);
    for row in 0..image_height {
        ensure!(
            !cancelled.load(std::sync::atomic::Ordering::Acquire),
            "capture cancelled"
        );
        let source_y = ((f64::from(y)
            + (f64::from(row) + 0.5) * f64::from(height) / f64::from(image_height))
            * sy
            - 0.5)
            .clamp(0., f64::from(source_height - 1)) as f32;
        for column in 0..image_width {
            let source_x = ((f64::from(x)
                + (f64::from(column) + 0.5) * f64::from(width) / f64::from(image_width))
                * sx
                - 0.5)
                .clamp(0., f64::from(source_width - 1)) as f32;
            let pixel = image::imageops::interpolate_bilinear(&image, source_x, source_y)
                .context("native sampling exceeds image")?;
            delivered.put_pixel(column, row, pixel);
        }
    }
    let mut png = Cursor::new(Vec::new());
    delivered.write_to(&mut png, image::ImageFormat::Png)?;
    ensure!(
        png.get_ref().len() <= 32 * 1024 * 1024,
        "screenshot exceeds 32 MiB"
    );

    Ok(CapturedImage {
        png: png.into_inner(),
        width: image_width,
        height: image_height,
        metadata: metadata(request, plan, image_width, image_height),
        frame: None,
        scene_serial: None,
    })
}

/// Capture-bound actions use exact process/window/output evidence. Ordinary
/// legacy screenshots remain available when the fork cannot provide it.
pub(super) fn current_frame(
    socket_path: Option<&Path>,
    request: &CaptureRequest,
) -> Result<CaptureFrame> {
    let scale = request.validate()?;
    let mut socket = super::desktop_socket(socket_path)?;
    let scene = if request.window_id.is_some() {
        Some(get_scene(&mut socket, request)?)
    } else {
        None
    };
    let plan = match &scene {
        Some(scene) => scene_plan(scene, request, scale)?,
        None => resolve(&mut socket, request, scale)?,
    };
    frame_for_plan(&mut socket, request, &plan, scene.as_ref())
}

fn frame_for_plan(
    socket: &mut Socket,
    request: &CaptureRequest,
    plan: &CapturePlan,
    scene: Option<&niri_ipc::CapturedScene>,
) -> Result<CaptureFrame> {
    let window = if let Some(id) = request.window_id {
        let Response::Windows(windows) =
            socket.send(Request::Windows)?.map_err(anyhow::Error::msg)?
        else {
            bail!("unexpected windows response");
        };
        let window = windows
            .into_iter()
            .find(|window| window.id == id)
            .context("window disappeared")?;
        let pid: u32 = window
            .pid
            .context("window process identity is unavailable")?
            .try_into()?;
        ensure!(pid > 0, "window process identity is unavailable");
        if let Some(scene) = scene {
            ensure!(
                scene
                    .windows
                    .iter()
                    .any(|w| w.id == id && w.pid == Some(pid as i32)),
                "captured window identity changed"
            );
        }
        Some(WindowIdentity {
            window_id: id,
            pid,
            process_start_time: process_start_time(pid)?,
            app_id: window.app_id,
            rendered_rect: scene
                .and_then(|scene| scene.windows.iter().find(|w| w.id == id))
                .context("window has no rendered geometry")?
                .rect,
            workspace_id: window
                .workspace_id
                .context("window workspace is unavailable")?,
        })
    } else {
        None
    };
    let Response::InputCoordination(coordination) = socket
        .send(Request::InputCoordination)?
        .map_err(anyhow::Error::msg)?
    else {
        bail!("compositor does not provide input coordination");
    };
    Ok(CaptureFrame {
        output: OutputFrame {
            name: request.output.clone(),
            x: plan.output.x,
            y: plan.output.y,
            width: plan.output.width,
            height: plan.output.height,
            scale: plan.output.scale,
            transform: format!("{:?}", plan.output.transform),
            compositor: scene
                .map(|scene| {
                    Ok::<_, anyhow::Error>(dojo_computer::CompositorIdentity {
                        pid: scene.compositor_pid,
                        process_start_time: process_start_time(scene.compositor_pid)?,
                    })
                })
                .transpose()?,
        },
        logical_x: plan.rect.0,
        logical_y: plan.rect.1,
        logical_width: plan.rect.2,
        logical_height: plan.rect.3,
        window,
        coordination_revision: coordination.revision,
    })
}

pub(super) fn process_start_time(pid: u32) -> Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .context("read window process identity")?;
    process_start_time_from_stat(&stat)
}

fn get_scene(socket: &mut Socket, request: &CaptureRequest) -> Result<niri_ipc::CapturedScene> {
    let Response::CapturedScene(scene) = socket
        .send(Request::CaptureScene {
            output: request.output.clone(),
        })?
        .map_err(anyhow::Error::msg)?
    else {
        bail!("compositor does not provide capture-correlated scene geometry");
    };
    ensure!(
        scene.output == request.output && scene.compositor_pid > 0 && scene.frame_serial > 0,
        "invalid capture scene identity"
    );
    ensure!(
        scene.png_base64.len() <= 44 * 1024 * 1024
            && scene.width > 0
            && scene.height > 0
            && u64::from(scene.width) * u64::from(scene.height) <= 8 * 1024 * 1024,
        "scene exceeds image budget"
    );
    Ok(scene)
}

fn scene_plan(
    scene: &niri_ipc::CapturedScene,
    request: &CaptureRequest,
    scale: f64,
) -> Result<CapturePlan> {
    let id = request
        .window_id
        .context("scene capture requires a window")?;
    let window = scene
        .windows
        .iter()
        .find(|window| window.id == id)
        .context("window is not visible in the captured scene")?;
    ensure!(
        !window.blocked_out,
        "window is excluded by screen-capture privacy rules"
    );
    let (x, y, width, height) = window.output_clip;
    let rect = clipped_rect(&scene.logical_output, x, y, width, height)?;
    Ok(CapturePlan {
        output: scene.logical_output,
        rect,
        scale,
    })
}

fn capture_window_scene(
    request: &CaptureRequest,
    scale: f64,
    socket: &mut Socket,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<CapturedImage> {
    ensure!(
        !cancelled.load(std::sync::atomic::Ordering::Acquire),
        "capture cancelled"
    );
    let scene = get_scene(socket, request)?;
    let plan = scene_plan(&scene, request, scale)?;
    let png = STANDARD.decode(&scene.png_base64)?;
    ensure!(
        png_dimensions(&png)? == (scene.width, scene.height),
        "scene PNG dimensions mismatch"
    );
    let frame = frame_for_plan(socket, request, &plan, Some(&scene))?;
    let mut image = resample_png(&png, scene.width, scene.height, request, &plan, cancelled)?;
    image.metadata["backend"] = "niri_capture_scene".into();
    image.metadata["scene_frame_serial"] = scene.frame_serial.into();
    image.frame = Some(frame);
    image.scene_serial = Some(scene.frame_serial);
    Ok(image)
}

fn process_start_time_from_stat(stat: &str) -> Result<u64> {
    // comm can contain spaces and closing parentheses. The final ')' precedes
    // field 3; starttime is field 22, index 19 in the remaining fields.
    let (_, fields) = stat.rsplit_once(')').context("invalid process stat")?;
    let start: u64 = fields
        .split_whitespace()
        .nth(19)
        .context("missing process start time")?
        .parse()?;
    ensure!(start > 0, "invalid process start time");
    Ok(start)
}

fn png_dimensions(png: &[u8]) -> Result<(u32, u32)> {
    let mut reader = image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let (width, height) = reader
        .into_dimensions()
        .context("read captured PNG dimensions")?;
    ensure!(
        width > 0 && height > 0,
        "screenshot dimensions must be nonzero"
    );
    ensure!(
        u64::from(width) * u64::from(height) <= 128 * 1024 * 1024,
        "screenshot exceeds the pixel budget"
    );
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn output() -> LogicalOutput {
        LogicalOutput {
            x: -800,
            y: 100,
            width: 800,
            height: 600,
            scale: 1.75,
            transform: niri_ipc::Transform::Normal,
        }
    }

    #[test]
    fn process_start_time_handles_spaces_and_parentheses_in_comm() {
        let fields = format!("S {} 12345 0", vec!["0"; 18].join(" "));
        assert_eq!(
            process_start_time_from_stat(&format!("42 (name ) with spaces) {fields}")).unwrap(),
            12345
        );
        assert!(process_start_time_from_stat("42 (short) S 0").is_err());
    }

    #[test]
    fn clipping_preserves_output_relative_origins_and_rounds_outward() {
        assert_eq!(
            clipped_rect(&output(), -10., 20., 50., 30.).unwrap(),
            (0, 20, 40, 30)
        );
        assert_eq!(
            clipped_rect(&output(), 780.2, 590.4, 50., 30.).unwrap(),
            (780, 590, 20, 10)
        );
        assert_eq!(
            clipped_rect(&output(), 10.25, 20.75, 100., 50.).unwrap(),
            (10, 20, 101, 51)
        );
        for rect in [
            (800., 0., 20., 20.),
            (0., 0., 0., 10.),
            (0., f64::NAN, 10., 10.),
            (-30., 0., 10., 10.),
        ] {
            assert!(clipped_rect(&output(), rect.0, rect.1, rect.2, rect.3).is_err());
        }
    }

    #[test]
    fn request_accepts_existing_calls_and_rejects_invalid_selectors_or_scale() {
        let request: CaptureRequest = serde_json::from_value(json!({"output": "eDP-1"})).unwrap();
        assert_eq!(request.validate().unwrap(), 1.);
        for scale in [0., 0.09, 1.01, f64::INFINITY, f64::NAN] {
            assert!(
                CaptureRequest {
                    output: "eDP-1".into(),
                    region: None,
                    window_id: None,
                    scale: Some(scale)
                }
                .validate()
                .is_err()
            );
        }
        let mixed: CaptureRequest = serde_json::from_value(
            json!({"output":"eDP-1", "window_id":1, "region":{"x":0,"y":0,"width":10,"height":10}}),
        )
        .unwrap();
        assert!(mixed.validate().is_err());
        assert!(
            serde_json::from_value::<CaptureRequest>(
                json!({"output":"eDP-1", "region":{"x":0,"y":0,"width":10,"height":10,"typo":1}})
            )
            .is_err()
        );
    }

    #[test]
    fn cropped_and_scaled_image_maps_back_to_desktop_coordinates() {
        let request: CaptureRequest = serde_json::from_value(json!({"output":"left"})).unwrap();
        let plan = CapturePlan {
            output: output(),
            rect: (100, 80, 400, 200),
            scale: 0.5,
        };
        let result = metadata(&request, &plan, 200, 100);
        assert_eq!(result["logical_x"], 100);
        assert_eq!(result["logical_y"], 80);
        assert_eq!(result["logical_x_per_image_pixel"], 2.);
        assert_eq!(result["logical_y_per_image_pixel"], 2.);
        // Image point (25, 30) is desktop point (150, 140) on this output.
        assert_eq!(
            result["logical_x"].as_f64().unwrap()
                + 25. * result["logical_x_per_image_pixel"].as_f64().unwrap(),
            150.
        );
        assert_eq!(
            result["logical_y"].as_f64().unwrap()
                + 30. * result["logical_y_per_image_pixel"].as_f64().unwrap(),
            140.
        );
    }

    #[test]
    fn dimensions_match_the_captured_png() {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&[255; 24], 2, 3, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(png_dimensions(&png).unwrap(), (2, 3));
    }

    #[test]
    fn excessive_dimensions_are_rejected_before_pixel_decoding() {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &vec![0; 32_769 * 4],
                32_769,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        assert!(png_dimensions(&png).is_err());
    }

    #[test]
    fn malformed_and_truncated_headers_are_rejected() {
        assert!(png_dimensions(b"not a PNG").is_err());
        assert!(png_dimensions(b"\x89PNG\r\n\x1a\n").is_err());
    }
}
