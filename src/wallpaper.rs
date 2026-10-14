use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use gpui::{
    App, AppContext, Asset, Context, DisplayId, Render, Subscription, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, div, img, layer_shell, prelude::*, px,
};
use sha2::{Digest, Sha256};

struct WallpaperSource {
    path: PathBuf,
    digest: [u8; 32],
}

struct ReadyWallpaper {
    source: WallpaperSource,
    image: Arc<gpui::RenderImage>,
}

#[derive(Default)]
struct WallpaperSelection {
    ready: Option<ReadyWallpaper>,
}

impl WallpaperSelection {
    fn digest(&self) -> Option<[u8; 32]> {
        self.ready.as_ref().map(|ready| ready.source.digest)
    }
    fn update(&mut self, ready: Option<ReadyWallpaper>) -> bool {
        let digest = ready.as_ref().map(|ready| ready.source.digest);
        if self.digest() == digest {
            return false;
        }
        self.ready = ready;
        true
    }
}

fn selected_wallpaper(file: PathBuf) -> anyhow::Result<Option<WallpaperSource>> {
    let path = match std::fs::read_to_string(file) {
        Ok(path) => PathBuf::from(path.trim()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("reading selected wallpaper"),
    };
    anyhow::ensure!(
        path.is_absolute() && path.is_file(),
        "selected wallpaper must be an existing absolute file"
    );
    let digest = Sha256::digest(std::fs::read(&path)?).into();
    Ok(Some(WallpaperSource { path, digest }))
}

fn monitor_wallpaper(
    source: &gpui::RenderImage,
    width: u32,
    height: u32,
) -> Arc<gpui::RenderImage> {
    let mut frames = Vec::new();
    for index in 0..source.frame_count() {
        let size = source.size(index);
        let pixels = image::RgbaImage::from_raw(
            size.width.0 as u32,
            size.height.0 as u32,
            source.as_bytes(index).unwrap().to_vec(),
        )
        .unwrap();
        let ratio = (width as f64 / pixels.width() as f64)
            .max(height as f64 / pixels.height() as f64)
            .min(1.0);
        let resized = image::imageops::resize(
            &pixels,
            (pixels.width() as f64 * ratio).ceil().max(1.) as u32,
            (pixels.height() as f64 * ratio).ceil().max(1.) as u32,
            image::imageops::FilterType::Lanczos3,
        );
        let crop_width = width.min(resized.width());
        let crop_height = height.min(resized.height());
        let cropped = image::imageops::crop_imm(
            &resized,
            (resized.width() - crop_width) / 2,
            (resized.height() - crop_height) / 2,
            crop_width,
            crop_height,
        )
        .to_image();
        frames.push(image::Frame::from_parts(cropped, 0, 0, source.delay(index)));
    }
    Arc::new(gpui::RenderImage::new(frames))
}

struct WallpaperView {
    source: Arc<gpui::RenderImage>,
    image: Arc<gpui::RenderImage>,
    prepared_for: Option<(u32, u32)>,
    pending_for: Option<(u32, u32)>,
}

impl Render for WallpaperView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = window.viewport_size();
        let scale = window.scale_factor();
        let target = (
            (f32::from(size.width) * scale).round().max(1.) as u32,
            (f32::from(size.height) * scale).round().max(1.) as u32,
        );
        if self.prepared_for != Some(target) && self.pending_for != Some(target) {
            self.pending_for = Some(target);
            let source = self.source.clone();
            let input = source.clone();
            let preparation = cx
                .background_executor()
                .spawn(async move { monitor_wallpaper(&input, target.0, target.1) });
            cx.spawn_in(window, async move |view, cx| {
                let image = preparation.await;
                let _ = view.update_in(cx, |view, window, cx| {
                    if !Arc::ptr_eq(&source, &view.source) || view.pending_for != Some(target) {
                        return;
                    }
                    view.pending_for = None;
                    view.prepared_for = Some(target);
                    let previous = std::mem::replace(&mut view.image, image);
                    window.on_next_frame(move |window, _| {
                        if let Err(error) = window.drop_image(previous) {
                            eprintln!("Could not retire previous wallpaper: {error:#}");
                        }
                    });
                    cx.notify();
                });
            })
            .detach();
        }
        window.set_input_region(Some(&[]));
        div().size_full().child(
            img(self.image.clone())
                .size_full()
                .object_fit(gpui::ObjectFit::Cover),
        )
    }
}

struct WallpaperController {
    selection: WallpaperSelection,
    windows: HashMap<DisplayId, WindowHandle<WallpaperView>>,
    observer: Option<Subscription>,
}

impl WallpaperController {
    fn reconcile(&mut self, cx: &mut App) -> anyhow::Result<()> {
        let displays = cx.displays();
        let ids: Vec<_> = displays.iter().map(|d| d.id()).collect();
        let removed: Vec<_> = self
            .windows
            .keys()
            .filter(|id| !ids.contains(id) || self.selection.ready.is_none())
            .copied()
            .collect();
        for id in removed {
            if let Some(window) = self.windows.remove(&id) {
                window.update(cx, |_, window, _| window.remove_window())?;
            }
        }
        let Some(ready) = self.selection.ready.as_ref() else {
            return Ok(());
        };
        for display in displays {
            if let Some(window) = self.windows.get(&display.id()) {
                window.update(cx, |view, _window, cx| {
                    if !Arc::ptr_eq(&view.source, &ready.image) {
                        view.source = ready.image.clone();
                        view.prepared_for = None;
                        view.pending_for = None;
                        cx.notify();
                    }
                })?;
            } else {
                let image = ready.image.clone();
                let window = cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(display.bounds())),
                        display_id: Some(display.id()),
                        titlebar: None,
                        app_id: Some("dojo".into()),
                        window_background: gpui::WindowBackgroundAppearance::Transparent,
                        kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                            namespace: format!("dojo-wallpaper-{}", u64::from(display.id())),
                            layer: layer_shell::Layer::Bottom,
                            anchor: layer_shell::Anchor::TOP
                                | layer_shell::Anchor::BOTTOM
                                | layer_shell::Anchor::LEFT
                                | layer_shell::Anchor::RIGHT,
                            exclusive_zone: Some(px(-1.)),
                            keyboard_interactivity: layer_shell::KeyboardInteractivity::None,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    |_, cx| {
                        cx.new(|_| WallpaperView {
                            source: image.clone(),
                            image,
                            prepared_for: None,
                            pending_for: None,
                        })
                    },
                )?;
                self.windows.insert(display.id(), window);
            }
        }
        Ok(())
    }
}

pub fn start(cx: &mut App, events: async_channel::Receiver<()>) -> anyhow::Result<()> {
    let path = dojo::theme::Paths::user_default()?
        .root
        .join("current/wallpaper.txt");
    let controller = cx.new(|_| WallpaperController {
        selection: WallpaperSelection::default(),
        windows: HashMap::new(),
        observer: None,
    });
    let weak = controller.downgrade();
    let observer = cx.observe_displays(move |_, cx| {
        if let Err(error) = weak
            .update(cx, |controller, cx| controller.reconcile(cx))
            .and_then(|result| result)
        {
            eprintln!("Dojo wallpaper output update failed: {error:#}");
        }
    });
    controller.update(cx, |controller, _| controller.observer = Some(observer));
    let task_controller = controller.clone();
    cx.spawn(async move |cx| {
        while events.recv().await.is_ok() {
            while events.try_recv().is_ok() {}
            let file = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { selected_wallpaper(file) })
                .await;
            let ready = match result {
                Ok(Some(source)) => {
                    let unchanged = task_controller.update(cx, |controller, _| {
                        controller.selection.digest() == Some(source.digest)
                    });
                    if unchanged {
                        continue;
                    }
                    let resource = gpui::Resource::Path(source.path.clone().into());
                    let loading = cx.update(|cx| {
                        let future = gpui::ImageAssetLoader::load(resource, cx);
                        cx.background_executor().spawn(future)
                    });
                    match loading.await {
                        Ok(image) => Some(ReadyWallpaper { source, image }),
                        Err(error) => {
                            eprintln!("Dojo retained its previous wallpaper: {error:#}");
                            continue;
                        }
                    }
                }
                Ok(None) => None,
                Err(error) => {
                    eprintln!("Dojo retained its previous wallpaper: {error:#}");
                    continue;
                }
            };
            if !events.is_empty() {
                continue;
            }
            if let Err(error) = task_controller.update(cx, |controller, cx| {
                if controller.selection.update(ready) {
                    controller.reconcile(cx)
                } else {
                    Ok(())
                }
            }) {
                eprintln!("Dojo wallpaper update failed: {error:#}");
            }
        }
    })
    .detach();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_preparation_filters_detail_and_preserves_frame_delays() {
        let mut pixels = image::RgbaImage::new(160, 120);
        for (x, y, pixel) in pixels.enumerate_pixels_mut() {
            let value = if (x + y) % 2 == 0 { 0 } else { 255 };
            *pixel = image::Rgba([value, value, value, 255]);
        }
        let source = gpui::RenderImage::new(vec![
            image::Frame::from_parts(
                pixels.clone(),
                0,
                0,
                image::Delay::from_numer_denom_ms(125, 1),
            ),
            image::Frame::new(pixels),
        ]);
        let rendered = monitor_wallpaper(&source, 40, 20);
        assert_eq!(
            rendered.size(0),
            gpui::size(gpui::DevicePixels(40), gpui::DevicePixels(20))
        );
        assert_eq!(rendered.frame_count(), 2);
        assert_eq!(rendered.delay(0), source.delay(0));
        assert!(
            rendered
                .as_bytes(0)
                .unwrap()
                .chunks_exact(4)
                .all(|pixel| (120..=136).contains(&pixel[0]))
        );
        let small = gpui::RenderImage::new(vec![image::Frame::new(image::RgbaImage::new(16, 9))]);
        assert_eq!(monitor_wallpaper(&small, 1920, 1080).size(0), small.size(0));
    }

    fn ready(source: Option<WallpaperSource>) -> Option<ReadyWallpaper> {
        source.map(|source| ReadyWallpaper {
            source,
            image: Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::new(1, 1),
            )])),
        })
    }
    #[test]
    fn theme_generations_with_identical_wallpaper_keep_the_rendered_source() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("generation-one");
        let second = directory.path().join("generation-two");
        let selected = directory.path().join("wallpaper.txt");
        std::fs::write(&first, b"same wallpaper snapshot")?;
        std::fs::write(&second, b"same wallpaper snapshot")?;
        let mut selection = WallpaperSelection::default();
        std::fs::write(&selected, first.to_string_lossy().as_bytes())?;
        assert!(selection.update(ready(selected_wallpaper(selected.clone())?)));
        std::fs::write(&selected, second.to_string_lossy().as_bytes())?;
        assert!(!selection.update(ready(selected_wallpaper(selected.clone())?)));
        assert_eq!(selection.ready.as_ref().unwrap().source.path, first);
        std::fs::write(&second, b"different wallpaper")?;
        assert!(selection.update(ready(selected_wallpaper(selected)?)));
        assert_eq!(selection.ready.as_ref().unwrap().source.path, second);
        Ok(())
    }
}
