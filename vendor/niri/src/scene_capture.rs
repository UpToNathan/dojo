use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use anyhow::{bail, ensure, Context};
use base64::{engine::general_purpose::STANDARD, Engine};
use niri_ipc::{CapturedScene, CapturedWindow, WindowPlacements};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::Output;
use smithay::utils::{Rectangle, Scale, Transform};

use crate::layout::LayoutElement;
use crate::niri::{Niri, State};
use crate::render_helpers::{render_to_vec, RenderCtx, RenderTarget};
use crate::utils::{logical_output, output_size, write_png_rgba8};

const MAX_CAPTURE_PIXELS: i64 = 8 * 1024 * 1024;
const MAX_PNG_BYTES: usize = 32 * 1024 * 1024;
static FRAME_SERIAL: AtomicU64 = AtomicU64::new(1);
static PENDING_CAPTURES: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct CapturePermit;
impl CapturePermit {
    pub(crate) fn acquire() -> Result<Self, &'static str> {
        PENDING_CAPTURES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                (pending < 2).then_some(pending + 1)
            })
            .map(|_| Self)
            .map_err(|_| "scene_capture_busy")
    }
}
impl Drop for CapturePermit {
    fn drop(&mut self) {
        PENDING_CAPTURES.fetch_sub(1, Ordering::Release);
    }
}

pub(crate) struct RawScene {
    metadata: CapturedScene,
    pixels: Vec<u8>,
}
impl RawScene {
    pub(crate) fn encode(mut self) -> anyhow::Result<CapturedScene> {
        let mut png = Vec::new();
        write_png_rgba8(
            &mut png,
            self.metadata.width,
            self.metadata.height,
            &self.pixels,
        )
        .context("scene_png_failed")?;
        ensure!(png.len() <= MAX_PNG_BYTES, "scene_capture_too_large");
        self.metadata.png_base64 = STANDARD.encode(png);
        Ok(self.metadata)
    }
}

impl State {
    pub(crate) fn window_placements(&mut self, name: &str) -> anyhow::Result<WindowPlacements> {
        let output = self
            .niri
            .global_space
            .outputs()
            .find(|output| output.name() == name)
            .cloned()
            .context("scene_output_unavailable")?;
        self.niri.window_placements(&output)
    }
    pub(crate) fn capture_scene(&mut self, name: &str) -> anyhow::Result<RawScene> {
        let output = self
            .niri
            .global_space
            .outputs()
            .find(|output| output.name() == name)
            .cloned()
            .context("scene_output_unavailable")?;
        self.backend
            .with_primary_renderer(|renderer| self.niri.capture_scene(renderer, &output))
            .context("scene_renderer_unavailable")?
    }
}

impl Niri {
    fn window_placements(&mut self, output: &Output) -> anyhow::Result<WindowPlacements> {
        if self.is_locked() || self.screenshot_ui.is_open() {
            bail!("scene_session_unavailable");
        }
        let state = self
            .output_state
            .get(output)
            .context("scene_output_unavailable")?;
        ensure!(
            !state.has_screen_transition(),
            "scene_screen_transition_unsupported"
        );
        let mode = output.current_mode().context("scene_output_unavailable")?;
        let size = output.current_transform().transform_size(mode.size);
        ensure!(size.w > 0 && size.h > 0, "scene_output_unavailable");

        self.update_render_elements(Some(output));
        let output_rect = Rectangle::from_size(output_size(output));
        let placements = self
            .layout
            .capture_placements(output)
            .map_err(anyhow::Error::msg)?;
        let windows = placements
            .into_iter()
            .filter_map(|(window, rect, is_floating)| {
                let clip = rect.intersection(output_rect)?;
                Some(CapturedWindow {
                    id: window.id().get(),
                    pid: window.credentials().map(|credentials| credentials.pid),
                    is_floating,
                    rect: (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h),
                    output_clip: (clip.loc.x, clip.loc.y, clip.size.w, clip.size.h),
                    blocked_out: RenderTarget::ScreenCapture
                        .should_block_out(window.rules().block_out_from),
                })
            })
            .collect::<Vec<_>>();
        ensure!(windows.len() <= 8192, "scene_geometry_too_large");
        let descriptor = logical_output(output);
        Ok(WindowPlacements {
            observation_serial: FRAME_SERIAL.fetch_add(1, Ordering::Relaxed),
            compositor_pid: std::process::id(),
            output: output.name(),
            logical_output: descriptor,
            width: size.w as u32,
            height: size.h as u32,
            windows,
        })
    }

    fn capture_scene(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
    ) -> anyhow::Result<RawScene> {
        let placements = self.window_placements(output)?;
        ensure!(
            i64::from(placements.width) * i64::from(placements.height) <= MAX_CAPTURE_PIXELS,
            "scene_capture_too_large"
        );
        let size = output
            .current_transform()
            .transform_size(output.current_mode().unwrap().size);
        let ctx = RenderCtx {
            renderer,
            target: RenderTarget::ScreenCapture,
            xray: None,
        };
        let elements = self.render_to_vec(ctx, output, false);
        let pixels = render_to_vec(
            renderer,
            size,
            Scale::from(output.current_scale().fractional_scale()),
            Transform::Normal,
            Fourcc::Abgr8888,
            elements.iter().rev(),
        )
        .context("scene_render_failed")?;
        ensure!(
            pixels.len() == size.w as usize * size.h as usize * 4,
            "scene_readback_size_mismatch"
        );
        Ok(RawScene {
            pixels,
            metadata: CapturedScene {
                frame_serial: placements.observation_serial,
                compositor_pid: placements.compositor_pid,
                output: placements.output,
                logical_output: placements.logical_output,
                width: placements.width,
                height: placements.height,
                png_base64: String::new(),
                windows: placements.windows,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_permits_limit_pending_memory_and_release_on_drop() {
        let first = CapturePermit::acquire().unwrap();
        let second = CapturePermit::acquire().unwrap();
        assert_eq!(CapturePermit::acquire().err(), Some("scene_capture_busy"));
        drop(first);
        let replacement = CapturePermit::acquire().unwrap();
        drop(second);
        drop(replacement);
        assert_eq!(PENDING_CAPTURES.load(Ordering::Acquire), 0);
    }
    #[test]
    fn capture_pixels_and_geometry_match_all_output_transforms() {
        use crate::tests::fixture::Fixture;
        use niri_ipc::{OutputAction, ScaleToSet, Transform as IpcTransform};
        let mut config = niri_config::Config::default();
        config.hotkey_overlay.skip_at_startup = true;
        let mut fixture = Fixture::with_config(config);
        fixture
            .niri_state()
            .backend
            .headless()
            .add_renderer()
            .unwrap();
        fixture.add_output(1, (640, 480));
        let output = fixture.niri_output(1);
        let client = fixture.add_client();
        let window = fixture.client(client).create_window();
        let surface = window.surface.clone();
        window.commit();
        fixture.roundtrip(client);
        let window = fixture.client(client).window(&surface);
        window.attach_red_buffer();
        window.set_size(100, 200);
        window.ack_last_and_commit();
        fixture.double_roundtrip(client);
        fixture.niri_complete_animations();
        fixture.niri_state().apply_transient_output_config(
            &output.name(),
            OutputAction::Scale {
                scale: ScaleToSet::Specific(1.25),
            },
        );
        for (transform, size) in [
            (IpcTransform::Normal, (640, 480)),
            (IpcTransform::_90, (480, 640)),
            (IpcTransform::_180, (640, 480)),
            (IpcTransform::_270, (480, 640)),
            (IpcTransform::Flipped, (640, 480)),
            (IpcTransform::Flipped90, (480, 640)),
            (IpcTransform::Flipped180, (640, 480)),
            (IpcTransform::Flipped270, (480, 640)),
        ] {
            fixture.niri_state().apply_transient_output_config(
                &output.name(),
                OutputAction::Transform { transform },
            );
            fixture.double_roundtrip(client);
            fixture.niri_complete_animations();
            let scene = fixture
                .niri_state()
                .capture_scene(&output.name())
                .unwrap()
                .encode()
                .unwrap();
            assert_eq!(scene.logical_output.transform, transform);
            assert_eq!(scene.logical_output.scale, 1.25);
            assert_eq!((scene.width, scene.height), size);
            assert_eq!(scene.windows.len(), 1);
            let window = &scene.windows[0];
            assert!(!window.blocked_out);
            let png = STANDARD.decode(&scene.png_base64).unwrap();
            let mut reader = png::Decoder::new(std::io::Cursor::new(png))
                .read_info()
                .unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((info.width, info.height), size);
            let (x, y, w, h) = window.output_clip;
            for (x, y) in [
                (x + w / 2., y + h / 2.),
                (x + 2., y + 2.),
                (x + w - 2., y + h - 2.),
            ] {
                let x = (x * 1.25).round() as usize;
                let y = (y * 1.25).round() as usize;
                let at = (y * scene.width as usize + x) * 4;
                assert_eq!(
                    &pixels[at..at + 4],
                    &[255, 0, 0, 255],
                    "{transform:?} at {x},{y}"
                );
            }
        }
    }
}
