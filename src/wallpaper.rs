use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use dojo::theme::wallpaper_layout::{Monitor, Region, WallpaperLayout, span_regions};
use gpui::{
    App, AppContext, Asset, Context, DisplayId, Render, Subscription, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, div, img, layer_shell, prelude::*, px,
};
use sha2::{Digest, Sha256};

struct WallpaperSource {
    path: PathBuf,
    digest: [u8; 32],
    /// Pixel size read from the image header; spanning needs it to know what each monitor shows.
    size: Option<(u32, u32)>,
}

/// A monitor-sized wallpaper read back from the cache.
#[derive(Clone)]
struct CachedImage {
    width: u32,
    height: u32,
    image: Arc<gpui::RenderImage>,
}

struct ReadyWallpaper {
    source: WallpaperSource,
    /// The full-size decode. Absent when cached monitor-sized images stood in for it; a window
    /// decodes the source itself if it needs a size the cache lacks.
    image: Option<Arc<gpui::RenderImage>>,
    cached: Vec<CachedImage>,
}

impl ReadyWallpaper {
    /// What a new window shows until it has prepared its exact size: the cached image that best
    /// fits the display, else the full-size decode.
    ///
    /// A spanned window never trusts a cached image, which was cut for a whole monitor, so it
    /// reports no prepared size and prepares its region itself.
    fn initial(
        &self,
        width: f32,
        height: f32,
        spanning: bool,
    ) -> Option<(Arc<gpui::RenderImage>, Option<(u32, u32)>)> {
        let placeholder = match crate::wallpaper_cache::best_for(
            &self.cached,
            |cached| (cached.width, cached.height),
            width,
            height,
        ) {
            Some(cached) => Some((cached.image.clone(), Some((cached.width, cached.height)))),
            None => self.image.clone().map(|image| (image, None)),
        };
        placeholder.map(|(image, prepared_for)| (image, prepared_for.filter(|_| !spanning)))
    }
}

#[derive(Default)]
struct WallpaperSelection {
    ready: Option<ReadyWallpaper>,
    layout: WallpaperLayout,
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
    let size = image::image_dimensions(&path).ok();
    Ok(Some(WallpaperSource { path, digest, size }))
}

/// `region` limits the cover crop to one part of the image, for a wallpaper spanning monitors.
fn monitor_wallpaper(
    source: &gpui::RenderImage,
    width: u32,
    height: u32,
    region: Option<Region>,
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
        let pixels = match region {
            Some(region) => {
                let (x, y, width, height) = region.pixels(pixels.width(), pixels.height());
                image::imageops::crop_imm(&pixels, x, y, width, height).to_image()
            }
            None => pixels,
        };
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

/// Every monitor-sized image the cache holds for this wallpaper.
fn load_cached(digest: &[u8; 32]) -> Vec<CachedImage> {
    let Some(directory) = crate::wallpaper_cache::directory() else {
        return Vec::new();
    };
    crate::wallpaper_cache::load(&directory, digest)
        .into_iter()
        .filter_map(|prepared| {
            let pixels =
                image::RgbaImage::from_raw(prepared.width, prepared.height, prepared.pixels)?;
            Some(CachedImage {
                width: prepared.width,
                height: prepared.height,
                image: Arc::new(gpui::RenderImage::new(vec![image::Frame::new(pixels)])),
            })
        })
        .collect()
}

/// Writes a prepared wallpaper to the cache so the next start skips the decode and the resize.
/// Animated wallpapers, and images smaller than the display, are left out.
fn remember(digest: &[u8; 32], target: (u32, u32), image: &gpui::RenderImage) {
    if image.frame_count() != 1 {
        return;
    }
    let size = image.size(0);
    if (size.width.0 as u32, size.height.0 as u32) != target {
        return;
    }
    let (Some(directory), Some(pixels)) = (crate::wallpaper_cache::directory(), image.as_bytes(0))
    else {
        return;
    };
    if let Err(error) =
        crate::wallpaper_cache::store(&directory, digest, target.0, target.1, pixels)
    {
        eprintln!("Dojo could not cache its prepared wallpaper: {error:#}");
    }
}

struct WallpaperView {
    digest: [u8; 32],
    path: PathBuf,
    /// The full-size decode, once there is one.
    source: Option<Arc<gpui::RenderImage>>,
    cached: Vec<CachedImage>,
    image: Arc<gpui::RenderImage>,
    prepared_for: Option<(u32, u32)>,
    pending_for: Option<(u32, u32)>,
    /// The part of the image this monitor shows when the wallpaper spans monitors.
    region: Option<Region>,
}

impl WallpaperView {
    fn new(
        ready: &ReadyWallpaper,
        region: Option<Region>,
        width: f32,
        height: f32,
    ) -> Option<Self> {
        let (image, prepared_for) = ready.initial(width, height, region.is_some())?;
        Some(Self {
            region,
            digest: ready.source.digest,
            path: ready.source.path.clone(),
            source: ready.image.clone(),
            cached: if region.is_some() {
                Vec::new()
            } else {
                ready.cached.clone()
            },
            image,
            prepared_for,
            pending_for: None,
        })
    }

    /// Switches to a different wallpaper or region. The old image stays up until the new one is ready.
    fn adopt(&mut self, ready: &ReadyWallpaper, region: Option<Region>, width: f32, height: f32) {
        self.region = region;
        self.digest = ready.source.digest;
        self.path = ready.source.path.clone();
        self.source = ready.image.clone();
        self.cached = if region.is_some() {
            Vec::new()
        } else {
            ready.cached.clone()
        };
        self.prepared_for = None;
        self.pending_for = None;
        if let Some((image, prepared_for)) = ready.initial(width, height, region.is_some()) {
            self.image = image;
            self.prepared_for = prepared_for;
        }
    }
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
            if let Some(hit) = self
                .cached
                .iter()
                .find(|cached| (cached.width, cached.height) == target)
            {
                self.prepared_for = Some(target);
                let previous = std::mem::replace(&mut self.image, hit.image.clone());
                window.on_next_frame(move |window, _| {
                    if let Err(error) = window.drop_image(previous) {
                        eprintln!("Could not retire previous wallpaper: {error:#}");
                    }
                });
            } else {
                self.pending_for = Some(target);
                let digest = self.digest;
                let region = self.region;
                let source = self.source.clone();
                let decoding = source.is_none().then(|| {
                    let resource = gpui::Resource::Path(self.path.clone().into());
                    let future = gpui::ImageAssetLoader::load(resource, cx);
                    cx.background_executor().spawn(future)
                });
                let preparation = cx.background_executor().spawn(async move {
                    let source = match (source, decoding) {
                        (Some(source), _) => source,
                        (None, Some(decoding)) => match decoding.await {
                            Ok(source) => source,
                            Err(error) => {
                                eprintln!("Dojo could not decode its wallpaper: {error:#}");
                                return None;
                            }
                        },
                        (None, None) => return None,
                    };
                    let image = monitor_wallpaper(&source, target.0, target.1, region);
                    if region.is_none() {
                        remember(&digest, target, &image);
                    }
                    Some((source, image))
                });
                cx.spawn_in(window, async move |view, cx| {
                    let Some((source, image)) = preparation.await else {
                        return;
                    };
                    let _ = view.update_in(cx, |view, window, cx| {
                        if view.digest != digest
                            || view.region != region
                            || view.pending_for != Some(target)
                        {
                            return;
                        }
                        view.source = Some(source);
                        view.pending_for = None;
                        view.prepared_for = Some(target);
                        crate::startup::mark("wallpaper-resized");
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
        }
        window.set_input_region(Some(&[]));
        div().size_full().child(
            img(self.image.clone())
                .size_full()
                .object_fit(gpui::ObjectFit::Cover),
        )
    }
}

/// What each display shows of the wallpaper when it spans them, keyed by the display's uuid.
/// Positions and sizes come from Niri, which knows the logical layout including scale and rotation.
fn span_by_display(size: (u32, u32)) -> anyhow::Result<HashMap<uuid::Uuid, Region>> {
    let mut socket = niri_ipc::socket::Socket::connect()?;
    socket.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
    socket.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
    let response = socket
        .send(niri_ipc::Request::Outputs)?
        .map_err(anyhow::Error::msg)?;
    let niri_ipc::Response::Outputs(outputs) = response else {
        anyhow::bail!("Niri returned an unexpected response to Outputs");
    };
    let mut names = Vec::new();
    let mut monitors = Vec::new();
    for output in outputs.into_values() {
        if let Some(logical) = output.logical {
            names.push(output.name);
            monitors.push(Monitor {
                x: f64::from(logical.x),
                y: f64::from(logical.y),
                width: f64::from(logical.width),
                height: f64::from(logical.height),
            });
        }
    }
    let regions = span_regions(&monitors, size).context("no monitors to span")?;
    Ok(names
        .into_iter()
        .zip(regions)
        .map(|(name, region)| (crate::edge_reservation::display_uuid(&name), region))
        .collect())
}

struct WallpaperController {
    selection: WallpaperSelection,
    /// The layout the windows were last reconciled with.
    applied_layout: WallpaperLayout,
    windows: HashMap<DisplayId, WindowHandle<WallpaperView>>,
    observer: Option<Subscription>,
}

impl WallpaperController {
    fn layout_pending(&self) -> bool {
        self.applied_layout != self.selection.layout
    }

    fn reconcile(&mut self, cx: &mut App) -> anyhow::Result<()> {
        self.applied_layout = self.selection.layout;
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
        // Without a readable image size or monitor layout a spanning wallpaper falls back to
        // filling each monitor, which is what the user sees either way until it can be sliced.
        let regions = match (self.selection.layout, ready.source.size) {
            (WallpaperLayout::Span, Some(size)) => span_by_display(size)
                .map_err(|error| eprintln!("Dojo could not span the wallpaper: {error:#}"))
                .unwrap_or_default(),
            _ => HashMap::new(),
        };
        for display in displays {
            let region = display
                .uuid()
                .ok()
                .and_then(|uuid| regions.get(&uuid).copied());
            if let Some(window) = self.windows.get(&display.id()) {
                window.update(cx, |view, _window, cx| {
                    if view.digest != ready.source.digest || view.region != region {
                        let bounds = display.bounds();
                        view.adopt(
                            ready,
                            region,
                            f32::from(bounds.size.width),
                            f32::from(bounds.size.height),
                        );
                        cx.notify();
                    }
                })?;
            } else {
                let bounds = display.bounds();
                let Some(view) = WallpaperView::new(
                    ready,
                    region,
                    f32::from(bounds.size.width),
                    f32::from(bounds.size.height),
                ) else {
                    continue;
                };
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
                    |_, cx| cx.new(|_| view),
                )?;
                self.windows.insert(display.id(), window);
                crate::startup::mark("wallpaper-window");
            }
        }
        Ok(())
    }
}

pub fn start(cx: &mut App, events: async_channel::Receiver<()>) -> anyhow::Result<()> {
    let root = dojo::theme::Paths::user_default()?.root;
    let path = root.join("current/wallpaper.txt");
    let controller = cx.new(|_| WallpaperController {
        selection: WallpaperSelection::default(),
        applied_layout: WallpaperLayout::default(),
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
            let layout_root = root.clone();
            let (result, layout) = cx
                .background_executor()
                .spawn(async move {
                    (
                        selected_wallpaper(file),
                        WallpaperLayout::read(&layout_root),
                    )
                })
                .await;
            match layout {
                Ok(layout) => {
                    task_controller.update(cx, |controller, _| controller.selection.layout = layout)
                }
                Err(error) => eprintln!("Dojo kept its wallpaper layout: {error:#}"),
            }
            let ready = match result {
                Ok(Some(source)) => {
                    let unchanged = task_controller.update(cx, |controller, _| {
                        controller.selection.digest() == Some(source.digest)
                    });
                    if unchanged {
                        if let Err(error) = task_controller.update(cx, |controller, cx| {
                            if controller.layout_pending() {
                                controller.reconcile(cx)
                            } else {
                                Ok(())
                            }
                        }) {
                            eprintln!("Dojo wallpaper layout update failed: {error:#}");
                        }
                        continue;
                    }
                    let digest = source.digest;
                    let cached = cx
                        .background_executor()
                        .spawn(async move { load_cached(&digest) })
                        .await;
                    if cached.is_empty() {
                        let resource = gpui::Resource::Path(source.path.clone().into());
                        let loading = cx.update(|cx| {
                            let future = gpui::ImageAssetLoader::load(resource, cx);
                            cx.background_executor().spawn(future)
                        });
                        match loading.await {
                            Ok(image) => {
                                crate::startup::mark("wallpaper-decoded");
                                Some(ReadyWallpaper {
                                    source,
                                    image: Some(image),
                                    cached,
                                })
                            }
                            Err(error) => {
                                eprintln!("Dojo retained its previous wallpaper: {error:#}");
                                continue;
                            }
                        }
                    } else {
                        crate::startup::mark("wallpaper-cached");
                        Some(ReadyWallpaper {
                            source,
                            image: None,
                            cached,
                        })
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
                if controller.selection.update(ready) || controller.layout_pending() {
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
        let rendered = monitor_wallpaper(&source, 40, 20, None);
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
        assert_eq!(
            monitor_wallpaper(&small, 1920, 1080, None).size(0),
            small.size(0)
        );
    }

    #[test]
    fn a_region_crops_the_source_before_the_cover_fit() {
        // Left half black, right half white: each half of a spanned pair shows one colour.
        let mut pixels = image::RgbaImage::new(200, 100);
        for (x, _, pixel) in pixels.enumerate_pixels_mut() {
            let value = if x < 100 { 0 } else { 255 };
            *pixel = image::Rgba([value, value, value, 255]);
        }
        let source = gpui::RenderImage::new(vec![image::Frame::new(pixels)]);
        let region = |left| {
            Some(Region {
                left,
                top: 0.,
                width: 0.5,
                height: 1.,
            })
        };
        let first = monitor_wallpaper(&source, 50, 50, region(0.));
        let second = monitor_wallpaper(&source, 50, 50, region(0.5));
        assert!(
            first
                .as_bytes(0)
                .unwrap()
                .chunks_exact(4)
                .all(|pixel| pixel[0] == 0)
        );
        assert!(
            second
                .as_bytes(0)
                .unwrap()
                .chunks_exact(4)
                .all(|pixel| pixel[0] == 255)
        );
    }

    fn ready(source: Option<WallpaperSource>) -> Option<ReadyWallpaper> {
        source.map(|source| ReadyWallpaper {
            source,
            image: Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::new(1, 1),
            )]))),
            cached: Vec::new(),
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
