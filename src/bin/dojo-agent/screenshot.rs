use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use niri_ipc::{Request, Response, socket::Socket};
use rmcp::model::ContentBlock;
use serde_json::json;

pub(super) fn capture(socket_path: Option<&Path>, output_name: &str) -> Result<Vec<ContentBlock>> {
    let mut socket = match socket_path {
        Some(path) => Socket::connect_to(path)?,
        None => Socket::connect()?,
    };
    let Response::Outputs(outputs) = socket.send(Request::Outputs)?.map_err(anyhow::Error::msg)?
    else {
        bail!("unexpected outputs response");
    };
    let output = outputs
        .get(output_name)
        .context("unknown output; inspect desktop_context")?;
    let logical = output.logical.as_ref().context("output is disabled")?;
    let mut image_file = tempfile::tempfile()?;
    let mut error_file = tempfile::tempfile()?;
    let mut child = Command::new("grim")
        .args(["-o", output_name, "-s", "1", "-"])
        .stdin(Stdio::null())
        .stdout(image_file.try_clone()?)
        .stderr(error_file.try_clone()?)
        .spawn()
        .context("start screenshot capture; install grim and expose it on PATH")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
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
    Ok(vec![
        ContentBlock::text(json!({
            "output": output_name,
            "image_width": width, "image_height": height,
            "logical_width": logical.width, "logical_height": logical.height,
            "logical_x_per_image_pixel": f64::from(logical.width) / f64::from(width),
            "logical_y_per_image_pixel": f64::from(logical.height) / f64::from(height),
            "coordinate_system": "output-relative logical pixels; multiply image x/y by the respective mapping before desktop_action move"
        }).to_string()),
        ContentBlock::image(STANDARD.encode(png), "image/png"),
    ])
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
