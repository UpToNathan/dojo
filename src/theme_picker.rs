use std::io::Read as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use dojo::extraction::Mode;
use dojo::procedural::{Palette, Recipe, Style, parse_colors};
use dojo::theme::wallpaper_layout::WallpaperLayout;
use dojo::theme::{Appearance, Paths, Preset, Selection, SemanticPalette, Theme, ThemeStore};
use gpui::{
    App, AppContext, Context, FocusHandle, KeyDownEvent, MouseButton, Render, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, img,
    layer_shell, point, prelude::*, px,
};

use crate::config::ThemeColors;

mod portal;

#[derive(Clone, Debug, Default, PartialEq, gpui::Action)]
#[action(namespace = theme_picker, no_json)]
pub struct ToggleThemePicker {
    pub display_id: Option<gpui::DisplayId>,
}

struct Candidate {
    mode: Mode,
    dark: SemanticPalette,
    light: SemanticPalette,
}
struct Analysis {
    path: PathBuf,
    bytes: Arc<Vec<u8>>,
    preview: Arc<gpui::RenderImage>,
    candidates: Vec<Candidate>,
    appearance: Appearance,
    /// Set when the wallpaper is generated: `path` and `bytes` are then its reference picture,
    /// and publishing it keeps it a recipe rather than a fixed image.
    recipe: Option<Recipe>,
}

fn analyze(preview_edge: u32) -> anyhow::Result<Analysis> {
    let store = ThemeStore::new(Paths::user_default()?);
    let selection = store.selection()?.context("Choose an image to preview.")?;
    let theme = store.theme(&selection.generation)?;
    let recipe = theme.recipe.clone();
    let snapshot = store
        .paths
        .root
        .join("generations")
        .join(selection.generation)
        .join("wallpaper");
    let path = if snapshot.is_file() {
        snapshot
    } else if !theme.wallpaper.is_empty() {
        PathBuf::from(theme.wallpaper)
    } else {
        anyhow::bail!("Choose an image to preview.")
    };
    let mut analysis = analyze_image(path, preview_edge)?;
    analysis.recipe = recipe;
    Ok(analysis)
}

fn analyze_image(path: PathBuf, preview_edge: u32) -> anyhow::Result<Analysis> {
    let path = path.canonicalize()?;
    anyhow::ensure!(path.is_file(), "wallpaper must be a regular image file");
    let mut file = std::fs::File::open(&path)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(50 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 50 * 1024 * 1024, "wallpaper exceeds 50 MiB");
    analyze_bytes(path, bytes, preview_edge)
}

fn analyze_bytes(path: PathBuf, bytes: Vec<u8>, preview_edge: u32) -> anyhow::Result<Analysis> {
    let reader = image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format()?;
    anyhow::ensure!(
        matches!(
            reader.format(),
            Some(image::ImageFormat::Jpeg | image::ImageFormat::Png | image::ImageFormat::WebP)
        ),
        "wallpaper must be JPEG, PNG, or WebP"
    );
    let (width, height) = reader.into_dimensions()?;
    anyhow::ensure!(
        width <= 16_384
            && height <= 16_384
            && u64::from(width) * u64::from(height) <= dojo::theme::MAX_WALLPAPER_PIXELS,
        "wallpaper exceeds image dimension limits"
    );
    let preview = wallpaper_thumbnail(&bytes, preview_edge)?;
    Ok(Analysis {
        path,
        bytes: Arc::new(bytes),
        preview,
        candidates: Vec::new(),
        appearance: dojo::theme::system_appearance().unwrap_or(Appearance::Dark),
        recipe: None,
    })
}

/// Where a generated wallpaper's colours come from.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PaletteSource {
    /// The selected theme's own colours, so the wallpaper belongs with the desktop.
    Theme,
    /// Chosen by the seed, unrelated to the theme.
    Random,
    /// Colours the user pasted.
    Pasted(Palette),
}

impl PaletteSource {
    /// The palette for a wallpaper with `seed`. `shell` is what to fall back on when no theme has
    /// been selected: the colours the desktop is currently drawn in.
    fn palette(&self, seed: u64, shell: &str) -> Palette {
        match self {
            Self::Random => Palette::from_seed(seed),
            Self::Pasted(palette) => palette.clone(),
            Self::Theme => selected_theme_palette()
                .or_else(|| Palette::from_colors(&parse_colors(shell)).ok())
                .unwrap_or_else(|| Palette::from_seed(seed)),
        }
    }
}

fn selected_theme_palette() -> Option<Palette> {
    let store = ThemeStore::new(Paths::user_default().ok()?);
    let selection = store.selection().ok()??;
    let theme = store.theme(&selection.generation).ok()?;
    Palette::from_theme(theme.palette(selection.appearance)).ok()
}

/// A generated wallpaper's reference picture, ready to preview and to derive a palette from.
fn analyze_recipe(recipe: Recipe, preview_edge: u32) -> anyhow::Result<Analysis> {
    let path = std::path::Path::new("/dojo/procedural").join(format!(
        "{}-{}.png",
        recipe.style.name(),
        recipe.seed
    ));
    let bytes = recipe.reference_png(dojo::procedural::desktop::current_aspect(), 1920)?;
    let mut analysis = analyze_bytes(path, bytes, preview_edge)?;
    analysis.recipe = Some(recipe);
    Ok(analysis)
}

fn wallpaper_thumbnail(bytes: &[u8], preview_edge: u32) -> anyhow::Result<Arc<gpui::RenderImage>> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    use image::ImageDecoder as _;
    if reader.format() == Some(image::ImageFormat::WebP) {
        use image::AnimationDecoder as _;
        let mut decoder = image::codecs::webp::WebPDecoder::new(std::io::Cursor::new(bytes))?;
        if decoder.has_animation() {
            let orientation = decoder.orientation()?;
            return animated_thumbnail(decoder.into_frames(), orientation, preview_edge);
        }
    }
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;
    let image = image::DynamicImage::from_decoder(decoder)?;
    Ok(Arc::new(gpui::RenderImage::new(vec![thumbnail_frame(
        image,
        orientation,
        preview_edge,
        image::Delay::from_numer_denom_ms(100, 1),
    )])))
}

fn thumbnail_frame(
    mut image: image::DynamicImage,
    orientation: image::metadata::Orientation,
    preview_edge: u32,
    delay: image::Delay,
) -> image::Frame {
    image.apply_orientation(orientation);
    let width = preview_edge.max(1).min(image.width());
    let height = preview_edge.max(1).min(image.height());
    let mut pixels = image.thumbnail(width, height).into_rgba8();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    image::Frame::from_parts(pixels, 0, 0, delay)
}

fn animated_thumbnail(
    frames: image::Frames<'_>,
    orientation: image::metadata::Orientation,
    preview_edge: u32,
) -> anyhow::Result<Arc<gpui::RenderImage>> {
    let mut thumbnails = Vec::new();
    let mut retained_bytes = 0;
    for frame in frames {
        if thumbnails.len() == 32 {
            thumbnails.truncate(1);
            break;
        }
        let frame = frame?;
        let delay = frame.delay();
        let thumbnail = thumbnail_frame(
            image::DynamicImage::ImageRgba8(frame.into_buffer()),
            orientation,
            preview_edge.min(2048),
            delay,
        );
        retained_bytes += thumbnail.buffer().len();
        if retained_bytes > 16 * 1024 * 1024 {
            thumbnails.truncate(1);
            break;
        }
        thumbnails.push(thumbnail);
    }
    anyhow::ensure!(
        !thumbnails.is_empty(),
        "animated wallpaper has no preview frame"
    );
    Ok(Arc::new(gpui::RenderImage::new(thumbnails)))
}

fn preview_edge(window: &Window) -> u32 {
    let width = (f32::from(window.viewport_size().width) - 48.).clamp(320., 940.);
    let preview_width = width - (width * 0.43).min(400.) - 66.;
    (preview_width.max(300.) * window.scale_factor())
        .ceil()
        .clamp(1., 4096.) as u32
}

fn derive_candidates(bytes: &[u8]) -> anyhow::Result<Vec<Candidate>> {
    dojo::theme::derive_wallpaper_modes(bytes)?
        .into_iter()
        .map(|theme| {
            Ok(Candidate {
                mode: theme.mode,
                dark: theme.dark,
                light: theme.light,
            })
        })
        .collect()
}

#[derive(Default)]
pub struct ThemePicker {
    window: Option<WindowHandle<PickerView>>,
}

impl ThemePicker {
    pub fn toggle(
        &mut self,
        display_id: Option<gpui::DisplayId>,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        if let Some(window) = self.window.take() {
            if window
                .update(cx, |view, window, _| view.close(window))
                .is_ok()
            {
                return Ok(());
            }
        }
        let display = display_id
            .and_then(|id| cx.displays().into_iter().find(|display| display.id() == id))
            .or_else(|| cx.primary_display())
            .or_else(|| cx.displays().into_iter().next())
            .context("no display for theme picker")?;
        let presets = dojo::theme::presets()?;
        self.window = Some(cx.open_window(
            WindowOptions {
                display_id: Some(display.id()),
                window_bounds: Some(WindowBounds::Windowed(display.bounds())),
                titlebar: None,
                app_id: Some("dojo".into()),
                window_background: WindowBackgroundAppearance::Transparent,
                kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                    namespace: "dojo-theme-picker".into(),
                    layer: layer_shell::Layer::Overlay,
                    anchor: layer_shell::Anchor::TOP
                        | layer_shell::Anchor::BOTTOM
                        | layer_shell::Anchor::LEFT
                        | layer_shell::Anchor::RIGHT,
                    exclusive_zone: Some(px(-1.)),
                    // The portal dialog must be able to take keyboard focus.
                    keyboard_interactivity: layer_shell::KeyboardInteractivity::OnDemand,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    cx.on_release_in(window, |view: &mut PickerView, window, _| {
                        view.file_request.take();
                        view.release_preview(window);
                    })
                    .detach();
                    let preview_edge = preview_edge(window);
                    cx.spawn_in(window, async move |this, cx| {
                        let result = cx
                            .background_executor()
                            .spawn(async move { analyze(preview_edge) })
                            .await;
                        if let Err(error) =
                            this.update_in(cx, |view: &mut PickerView, window, cx| {
                                if view.analysis_revision != 0 {
                                    return;
                                }
                                view.loading = false;
                                match result {
                                    Ok(analysis) => {
                                        if !view.show_presets {
                                            view.appearance = analysis.appearance;
                                        }
                                        view.replace_analysis(analysis, window);
                                        if view.show_wallpaper {
                                            view.message =
                                                "Choose an image, or keep the current wallpaper."
                                                    .into();
                                        } else if !view.show_presets {
                                            view.derive(window, cx);
                                        }
                                    }
                                    Err(error) => view.message = format!("{error:#}"),
                                }
                                cx.notify();
                            })
                        {
                            eprintln!("Theme picker closed during analysis: {error:#}");
                        }
                    })
                    .detach();
                    let saved_layout = dojo::theme::Paths::user_default()
                        .and_then(|paths| WallpaperLayout::read(&paths.root))
                        .unwrap_or_default();
                    PickerView {
                        focus,
                        analysis: None,
                        selected: None,
                        presets,
                        show_presets: false,
                        appearance: Appearance::Dark,
                        loading: true,
                        applying: false,
                        message: "Choose a wallpaper. No desktop changes until Apply.".into(),
                        panel: None,
                        show_wallpaper: true,
                        layout: saved_layout,
                        saved_layout,
                        image_chosen: false,
                        generated: None,
                        palette_source: PaletteSource::Theme,
                        file_request: None,
                        file_request_revision: 0,
                        file_request_error: None,
                        analysis_revision: 0,
                        reveal: crate::design::MenuReveal::opening_for(
                            std::time::Instant::now(),
                            std::time::Duration::from_millis(250),
                        ),
                    }
                })
            },
        )?);
        Ok(())
    }
}

struct PickerView {
    focus: FocusHandle,
    analysis: Option<Arc<Analysis>>,
    selected: Option<usize>,
    presets: Vec<Preset>,
    show_presets: bool,
    appearance: Appearance,
    loading: bool,
    applying: bool,
    message: String,
    panel: Option<gpui::Bounds<gpui::Pixels>>,
    show_wallpaper: bool,
    /// The layout shown as selected; saved only when Apply runs.
    layout: WallpaperLayout,
    saved_layout: WallpaperLayout,
    /// An image was chosen this session, as opposed to the current wallpaper loaded at open.
    image_chosen: bool,
    /// The style of the wallpaper generated this session, when the preview is one.
    generated: Option<(Style, u64)>,
    palette_source: PaletteSource,
    file_request: Option<portal::Request>,
    file_request_revision: u64,
    file_request_error: Option<String>,
    analysis_revision: u64,
    reveal: crate::design::MenuReveal,
}

fn confirmable(selected: Option<usize>, count: usize, loading: bool, applying: bool) -> bool {
    !loading && !applying && selected.is_some_and(|index| index < count)
}

/// What Apply wallpaper does: the layout to save (when it differs from the saved one) and whether
/// to publish the wallpaper image. A layout-only change leaves the image and theme untouched; with
/// nothing changed Apply republishes the current wallpaper, as it always has.
fn wallpaper_apply_plan(
    layout: WallpaperLayout,
    saved: WallpaperLayout,
    image_chosen: bool,
) -> (Option<WallpaperLayout>, bool) {
    let layout_to_save = (layout != saved).then_some(layout);
    (layout_to_save, image_chosen || layout_to_save.is_none())
}

fn picker_targets() -> Vec<String> {
    dojo::theme::TARGETS
        .iter()
        .map(|target| (*target).to_string())
        .collect()
}

fn apply_results(report: &dojo::theme::ApplyReport) -> String {
    report
        .targets
        .iter()
        .map(|target| {
            let status = match target.status.as_str() {
                "published" => "theme published",
                "reload_requested" => "live reload sent",
                "ready" => "ready for next launch",
                "needs_selection" => "select Dojo once in the app",
                "needs_setup" => "one-time setup required",
                "blocked" => "blocked",
                "reload_failed" => "reload failed",
                "unsupported" => "not integrated",
                other => other,
            };
            if matches!(
                target.status.as_str(),
                "blocked" | "reload_failed" | "needs_setup" | "unsupported"
            ) {
                format!("{}: {status} — {}", target.target, target.detail)
            } else {
                format!("{}: {status}", target.target)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn theme_content_row() -> gpui::Div {
    // Flex's automatic minimum height would otherwise fit the entire preset list,
    // pushing its scroll viewport and the footer outside the fixed-height panel.
    div().flex_1().min_h(px(0.)).flex().gap(px(22.))
}

impl PickerView {
    fn release_preview(&mut self, window: &mut Window) {
        if let Some(analysis) = self.analysis.take() {
            if let Err(error) = window.drop_image(analysis.preview.clone()) {
                eprintln!("Could not release wallpaper preview: {error:#}");
            }
        }
    }

    fn replace_analysis(&mut self, analysis: Analysis, window: &mut Window) {
        self.release_preview(window);
        self.analysis = Some(Arc::new(analysis));
    }

    fn close(&mut self, window: &mut Window) {
        self.analysis_revision += 1;
        self.file_request_revision += 1;
        self.file_request.take();
        window.set_input_region(None);
        self.release_preview(window);
        self.reveal.set_open(false, std::time::Instant::now());
        window.refresh();
    }

    fn derive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || self.applying {
            return;
        }
        let Some(analysis) = self.analysis.clone() else {
            return;
        };
        if !analysis.candidates.is_empty() {
            return;
        }
        self.analysis_revision += 1;
        let revision = self.analysis_revision;
        self.loading = true;
        self.message = "Preparing derivation previews. Nothing is applied.".into();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let bytes = analysis.bytes.clone();
            let result = cx
                .background_executor()
                .spawn(async move { derive_candidates(bytes.as_ref()) })
                .await;
            if let Err(error) = this.update_in(cx, |view, _, cx| {
                if view.analysis_revision != revision {
                    return;
                }
                view.loading = false;
                match result {
                    Ok(candidates) => {
                        view.analysis = Some(Arc::new(Analysis {
                            path: analysis.path.clone(),
                            bytes: analysis.bytes.clone(),
                            preview: analysis.preview.clone(),
                            candidates,
                            appearance: analysis.appearance,
                            recipe: analysis.recipe.clone(),
                        }));
                        if !view.show_wallpaper && !view.show_presets {
                            view.message =
                                "Choose a derivation attempt to preview. Nothing is applied."
                                    .into();
                        }
                    }
                    Err(error) => view.message = format!("Could not derive a palette: {error:#}"),
                }
                cx.notify();
            }) {
                eprintln!("Theme picker closed during derivation: {error:#}");
            }
        })
        .detach();
    }

    /// Like an image, the layout is only previewed here; Apply wallpaper saves it.
    fn set_layout(&mut self, layout: WallpaperLayout, cx: &mut Context<Self>) {
        if self.layout == layout || self.applying {
            return;
        }
        self.layout = layout;
        self.message = "Preview only. Apply wallpaper saves the layout.".into();
        cx.notify();
    }

    fn choose_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.choose_image_with(window, cx, portal::Request::start);
    }

    fn choose_image_with(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        start: impl FnOnce() -> anyhow::Result<(
            portal::Request,
            async_channel::Receiver<anyhow::Result<Option<PathBuf>>>,
        )>,
    ) {
        if self.file_request.is_some() || self.loading || self.applying || !self.reveal.is_open() {
            return;
        }
        self.file_request_error = None;
        match start() {
            Ok((request, response)) => {
                self.file_request_revision += 1;
                let revision = self.file_request_revision;
                self.file_request = Some(request);
                window.set_input_region(Some(&[]));
                cx.spawn_in(window, async move |this, cx| {
                    let result = response.recv().await.unwrap_or_else(|error| {
                        Err(anyhow::anyhow!("File picker stopped: {error}"))
                    });
                    let _ = this.update_in(cx, |view, window, cx| {
                        view.finish_image_request(revision, result, window, cx);
                    });
                })
                .detach();
            }
            Err(error) => {
                self.file_request_error = Some(format!("Could not open file picker: {error:#}"))
            }
        }
        cx.notify();
    }

    fn finish_image_request(
        &mut self,
        revision: u64,
        result: anyhow::Result<Option<PathBuf>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.file_request_revision != revision || !self.reveal.is_open() {
            return;
        }
        self.file_request.take();
        window.set_input_region(None);
        window.activate_window();
        window.focus(&self.focus, cx);
        match result {
            Ok(Some(path)) => self.choose_wallpaper(path, window, cx),
            Ok(None) => {} // Cancel leaves the preview and selection unchanged.
            Err(error) => {
                self.file_request_error = Some(format!("Could not choose image: {error:#}"))
            }
        }
        cx.notify();
    }

    /// Draws a wallpaper of `style` to preview, with a new seed unless one is given. It is a canvas
    /// across every monitor, so the layout is preset to span; it stays a choice until Apply.
    fn generate(
        &mut self,
        style: Style,
        seed: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.applying || self.file_request.is_some() || !self.reveal.is_open() {
            return;
        }
        self.analysis_revision += 1;
        let revision = self.analysis_revision;
        self.loading = true;
        self.selected = None;
        self.release_preview(window);
        self.message = format!("Drawing {}…", style.label());
        cx.notify();
        let preview_edge = preview_edge(window);
        let seed = seed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |elapsed| elapsed.as_nanos() as u64)
        });
        let source = self.palette_source.clone();
        let colors = *cx.global::<ThemeColors>();
        let shell = format!(
            "{} {} {}",
            colors.background.hex(),
            colors.primary.hex(),
            colors.foreground.hex()
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let palette = source.palette(seed, &shell);
                    analyze_recipe(Recipe::with_palette(style, seed, palette), preview_edge)
                })
                .await;
            if let Err(error) = this.update_in(cx, |view, window, cx| {
                if view.analysis_revision != revision {
                    return;
                }
                view.loading = false;
                match result {
                    Ok(analysis) => {
                        view.generated = Some((style, seed));
                        view.replace_analysis(analysis, window);
                        view.image_chosen = true;
                        view.layout = WallpaperLayout::Span;
                        view.message =
                            "Preview only. Apply wallpaper keeps your selected theme.".into();
                        if !view.show_wallpaper && !view.show_presets {
                            view.derive(window, cx);
                        }
                    }
                    Err(error) => view.message = format!("Could not draw wallpaper: {error:#}"),
                }
                cx.notify();
            }) {
                eprintln!("Wallpaper picker closed: {error:#}");
            }
        })
        .detach();
    }

    /// Chooses where generated colours come from and, when a wallpaper is already drawn, draws it
    /// again in them so the colours can be compared on the same picture.
    fn set_palette_source(
        &mut self,
        source: PaletteSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.applying || self.loading {
            return;
        }
        self.palette_source = source;
        match self.generated {
            Some((style, seed)) => self.generate(style, Some(seed), window, cx),
            None => {
                self.message = "Colours chosen. Pick a style to draw a wallpaper.".into();
                cx.notify();
            }
        }
    }

    /// Takes a palette from the clipboard: any text with at least two `#rrggbb` colours.
    fn paste_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        match Palette::from_colors(&parse_colors(&text)) {
            Ok(palette) => self.set_palette_source(PaletteSource::Pasted(palette), window, cx),
            Err(error) => {
                self.message = format!("Copy colours such as #0b0b14 #5b6ee1 first. {error}");
                cx.notify();
            }
        }
    }

    fn choose_wallpaper(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.applying {
            return;
        }
        self.analysis_revision += 1;
        let revision = self.analysis_revision;
        self.loading = true;
        self.selected = None;
        self.release_preview(window);
        self.message = "Loading wallpaper preview…".into();
        cx.notify();
        let preview_edge = preview_edge(window);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { analyze_image(path, preview_edge) })
                .await;
            if let Err(error) = this.update_in(cx, |view, window, cx| {
                if view.analysis_revision != revision {
                    return;
                }
                view.loading = false;
                match result {
                    Ok(analysis) => {
                        view.generated = None;
                        view.replace_analysis(analysis, window);
                        view.image_chosen = true;
                        view.message =
                            "Preview only. Apply wallpaper keeps your selected theme.".into();
                        if !view.show_wallpaper && !view.show_presets {
                            view.derive(window, cx);
                        }
                    }
                    Err(error) => view.message = format!("Could not load wallpaper: {error:#}"),
                }
                cx.notify();
            }) {
                eprintln!("Wallpaper picker closed: {error:#}");
            }
        })
        .detach();
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_request.is_some() || !self.reveal.is_open() {
            return;
        }
        if self.show_wallpaper {
            if self.loading || self.applying {
                return;
            }
            let Some(analysis) = self.analysis.clone() else {
                return;
            };
            let colors = *cx.global::<ThemeColors>();
            let (layout_to_save, apply_image) =
                wallpaper_apply_plan(self.layout, self.saved_layout, self.image_chosen);
            self.applying = true;
            self.message = "Applying wallpaper without changing your theme…".into();
            cx.notify();
            cx.spawn_in(window, async move |this, cx| {
                let (layout_saved, result) = cx
                    .background_executor()
                    .spawn(async move {
                        let store = ThemeStore::new(Paths::user_default()?);
                        let mut layout_saved = false;
                        let result = (|| {
                            if let Some(layout) = layout_to_save {
                                layout.write(&store.paths.root)?;
                                layout_saved = true;
                            }
                            if !apply_image {
                                return Ok(None);
                            }
                            let fallback = if store.selection()?.is_none() {
                                Some(colors.semantic_palette(dojo::theme::system_appearance()?)?)
                            } else {
                                None
                            };
                            match analysis.recipe.clone() {
                                Some(recipe) => dojo::theme::apply_procedural(
                                    &store,
                                    recipe,
                                    dojo::procedural::desktop::current_aspect(),
                                    fallback,
                                ),
                                None => dojo::theme::apply_wallpaper(
                                    &store,
                                    &analysis.path,
                                    analysis.bytes.as_ref().clone(),
                                    fallback,
                                ),
                            }
                            .map(Some)
                        })();
                        Ok::<_, anyhow::Error>((layout_saved, result))
                    })
                    .await
                    .unwrap_or_else(|error| (false, Err(error)));
                if let Err(error) = this.update_in(cx, |view, _, cx| {
                    view.applying = false;
                    if layout_saved {
                        view.saved_layout = view.layout;
                    }
                    view.message = match result {
                        Ok(Some(report)) => format!(
                            "Wallpaper selected. Theme and darkman mode unchanged.\n{}",
                            apply_results(&report)
                        ),
                        Ok(None) => "Wallpaper layout saved.".into(),
                        Err(error) => format!("Could not apply wallpaper: {error:#}"),
                    };
                    cx.notify();
                }) {
                    eprintln!("Wallpaper picker closed after apply: {error:#}");
                }
            })
            .detach();
            return;
        }
        if self.show_presets {
            if !confirmable(self.selected, self.presets.len(), false, self.applying) {
                return;
            }
            let Some(preset) = self
                .selected
                .and_then(|index| self.presets.get(index))
                .cloned()
            else {
                return;
            };
            self.applying = true;
            self.message = format!(
                "Applying {} and switching darkman to {}…",
                preset.name,
                preset.appearance.name()
            );
            cx.notify();
            cx.spawn_in(window, async move |this, cx| {
                let name = preset.name.clone();
                let appearance = preset.appearance;
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let store = ThemeStore::new(Paths::user_default()?);
                        dojo::theme::apply_preset(&store, &preset, picker_targets())
                    })
                    .await;
                if let Err(error) = this.update_in(cx, |view, _, cx| {
                    view.applying = false;
                    view.message = match result {
                        Ok(report) => format!(
                            "Selected {name}. Darkman is {}. Wallpaper unchanged.\n{}",
                            appearance.name(),
                            apply_results(&report)
                        ),
                        Err(error) => format!("Could not apply: {error:#}"),
                    };
                    cx.notify();
                }) {
                    eprintln!("Theme picker closed after preset apply: {error:#}");
                }
            })
            .detach();
            return;
        }
        let Some(analysis) = self.analysis.clone() else {
            return;
        };
        if !confirmable(
            self.selected,
            analysis.candidates.len(),
            self.loading,
            self.applying,
        ) {
            return;
        }
        let Some(candidate) = self
            .selected
            .and_then(|index| analysis.candidates.get(index))
        else {
            return;
        };
        let mode = candidate.mode;
        let appearance = self.appearance;
        self.applying = true;
        self.message = format!("Applying {} theme…", appearance.name());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut theme = Theme::from_snapshot(
                        &analysis.path,
                        analysis.bytes.as_ref().clone(),
                        mode,
                    )?;
                    // A generated wallpaper stays one: only its palette is derived here.
                    theme.recipe = analysis.recipe.clone();
                    let store = ThemeStore::new(Paths::user_default()?);
                    let generation = store.generate(&theme)?;
                    dojo::theme::set_system_appearance(appearance)?;
                    dojo::theme::apply(
                        &store,
                        Selection {
                            generation,
                            appearance,
                            follow_system: false,
                            targets: picker_targets(),
                        },
                    )
                })
                .await;
            if let Err(error) = this.update_in(cx, |view, _, cx| {
                view.applying = false;
                view.message = match result {
                    Ok(report) => format!(
                        "Selected {}. Darkman is {}.\n{}",
                        mode.name(),
                        appearance.name(),
                        apply_results(&report)
                    ),
                    Err(error) => format!("Could not apply: {error:#}"),
                };
                cx.notify();
            }) {
                eprintln!("Theme picker closed after apply: {error:#}");
            }
        })
        .detach();
    }
}

impl Render for PickerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (progress, moving) = self
            .reveal
            .sample_with_motion(std::time::Instant::now(), cx.reduce_motion());
        if moving {
            window.request_animation_frame();
        } else if !self.reveal.is_open() {
            self.release_preview(window);
            window.remove_window();
        }
        let choosing = self.file_request.is_some();
        let colors = *cx.global::<ThemeColors>();
        let width = (f32::from(window.viewport_size().width) - 24.).clamp(1., 940.);
        let height = (f32::from(window.viewport_size().height)
            - crate::geometry::TOP_BAR_HEIGHT as f32
            - 24.)
            .clamp(1., 720.);
        let narrow = width < 650.;
        let modes_width = if narrow {
            width - 32.
        } else {
            (width * 0.43).min(400.)
        };
        let mut mode_buttons = Vec::new();
        if self.show_wallpaper {
            let choosing = self.file_request.is_some();
            let enabled = !choosing && !self.loading && !self.applying;
            mode_buttons.push(
                div()
                    .id("choose-wallpaper-image")
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(crate::design::PILL_RADIUS))
                    .bg(colors.background_highlight.rgb())
                    .text_color(colors.selection_foreground.rgb())
                    .when(enabled, |button| button.cursor_pointer())
                    .child(if choosing {
                        "Choosing image…"
                    } else if self.loading {
                        "Loading preview…"
                    } else {
                        "Choose image…"
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, window, cx| view.choose_image(window, cx)),
                    ),
            );
            let mut generate_buttons = div().id("wallpaper-generate").w_full().flex().flex_wrap().gap(px(8.));
            for (index, style) in Style::ALL.into_iter().enumerate() {
                let selected = self.generated.is_some_and(|(drawn, _)| drawn == style);
                generate_buttons = generate_buttons.child(
                    div()
                        .id(("wallpaper-generate-style", index))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(crate::design::PILL_RADIUS))
                        .when(enabled, |button| button.cursor_pointer())
                        .bg(if selected {
                            colors.primary.rgb()
                        } else {
                            colors.background_highlight.rgb()
                        })
                        .text_color(if selected {
                            colors.primary_foreground.rgb()
                        } else {
                            colors.selection_foreground.rgb()
                        })
                        .child(style.label())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, window, cx| view.generate(style, None, window, cx)),
                        ),
                );
            }
            mode_buttons.push(
                div()
                    .id("wallpaper-generate-hint")
                    .w_full()
                    .text_color(colors.foreground_muted.rgb())
                    .child("Or draw one: each click makes a new picture at every monitor's own resolution, in your theme's colours, random ones, or a palette you copied."),
            );
            mode_buttons.push(generate_buttons);
            let mut colour_buttons = div().id("wallpaper-colours").w_full().flex().flex_wrap().gap(px(8.));
            for (index, label) in ["Theme colours", "Random colours", "Paste palette"].into_iter().enumerate() {
                let selected = match index {
                    0 => self.palette_source == PaletteSource::Theme,
                    1 => self.palette_source == PaletteSource::Random,
                    _ => matches!(self.palette_source, PaletteSource::Pasted(_)),
                };
                colour_buttons = colour_buttons.child(
                    div()
                        .id(("wallpaper-colour-source", index))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(crate::design::PILL_RADIUS))
                        .cursor_pointer()
                        .bg(if selected {
                            colors.primary.rgb()
                        } else {
                            colors.background_highlight.rgb()
                        })
                        .text_color(if selected {
                            colors.primary_foreground.rgb()
                        } else {
                            colors.selection_foreground.rgb()
                        })
                        .child(label)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, window, cx| match index {
                                0 => view.set_palette_source(PaletteSource::Theme, window, cx),
                                1 => view.set_palette_source(PaletteSource::Random, window, cx),
                                _ => view.paste_palette(window, cx),
                            }),
                        ),
                );
            }
            mode_buttons.push(colour_buttons);
            let mut layout_buttons = div().id("wallpaper-layout").w_full().flex().gap(px(8.));
            for (index, (layout, label)) in [
                (WallpaperLayout::Fill, "Each monitor"),
                (WallpaperLayout::Span, "Span all monitors"),
            ]
            .into_iter()
            .enumerate()
            {
                let selected = self.layout == layout;
                layout_buttons = layout_buttons.child(
                    div()
                        .id(("wallpaper-layout-option", index))
                        .px(px(12.))
                        .py(px(8.))
                        .rounded(px(crate::design::PILL_RADIUS))
                        .cursor_pointer()
                        .bg(if selected {
                            colors.primary.rgb()
                        } else {
                            colors.background_highlight.rgb()
                        })
                        .text_color(if selected {
                            colors.primary_foreground.rgb()
                        } else {
                            colors.selection_foreground.rgb()
                        })
                        .child(label)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, _, cx| view.set_layout(layout, cx)),
                        ),
                );
            }
            mode_buttons.push(layout_buttons);
            mode_buttons.push(div().id("wallpaper-picker-hint").w_full().text_color(colors.foreground_muted.rgb()).child("Choose a JPEG, PNG, or WebP with the system file picker. Preview it here, then Apply."));
            if let Some(error) = &self.file_request_error {
                mode_buttons.push(
                    div()
                        .id("wallpaper-picker-error")
                        .w_full()
                        .child(error.clone()),
                );
            }
        } else if self.show_presets {
            let mut previous_family = "";
            for (index, preset) in self.presets.iter().enumerate() {
                if preset.family != previous_family {
                    mode_buttons.push(
                        div()
                            .id(("preset-family", index))
                            .w_full()
                            .mt(px(10.))
                            .text_color(colors.foreground_muted.rgb())
                            .child(preset.family.clone()),
                    );
                    previous_family = &preset.family;
                }
                let appearance = preset.appearance;
                mode_buttons.push(div().id(("static-preset",index)).w_full().h(px(34.)).px(px(8.)).flex().items_center().justify_between().rounded(px(6.)).cursor_pointer().bg(if self.selected == Some(index) {colors.background_highlight.rgb()} else {colors.background.rgb()}).text_color(if self.selected == Some(index) { colors.selection_foreground.rgb() } else { colors.foreground.rgb() }).child(preset.name.clone()).child(div().text_size(px(10.)).child(appearance.name())).on_mouse_down(MouseButton::Left,cx.listener(move |view,_,_,cx| {
                    if view.applying {return;}
                    view.selected = Some(index);
                    view.appearance = appearance;
                    view.message = format!("Preview only. Apply switches darkman to {}; wallpaper unchanged.",appearance.name());
                    cx.notify();
                })));
            }
        } else {
            for (index, mode) in Mode::ALL.into_iter().enumerate() {
                mode_buttons.push(
                    div()
                        .id(("theme-mode", index))
                        .w(px((modes_width - 32.) / 2.))
                        .h(px(32.))
                        .px(px(8.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .bg(if self.selected == Some(index) {
                            colors.background_highlight.rgb()
                        } else {
                            colors.background.rgb()
                        })
                        .text_color(if self.selected == Some(index) {
                            colors.selection_foreground.rgb()
                        } else {
                            colors.foreground.rgb()
                        })
                        .child(mode.name())
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, _, cx| {
                                if view.loading || view.applying {
                                    return;
                                }
                                view.selected = Some(index);
                                view.message =
                                    "Preview only. Apply when you like the result.".into();
                                cx.notify();
                            }),
                        ),
                );
            }
        }
        let mut preview = div()
            .id("theme-mode-preview")
            .flex_1()
            .min_w(px(0.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(12.));
        if let Some(analysis) = &self.analysis {
            preview = preview.child(
                img(analysis.preview.clone())
                    .w_full()
                    .h(px(if narrow {
                        130.
                    } else if self.show_wallpaper {
                        300.
                    } else {
                        160.
                    }))
                    .object_fit(gpui::ObjectFit::Cover)
                    .rounded(px(10.)),
            );
        }
        if self.show_wallpaper {
            if let Some(analysis) = &self.analysis {
                preview = preview.child(div().text_size(px(12.)).child(format!("Selected · {}",
                    analysis.path.file_name().unwrap_or_default().to_string_lossy())));
            } else {
                preview = preview.child(
                    div()
                        .h(px(if narrow { 100. } else { 220. }))
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(colors.background_highlight.rgb())
                        .text_color(colors.selection_foreground.rgb())
                        .rounded(px(10.))
                        .child(if self.loading {
                            "Loading preview…"
                        } else {
                            "Select an image to preview"
                        }),
                );
            }
        }
        let selected_palette = if self.show_wallpaper {
            None
        } else if self.show_presets {
            self.selected
                .and_then(|index| self.presets.get(index))
                .map(|preset| (preset.name.clone(), &preset.palette))
        } else {
            self.analysis
                .as_ref()
                .and_then(|analysis| {
                    self.selected
                        .and_then(|index| analysis.candidates.get(index))
                })
                .map(|candidate| {
                    (
                        candidate.mode.name().to_string(),
                        match self.appearance {
                            Appearance::Dark => &candidate.dark,
                            Appearance::Light => &candidate.light,
                        },
                    )
                })
        };
        if let Some((name, p)) = selected_palette {
            let rgb = |role: &str| {
                gpui::rgb(
                    u32::from_str_radix(p.color(role).trim_start_matches('#'), 16).unwrap_or(0),
                )
            };
            let swatches = p
                .terminal
                .iter()
                .map(|color| {
                    div().flex_1().h(px(24.)).bg(gpui::rgb(
                        u32::from_str_radix(color.trim_start_matches('#'), 16).unwrap_or(0),
                    ))
                })
                .collect::<Vec<_>>();
            preview = preview.child(
                div()
                    .p(px(18.))
                    .rounded(px(12.))
                    .bg(rgb("background"))
                    .text_color(rgb("foreground"))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(format!("{} · {} preview", name, p.appearance.name()))
                    .child(div().flex().children(swatches))
                    .child(
                        div()
                            .p(px(12.))
                            .rounded(px(8.))
                            .bg(rgb("surface"))
                            .child("Body text on an application surface"),
                    )
                    .child(
                        div()
                            .p(px(10.))
                            .rounded(px(6.))
                            .bg(rgb("background_highlight"))
                            .text_color(rgb("selection_foreground"))
                            .child("Selected item / hover"),
                    )
                    .child(
                        div()
                            .text_color(rgb("foreground_muted"))
                            .child("Muted text and secondary information"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(div().text_color(rgb("primary")).child("Accent"))
                            .child(div().text_color(rgb("success")).child("Success"))
                            .child(div().text_color(rgb("warning")).child("Warning"))
                            .child(div().text_color(rgb("error")).child("Error")),
                    ),
            );
        } else {
            preview = preview.child(if self.show_wallpaper {
                "Choose an image from a folder. Apply wallpaper keeps your existing theme."
            } else {
                "Choose a theme to preview its colors."
            });
        }
        let mut appearance_buttons = div().flex().gap(px(8.));
        for appearance in [Appearance::Dark, Appearance::Light] {
            appearance_buttons = appearance_buttons.child(
                div()
                    .id(format!("theme-preview-{}", appearance.name()))
                    .cursor_pointer()
                    .px(px(12.))
                    .py(px(7.))
                    .rounded(px(crate::design::PILL_RADIUS))
                    .bg(if self.appearance == appearance {
                        colors.primary.rgb()
                    } else {
                        colors.background_highlight.rgb()
                    })
                    .text_color(if self.appearance == appearance {
                        colors.primary_foreground.rgb()
                    } else {
                        colors.selection_foreground.rgb()
                    })
                    .child(match appearance {
                        Appearance::Dark => "Dark",
                        Appearance::Light => "Light",
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            if view.applying {
                                return;
                            }
                            view.appearance = appearance;
                            cx.notify();
                        }),
                    ),
            );
        }
        if !self.show_presets && !self.show_wallpaper {
            preview = preview.child(appearance_buttons);
        }
        let ready = !choosing
            && if self.show_wallpaper {
                self.analysis.is_some() && !self.loading && !self.applying
            } else {
                confirmable(
                    self.selected,
                    if self.show_presets {
                        self.presets.len()
                    } else {
                        self.analysis
                            .as_ref()
                            .map_or(0, |analysis| analysis.candidates.len())
                    },
                    !self.show_presets && self.loading,
                    self.applying,
                )
            };
        let mut source_buttons = div().flex().gap(px(8.));
        for (show_wallpaper, show_presets, label) in [
            (true, false, "Wallpaper"),
            (false, true, "Presets"),
            (false, false, "Derive"),
        ] {
            source_buttons = source_buttons.child(
                div()
                    .id(if show_wallpaper {
                        "wallpaper-picker-source"
                    } else if show_presets {
                        "preset-source"
                    } else {
                        "wallpaper-source"
                    })
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(crate::design::PILL_RADIUS))
                    .cursor_pointer()
                    .bg(
                        if self.show_presets == show_presets
                            && self.show_wallpaper == show_wallpaper
                        {
                            colors.background_highlight.rgb()
                        } else {
                            colors.background.rgb()
                        },
                    )
                    .text_color(
                        if self.show_presets == show_presets
                            && self.show_wallpaper == show_wallpaper
                        {
                            colors.selection_foreground.rgb()
                        } else {
                            colors.foreground.rgb()
                        },
                    )
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, window, cx| {
                            if view.applying
                                || (view.show_presets == show_presets
                                    && view.show_wallpaper == show_wallpaper)
                            {
                                return;
                            }
                            view.show_presets = show_presets;
                            view.show_wallpaper = show_wallpaper;
                            view.selected = None;
                            view.message = "Choose a theme. No desktop changes until Apply.".into();
                            if show_wallpaper {
                                view.message =
                                    "Choose an image, or keep the current wallpaper.".into();
                            } else if !show_presets {
                                view.derive(window, cx);
                            }
                            cx.notify();
                        }),
                    ),
            );
        }
        let panel = div()
            .id("theme-picker-panel")
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .w(px(width))
            .h(px(height))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(14.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(20.)).child("Wallpaper & themes"))
            .child(source_buttons)
            .child(
                theme_content_row()
                    .when(narrow, |row| row.flex_col().gap(px(8.)))
                    .child(
                        div()
                            .w(px(modes_width))
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .when(narrow, |list| list.flex_1())
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                div()
                                    .id("theme-mode-list")
                                    .flex_1()
                                    .min_h(px(0.))
                                    .overflow_y_scroll()
                                    .flex()
                                    .flex_wrap()
                                    .content_start()
                                    .gap(px(8.))
                                    .children(mode_buttons),
                            ),
                    )
                    .child(preview),
            )
            .child(
                div()
                    .id("theme-apply-results")
                    .max_h(px(96.))
                    .flex_shrink_0()
                    .overflow_y_scroll()
                    .text_size(px(12.))
                    .text_color(colors.foreground_muted.rgb())
                    .child(self.message.clone()),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(11.))
                            .child("Preview only. Nothing changes until Apply."),
                    )
                    .child(
                        div()
                            .id("apply-theme-mode")
                            .px(px(18.))
                            .py(px(10.))
                            .rounded(px(crate::design::PILL_RADIUS))
                            .bg(if ready {
                                colors.primary.rgb()
                            } else {
                                colors.background_highlight.rgb()
                            })
                            .text_color(if ready {
                                colors.primary_foreground.rgb()
                            } else {
                                colors.selection_foreground.rgb()
                            })
                            .child(if self.applying {
                                "Applying…"
                            } else if self.show_wallpaper {
                                "Apply wallpaper"
                            } else {
                                "Apply theme"
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _, window, cx| view.apply(window, cx)),
                            ),
                    ),
            );
        self.panel = Some(gpui::Bounds {
            origin: point(
                (window.viewport_size().width - px(width)) / 2.,
                px(crate::geometry::TOP_BAR_HEIGHT as f32),
            ),
            size: gpui::Size::new(px(width), px(height)),
        });
        let mut scrim = colors.background.rgb();
        scrim.a = 0.18;
        div()
            .size_full()
            // Layer-shell overlays sit above normal portal windows: hide our
            // surface and let input through while the system dialog is open.
            .opacity(if choosing { 0. } else { 1. })
            .bg(scrim)
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .text_size(px(12.))
            .text_color(colors.foreground.rgb())
            .flex()
            .items_start()
            .justify_center()
            .pt(px(crate::geometry::TOP_BAR_HEIGHT as f32))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" if !view.applying => view.close(window),
                    "enter" if !event.is_held && view.file_request.is_none() => {
                        view.apply(window, cx)
                    }
                    _ => {}
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &gpui::MouseDownEvent, window, _| {
                    if !view.applying
                        && view
                            .panel
                            .is_some_and(|bounds| !bounds.contains(&event.position))
                    {
                        view.close(window);
                    }
                }),
            )
            .child(crate::design::shell_extension(
                gpui::Bounds::new(
                    gpui::point(
                        (window.viewport_size().width - px(width)) / 2.,
                        px(crate::geometry::TOP_BAR_HEIGHT as f32),
                    ),
                    gpui::size(px(width), px(height * progress)),
                ),
                colors.background.rgb(),
                colors.border.rgb(),
                false,
            ))
            .child(
                div()
                    .relative()
                    .w(px(width))
                    .h(px(height * progress))
                    .overflow_hidden()
                    .child(panel),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PaletteSource, analyze_recipe, confirmable, derive_candidates, theme_content_row,
        wallpaper_apply_plan,
    };
    use dojo::procedural::{Palette, Recipe, Style, parse_colors};
    use dojo::theme::wallpaper_layout::WallpaperLayout::{Fill, Span};
    use anyhow::Context as _;
    use gpui::{Styled, px};
    use std::path::PathBuf;

    #[gpui::test]
    fn picker_opens_on_the_requested_monitor(cx: &mut gpui::TestAppContext) {
        let external = gpui::DisplayId::from(2);
        cx.simulate_display_added(external, None);
        let mut picker = super::ThemePicker::default();
        cx.update(|cx| {
            cx.set_global(crate::config::ThemeColors::default());
            picker.toggle(Some(external), cx).unwrap();
            picker
                .window
                .unwrap()
                .update(cx, |_, window, cx| {
                    assert_eq!(window.display(cx).unwrap().id(), external);
                })
                .unwrap();
            picker.toggle(None, cx).unwrap();
        });
    }

    fn animated_webp(frame_count: usize) -> anyhow::Result<Vec<u8>> {
        fn chunk(output: &mut Vec<u8>, name: &[u8; 4], payload: &[u8]) {
            output.extend_from_slice(name);
            output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            output.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                output.push(0);
            }
        }
        let mut body = b"WEBP".to_vec();
        chunk(&mut body, b"VP8X", &[2, 0, 0, 0, 1, 0, 0, 1, 0, 0]);
        chunk(&mut body, b"ANIM", &[0; 6]);
        for index in 0..frame_count {
            let color = if index % 2 == 0 {
                [255, 0, 0]
            } else {
                [0, 0, 255]
            };
            let image = image::RgbImage::from_pixel(2, 2, image::Rgb(color));
            let mut encoded = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image)
                .write_to(&mut encoded, image::ImageFormat::WebP)?;
            let mut frame = vec![0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 50, 0, 0, 2];
            frame.extend_from_slice(&encoded.into_inner()[12..]);
            chunk(&mut body, b"ANMF", &frame);
        }
        let mut encoded = b"RIFF".to_vec();
        encoded.extend_from_slice(&(body.len() as u32).to_le_bytes());
        encoded.extend(body);
        Ok(encoded)
    }

    #[test]
    fn derived_apply_matches_all_mode_previews_and_exports_semantic_feedback() -> anyhow::Result<()>
    {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            8,
            8,
            image::Rgb([40, 160, 90]),
        ))
        .write_to(&mut bytes, image::ImageFormat::Png)?;
        for candidate in super::derive_candidates(bytes.get_ref())? {
            let theme = dojo::theme::Theme::from_snapshot(
                std::path::Path::new("/test-wallpaper.png"),
                bytes.get_ref().clone(),
                candidate.mode,
            )?;
            assert_eq!(theme.algorithm_revision, "dojo-semantic-wallpaper-2");
            for (appearance, preview) in [
                (dojo::theme::Appearance::Dark, candidate.dark),
                (dojo::theme::Appearance::Light, candidate.light),
            ] {
                assert_eq!(
                    serde_json::to_value(theme.palette(appearance))?,
                    serde_json::to_value(preview)?
                );
                let outputs = theme.outputs(appearance)?;
                let t3: serde_json::Value = serde_json::from_slice(&outputs["t3code.json"])?;
                let colors = &t3["variants"][appearance.name()];
                assert_eq!(
                    colors["warningSurface"],
                    theme.palette(appearance).color("warning_background")
                );
                assert_eq!(
                    colors["errorSurface"],
                    theme.palette(appearance).color("error_background")
                );
                assert_eq!(colors.as_object().unwrap().len(), 61);
            }
        }
        Ok(())
    }

    #[test]
    fn wallpaper_preview_animates_within_budget_and_keeps_first_frame_over_budget()
    -> anyhow::Result<()> {
        let preview = super::wallpaper_thumbnail(&animated_webp(2)?, 8)?;
        assert_eq!(preview.frame_count(), 2);
        assert_eq!(
            preview.size(0),
            gpui::size(gpui::DevicePixels(2), gpui::DevicePixels(2))
        );
        assert_eq!(
            preview.as_bytes(0).context("missing first frame")?,
            &[0, 0, 255, 255].repeat(4)
        );
        assert_eq!(
            preview.as_bytes(1).context("missing second frame")?,
            &[255, 0, 0, 255].repeat(4)
        );
        assert_eq!(preview.delay(1).numer_denom_ms(), (50, 1));
        let preview = super::wallpaper_thumbnail(&animated_webp(33)?, 8)?;
        assert_eq!(preview.frame_count(), 1);
        assert_eq!(
            preview.as_bytes(0).context("missing static fallback")?,
            &[0, 0, 255, 255].repeat(4)
        );

        let frames = image::Frames::new(Box::new((0..8).map(|_| {
            Ok(image::Frame::new(image::RgbaImage::from_pixel(
                1024,
                1024,
                image::Rgba([255, 0, 0, 255]),
            )))
        })));
        let preview =
            super::animated_thumbnail(frames, image::metadata::Orientation::NoTransforms, 1024)?;
        assert_eq!(preview.frame_count(), 1);
        assert_eq!(
            preview.size(0),
            gpui::size(gpui::DevicePixels(1024), gpui::DevicePixels(1024))
        );
        assert_eq!(
            preview
                .as_bytes(0)
                .context("missing pixel-budget fallback")?
                .get(..4),
            Some([0, 0, 255, 255].as_slice())
        );
        Ok(())
    }

    #[test]
    fn wallpaper_preview_loads_without_requiring_a_derivable_palette() -> anyhow::Result<()> {
        let path = std::env::temp_dir().join(format!(
            "dojo-wallpaper-preview-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        image::RgbImage::from_pixel(1, 1, image::Rgb([200, 30, 50])).save(&path)?;
        let preview = super::analyze_image(path.clone(), 512)?;
        assert_eq!(preview.path, path.canonicalize()?);
        let image = image::load_from_memory(preview.bytes.as_ref())?.to_rgb8();
        assert_eq!(image.get_pixel(0, 0).0, [200, 30, 50]);
        assert!(preview.candidates.is_empty());
        assert_eq!(super::derive_candidates(preview.bytes.as_ref())?.len(), 23);
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn wallpaper_preview_accepts_large_photographs_before_monitor_scaling() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = match std::env::var_os("DOJO_TEST_WALLPAPER") {
            Some(path) => PathBuf::from(path),
            None => {
                let path = directory.path().join("alley-sized.jpg");
                image::RgbImage::from_pixel(7952, 5304, image::Rgb([200, 30, 50])).save(&path)?;
                path
            }
        };
        let analysis = super::analyze_image(path, 8)?;
        assert_eq!(
            analysis.preview.size(0),
            gpui::size(gpui::DevicePixels(8), gpui::DevicePixels(5))
        );
        let palette = crate::config::ThemeColors::default()
            .semantic_palette(dojo::theme::Appearance::Dark)?;
        let store = super::ThemeStore::new(super::Paths {
            home: directory.path().join("home"),
            config: directory.path().join("config"),
            data: directory.path().join("data"),
            root: directory.path().join("themes"),
        });
        let report = dojo::theme::apply_wallpaper(
            &store,
            &analysis.path,
            analysis.bytes.as_ref().clone(),
            Some(palette),
        )?;
        assert_eq!(report.targets[0].target, "dojo");
        assert_eq!(report.targets[0].status, "published");
        assert_eq!(
            std::fs::read(
                store
                    .paths
                    .root
                    .join("generations")
                    .join(report.selection.generation)
                    .join("wallpaper")
            )?,
            analysis.bytes.as_ref().clone()
        );
        Ok(())
    }

    #[test]
    fn wallpaper_preview_keeps_original_bytes_and_bounds_decoded_pixels() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        for extension in ["png", "jpg", "webp"] {
            let path = directory.path().join(format!("wallpaper.{extension}"));
            image::RgbImage::from_pixel(40, 20, image::Rgb([200, 30, 50])).save(&path)?;
            let original = std::fs::read(&path)?;
            let analysis = super::analyze_image(path, 8)?;
            assert_eq!(analysis.bytes.as_ref(), &original);
            assert_eq!(
                analysis.preview.size(0),
                gpui::size(gpui::DevicePixels(8), gpui::DevicePixels(4))
            );
            assert_eq!(analysis.preview.frame_count(), 1);
            let pixels = analysis
                .preview
                .as_bytes(0)
                .context("missing thumbnail pixels")?;
            assert_eq!(pixels.len(), 8 * 4 * 4);
            let expected = [50u8, 30, 200, 255];
            for pixel in pixels.chunks_exact(4) {
                assert!(
                    pixel
                        .iter()
                        .zip(expected)
                        .all(|(actual, expected)| actual.abs_diff(expected) <= 3)
                );
            }
        }
        Ok(())
    }

    #[test]
    fn wallpaper_preview_respects_jpeg_orientation() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("oriented.jpg");
        image::RgbImage::from_pixel(20, 40, image::Rgb([200, 30, 50])).save(&path)?;
        let encoded = std::fs::read(&path)?;
        let exif = [
            b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1,
            0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut oriented = encoded[..2].to_vec();
        oriented.extend_from_slice(&[0xff, 0xe1, 0, 34]);
        oriented.extend_from_slice(&exif);
        oriented.extend_from_slice(&encoded[2..]);
        std::fs::write(&path, &oriented)?;
        let analysis = super::analyze_image(path, 8)?;
        assert_eq!(analysis.bytes.as_ref(), &oriented);
        assert_eq!(
            analysis.preview.size(0),
            gpui::size(gpui::DevicePixels(8), gpui::DevicePixels(4))
        );
        Ok(())
    }

    #[test]
    fn wallpaper_preview_rejects_excessive_dimensions_before_decode() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("too-wide.png");
        image::RgbImage::from_pixel(16_385, 1, image::Rgb([200, 30, 50])).save(&path)?;
        let error = super::analyze_image(path, 8)
            .err()
            .context("oversized image was accepted")?;
        assert!(error.to_string().contains("image dimension limits"));
        let path = directory.path().join("accepted.png");
        image::RgbImage::from_pixel(16_384, 1, image::Rgb([200, 30, 50])).save(&path)?;
        let analysis = super::analyze_image(path, 8)?;
        assert_eq!(
            analysis.preview.size(0),
            gpui::size(gpui::DevicePixels(8), gpui::DevicePixels(1))
        );
        Ok(())
    }

    #[test]
    fn picker_apply_includes_all_scoped_apps_not_just_dojo() {
        let targets = super::picker_targets();
        for target in dojo::theme::TARGETS {
            assert!(
                targets.iter().any(|selected| selected == target),
                "picker silently omits {target}"
            );
        }
    }

    #[test]
    fn application_results_expose_setup_and_reload_failures() {
        let report = dojo::theme::ApplyReport {
            selection: dojo::theme::Selection {
                generation: "fixture".into(),
                appearance: dojo::theme::Appearance::Dark,
                follow_system: false,
                targets: vec![],
            },
            targets: vec![
                dojo::theme::TargetResult {
                    target: "kitty".into(),
                    status: "reload_requested".into(),
                    detail: "signal sent".into(),
                },
                dojo::theme::TargetResult {
                    target: "zen".into(),
                    status: "needs_setup".into(),
                    detail: "extension missing".into(),
                },
                dojo::theme::TargetResult {
                    target: "opencode".into(),
                    status: "blocked".into(),
                    detail: "settings conflict".into(),
                },
            ],
        };
        let message = super::apply_results(&report);
        assert!(message.contains("kitty: live reload sent"));
        assert!(message.contains("zen: one-time setup required — extension missing"));
        assert!(message.contains("opencode: blocked — settings conflict"));
    }

    #[gpui::test]
    fn choosing_an_image_is_preview_only_and_invalidates_old_apply_source(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut picker = super::ThemePicker::default();
        cx.update(|cx| {
            cx.set_global(crate::config::ThemeColors::default());
            picker.toggle(None, cx).unwrap();
            picker
                .window
                .unwrap()
                .update(cx, |view, window, cx| {
                    assert!(view.show_wallpaper);
                    assert!(!view.show_presets);
                    let path = PathBuf::from("/missing/preview.png");
                    let revision = view.file_request_revision;
                    view.finish_image_request(revision, Ok(Some(path)), window, cx);
                    assert!(view.loading);
                    assert!(view.analysis.is_none());
                    assert!(!view.applying);
                    assert!(view.selected.is_none());
                    // Enter/Apply while loading must not publish anything.
                    view.apply(window, cx);
                    assert!(!view.applying);
                    view.close(window);
                })
                .unwrap();
        });
    }

    #[gpui::test]
    fn repeated_choose_clicks_do_not_start_another_request_and_close_cancels(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut picker = super::ThemePicker::default();
        cx.update(|cx| {
            cx.set_global(crate::config::ThemeColors::default());
            picker.toggle(None, cx).unwrap();
            picker
                .window
                .unwrap()
                .update(cx, |view, window, cx| {
                    view.loading = false;
                    let (request, mut cancelled) = super::portal::Request::pending_for_test();
                    let (_sender, response) = async_channel::bounded(1);
                    view.choose_image_with(window, cx, || Ok((request, response)));
                    let revision = view.file_request_revision;
                    assert!(view.file_request.is_some());
                    view.choose_image_with(window, cx, || panic!("duplicate portal request"));
                    assert_eq!(view.file_request_revision, revision);
                    view.apply(window, cx);
                    assert!(!view.applying);
                    view.close(window);
                    assert!(view.file_request.is_none());
                    assert_eq!(cancelled.try_recv(), Ok(()));
                    view.choose_image_with(window, cx, || panic!("request after close"));
                })
                .unwrap();
        });
    }

    #[gpui::test]
    fn portal_cancel_errors_and_late_responses_do_not_change_selection(
        cx: &mut gpui::TestAppContext,
    ) {
        let mut picker = super::ThemePicker::default();
        cx.update(|cx| {
            cx.set_global(crate::config::ThemeColors::default());
            picker.toggle(None, cx).unwrap();
            picker
                .window
                .unwrap()
                .update(cx, |view, window, cx| {
                    view.loading = false;
                    view.selected = Some(3);
                    view.message = "Existing preview".into();
                    let revision = view.file_request_revision;
                    let analysis_revision = view.analysis_revision;
                    view.finish_image_request(revision, Ok(None), window, cx);
                    assert_eq!(view.selected, Some(3));
                    assert_eq!(view.message, "Existing preview");
                    assert_eq!(view.analysis_revision, analysis_revision);
                    view.finish_image_request(
                        revision,
                        Err(anyhow::anyhow!("portal unavailable")),
                        window,
                        cx,
                    );
                    assert!(
                        view.file_request_error
                            .as_ref()
                            .unwrap()
                            .contains("portal unavailable")
                    );
                    assert_eq!(view.selected, Some(3));
                    assert_eq!(view.message, "Existing preview");
                    view.close(window);
                    view.finish_image_request(revision, Ok(Some("/late.png".into())), window, cx);
                    assert_eq!(view.selected, Some(3));
                    assert!(!view.loading);
                    assert!(!view.applying);
                })
                .unwrap();
        });
    }

    #[test]
    fn config_palette_preserves_all_existing_shell_roles() -> anyhow::Result<()> {
        let colors = crate::config::ThemeColors::default();
        let palette = colors.semantic_palette(dojo::theme::Appearance::Dark)?;
        for (role, color) in [
            ("background", colors.background),
            ("foreground", colors.foreground),
            ("primary", colors.primary),
            ("background_highlight", colors.background_highlight),
            ("border", colors.border),
            ("foreground_muted", colors.foreground_muted),
        ] {
            assert_eq!(palette.color(role), color.hex());
        }
        Ok(())
    }

    #[test]
    fn theme_content_can_shrink_below_the_full_preset_list_height() {
        let mut row = theme_content_row();
        assert_eq!(row.style().min_size.height, Some(px(0.).into()));
    }
    #[test]
    fn a_layout_change_is_saved_by_apply_and_publishes_the_image_only_when_one_was_chosen() {
        // Nothing changed: Apply republishes the current wallpaper, as it always has.
        assert_eq!(wallpaper_apply_plan(Fill, Fill, false), (None, true));
        // Layout alone: save it, leave the wallpaper and theme alone.
        assert_eq!(wallpaper_apply_plan(Span, Fill, false), (Some(Span), false));
        // A new image with the same layout, and with a new layout: publish the image.
        assert_eq!(wallpaper_apply_plan(Fill, Fill, true), (None, true));
        assert_eq!(wallpaper_apply_plan(Span, Fill, true), (Some(Span), true));
    }

    #[test]
    fn applying_requires_an_explicit_ready_selection() {
        assert!(!confirmable(None, 23, false, false));
        assert!(!confirmable(Some(23), 23, false, false));
        assert!(!confirmable(Some(1), 23, true, false));
        assert!(!confirmable(Some(1), 23, false, true));
        assert!(confirmable(Some(1), 23, false, false));
    }

    #[test]
    fn random_and_pasted_colours_do_not_depend_on_the_selected_theme() -> anyhow::Result<()> {
        // The theme source reads the user's own store, so only the other two are checked here.
        assert_eq!(
            PaletteSource::Random.palette(9, "#000000 #ffffff"),
            Palette::from_seed(9)
        );
        let pasted = Palette::from_colors(&parse_colors("#101018 #4466cc #99bbff"))?;
        assert_eq!(
            PaletteSource::Pasted(pasted.clone()).palette(9, ""),
            pasted
        );
        assert_ne!(PaletteSource::Theme, PaletteSource::Random);
        Ok(())
    }

    #[test]
    fn a_generated_wallpaper_previews_as_its_recipe_with_the_picture_to_derive_from() -> anyhow::Result<()> {
        let recipe = Recipe::new(Style::Aurora, 4);
        let analysis = analyze_recipe(recipe.clone(), 256)?;
        assert_eq!(analysis.recipe.as_ref(), Some(&recipe));
        assert!(analysis.path.is_absolute());
        // The bytes are a real PNG of the canvas, which the derive tab can read like any image.
        let picture = image::load_from_memory(&analysis.bytes)?;
        assert_eq!(picture.width().max(picture.height()), 1920);
        assert!(!derive_candidates(&analysis.bytes)?.is_empty());
        Ok(())
    }
}
