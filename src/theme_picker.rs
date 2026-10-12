use std::io::Read as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use dojo::extraction::Mode;
use dojo::theme::{Appearance, Paths, Preset, Selection, SemanticPalette, Theme, ThemeStore};
use gpui::{
    App, AppContext, Context, FocusHandle, KeyDownEvent, MouseButton, Render, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, img,
    layer_shell, point, prelude::*, px,
};

use crate::config::ThemeColors;

gpui::actions!(theme_picker, [ToggleThemePicker]);

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
}

fn analyze(preview_edge: u32) -> anyhow::Result<Analysis> {
    let store = ThemeStore::new(Paths::user_default()?);
    let path = if let Some(selection) = store.selection()? {
        let theme = store.theme(&selection.generation)?;
        let snapshot = store
            .paths
            .root
            .join("generations")
            .join(selection.generation)
            .join("wallpaper");
        if snapshot.is_file() {
            snapshot
        } else if !theme.wallpaper.is_empty() {
            PathBuf::from(theme.wallpaper)
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("astro.jpg")
        }
    } else {
        // The repo's current btrbg wallpaper is the initial source until wallpaper
        // selection is provided by the appearance UI.
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("astro.jpg")
    };
    analyze_image(path, preview_edge)
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
        width <= 16_384 && height <= 16_384 && u64::from(width) * u64::from(height) <= 40_000_000,
        "wallpaper exceeds image dimension limits"
    );
    let preview = wallpaper_thumbnail(&bytes, preview_edge)?;
    Ok(Analysis {
        path,
        bytes: Arc::new(bytes),
        preview,
        candidates: Vec::new(),
        appearance: dojo::theme::system_appearance().unwrap_or(Appearance::Dark),
    })
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
    dojo::extraction::extract_all_pairs(bytes)?
        .into_iter()
        .map(|(mode, dark, light)| {
            let dark = SemanticPalette::derive(&dark, Appearance::Dark);
            let light = SemanticPalette::derive(&light, Appearance::Light);
            dark.validate()?;
            light.validate()?;
            Ok(Candidate { mode, dark, light })
        })
        .collect()
}

fn wallpaper_entries(directory: &std::path::Path) -> anyhow::Result<Vec<(PathBuf, bool)>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let directory = path.is_dir();
        if directory
            || (path.is_file()
                && path
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "jpg" | "jpeg" | "png" | "webp"
                        )
                    }))
        {
            entries.push((path, directory));
        }
    }
    entries.sort_by_key(|(path, directory)| {
        (
            !*directory,
            path.file_name().map(|s| s.to_string_lossy().to_lowercase()),
        )
    });
    anyhow::ensure!(
        entries.len() <= 1024,
        "folder has more than 1024 images/subfolders; choose a smaller folder"
    );
    Ok(entries)
}

#[derive(Default)]
pub struct ThemePicker {
    window: Option<WindowHandle<PickerView>>,
}

impl ThemePicker {
    pub fn toggle(&mut self, cx: &mut App) -> anyhow::Result<()> {
        if let Some(window) = self.window.take() {
            if window
                .update(cx, |view, window, _| view.close(window))
                .is_ok()
            {
                return Ok(());
            }
        }
        let display = cx
            .primary_display()
            .or_else(|| cx.displays().into_iter().next())
            .context("no display for theme picker")?;
        let presets = dojo::theme::presets()?;
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
        let directory = if home.join("Pictures").is_dir() {
            home.join("Pictures")
        } else {
            home
        };
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
                    keyboard_interactivity: layer_shell::KeyboardInteractivity::Exclusive,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    cx.on_release_in(window, |view: &mut PickerView, window, _| {
                        view.release_preview(window);
                    }).detach();
                    let preview_edge = preview_edge(window);
                    cx.spawn_in(window, async move |this, cx| {
                        let result = cx.background_executor().spawn(async move { analyze(preview_edge) }).await;
                        if let Err(error) = this.update_in(cx, |view: &mut PickerView, window, cx| {
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
                                        view.message = "Choose an image, or keep the current wallpaper. Derivation is optional.".into();
                                    } else if !view.show_presets {
                                        view.derive(window, cx);
                                    }
                                }
                                Err(error) => view.message = format!("{error:#}"),
                            }
                            cx.notify();
                        }) {
                            eprintln!("Theme picker closed during analysis: {error:#}");
                        }
                    })
                    .detach();
                    let mut view = PickerView {
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
                        directory,
                        entries: Vec::new(),
                        browsing: false,
                        analysis_revision: 0,
                    };
                    view.browse(view.directory.clone(), window, cx);
                    view
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
    directory: PathBuf,
    entries: Vec<(PathBuf, bool)>,
    browsing: bool,
    analysis_revision: u64,
}

fn button_label(background: gpui::Rgba) -> gpui::Rgba {
    let linear = |channel: f32| {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(background.r)
        + 0.7152 * linear(background.g)
        + 0.0722 * linear(background.b);
    if (luminance + 0.05) / 0.05 >= 1.05 / (luminance + 0.05) {
        gpui::rgb(0x000000)
    } else {
        gpui::rgb(0xffffff)
    }
}

fn confirmable(selected: Option<usize>, count: usize, loading: bool, applying: bool) -> bool {
    !loading && !applying && selected.is_some_and(|index| index < count)
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
        self.release_preview(window);
        window.remove_window();
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

    fn browse(&mut self, directory: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.applying || self.browsing {
            return;
        }
        self.browsing = true;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let path = directory.clone();
            let result = cx
                .background_executor()
                .spawn(async move { wallpaper_entries(&path) })
                .await;
            if let Err(error) = this.update_in(cx, |view, _, cx| {
                view.browsing = false;
                match result {
                    Ok(entries) => {
                        view.directory = directory;
                        view.entries = entries;
                    }
                    Err(error) => view.message = format!("Could not browse folder: {error:#}"),
                }
                cx.notify();
            }) {
                eprintln!("Wallpaper browser closed: {error:#}");
            }
        })
        .detach();
    }

    fn choose_wallpaper(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.applying {
            return;
        }
        self.analysis_revision += 1;
        let revision = self.analysis_revision;
        self.loading = true;
        self.selected = None;
        self.message = "Loading wallpaper preview…".into();
        cx.notify();
        let preview_edge = preview_edge(window);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move {analyze_image(path, preview_edge)}).await;
            if let Err(error) = this.update_in(cx, |view, window, cx| {
                if view.analysis_revision != revision {return;}
                view.loading = false;
                match result {
                    Ok(analysis) => {
                        view.replace_analysis(analysis, window);
                        view.message="Preview only. Apply wallpaper keeps your theme; Derive attempts a new palette.".into();
                        if !view.show_wallpaper && !view.show_presets {
                            view.derive(window, cx);
                        }
                    },
                    Err(error) => view.message=format!("Could not load wallpaper: {error:#}"),
                }
                cx.notify();
            }) {eprintln!("Wallpaper picker closed: {error:#}");}
        }).detach();
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_wallpaper {
            if self.loading || self.applying {
                return;
            }
            let Some(analysis) = self.analysis.clone() else {
                return;
            };
            let colors = *cx.global::<ThemeColors>();
            self.applying = true;
            self.message = "Applying wallpaper without changing your theme…".into();
            cx.notify();
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let store = ThemeStore::new(Paths::user_default()?);
                        let fallback = if store.selection()?.is_none() {
                            Some(colors.semantic_palette(dojo::theme::system_appearance()?)?)
                        } else {
                            None
                        };
                        dojo::theme::apply_wallpaper(
                            &store,
                            &analysis.path,
                            analysis.bytes.as_ref().clone(),
                            fallback,
                        )
                    })
                    .await;
                if let Err(error) = this.update_in(cx, |view, _, cx| {
                    view.applying = false;
                    view.message = match result {
                        Ok(report) => format!(
                            "Wallpaper selected. Theme and darkman mode unchanged.\n{}",
                            apply_results(&report)
                        ),
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
        self.applying = true;
        self.message = "Applying selected mode across supported apps…".into();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let theme = Theme::from_snapshot(
                        &analysis.path,
                        analysis.bytes.as_ref().clone(),
                        mode,
                    )?;
                    let store = ThemeStore::new(Paths::user_default()?);
                    let appearance = dojo::theme::system_appearance()?;
                    let generation = store.generate(&theme)?;
                    dojo::theme::apply(
                        &store,
                        Selection {
                            generation,
                            appearance,
                            follow_system: true,
                            targets: picker_targets(),
                        },
                    )
                })
                .await;
            if let Err(error) = this.update_in(cx, |view, _, cx| {
                view.applying = false;
                view.message = match result {
                    Ok(report) => format!(
                        "Selected {}. Appearance follows darkman.\n{}",
                        mode.name(),
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
        let colors = *cx.global::<ThemeColors>();
        let width = (f32::from(window.viewport_size().width) - 48.)
            .min(940.)
            .max(320.);
        let height = (f32::from(window.viewport_size().height) - 96.)
            .min(720.)
            .max(300.);
        let modes_width = (width * 0.43).min(400.);
        let mut mode_buttons = Vec::new();
        if self.show_wallpaper {
            if let Some(parent) = self.directory.parent() {
                let parent = parent.to_path_buf();
                mode_buttons.push(
                    div()
                        .id("wallpaper-parent")
                        .w_full()
                        .h(px(32.))
                        .child("↑ Parent folder")
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, window, cx| {
                                view.browse(parent.clone(), window, cx)
                            }),
                        ),
                );
            }
            mode_buttons.push(
                div()
                    .id("wallpaper-directory")
                    .w_full()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_size(px(11.))
                    .text_color(colors.foreground_muted.rgb())
                    .child(self.directory.display().to_string()),
            );
            for (index, (path, directory)) in self.entries.iter().enumerate() {
                let path = path.clone();
                let directory = *directory;
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                mode_buttons.push(
                    div()
                        .id(("wallpaper-file", index))
                        .w_full()
                        .overflow_hidden()
                        .text_ellipsis()
                        .h(px(34.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .child(format!("{} {name}", if directory { "▸" } else { "▧" }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |view, _, window, cx| {
                                if directory {
                                    view.browse(path.clone(), window, cx);
                                } else {
                                    view.choose_wallpaper(path.clone(), window, cx);
                                }
                            }),
                        ),
                );
            }
            if self.browsing {
                mode_buttons.push(
                    div()
                        .id("wallpaper-folder-loading")
                        .w_full()
                        .child("Loading folder…"),
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
                mode_buttons.push(div().id(("static-preset",index)).w_full().h(px(34.)).px(px(8.)).flex().items_center().justify_between().rounded(px(6.)).cursor_pointer().bg(if self.selected == Some(index) {colors.background_highlight.rgb()} else {colors.background.rgb()}).text_color(if self.selected == Some(index) {button_label(colors.background_highlight.rgb())} else {colors.foreground.rgb()}).child(preset.name.clone()).child(div().text_size(px(10.)).child(appearance.name())).on_mouse_down(MouseButton::Left,cx.listener(move |view,_,_,cx| {
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
                            button_label(colors.background_highlight.rgb())
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
                    .h(px(if self.show_wallpaper { 300. } else { 160. }))
                    .object_fit(gpui::ObjectFit::Cover)
                    .rounded(px(10.)),
            );
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
            preview = preview.child(if self.show_wallpaper {"Choose an image from a folder. Apply wallpaper keeps your existing theme; Derive is optional."} else {"Choose a theme to preview its colors."});
        }
        let mut appearance_buttons = div().flex().gap(px(8.));
        for appearance in [Appearance::Dark, Appearance::Light] {
            appearance_buttons = appearance_buttons.child(
                div()
                    .id(format!("theme-preview-{}", appearance.name()))
                    .px(px(12.))
                    .py(px(7.))
                    .rounded(px(crate::design::PILL_RADIUS))
                    .bg(colors.background_highlight.rgb())
                    .text_color(button_label(colors.background_highlight.rgb()))
                    .child(format!("{} preview", appearance.name()))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.appearance = appearance;
                            cx.notify();
                        }),
                    ),
            );
        }
        if !self.show_presets && !self.show_wallpaper {
            preview = preview.child(appearance_buttons);
        }
        let ready = if self.show_wallpaper {
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
            (false, true, "Presets · 36 themes"),
            (false, false, "Derive"),
        ] {
            source_buttons = source_buttons.child(
                div()
                    .id(if show_wallpaper {"wallpaper-browser-source"} else if show_presets {
                        "preset-source"
                    } else {
                        "wallpaper-source"
                    })
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(crate::design::PILL_RADIUS))
                    .cursor_pointer()
                    .bg(if self.show_presets == show_presets && self.show_wallpaper == show_wallpaper {
                        colors.background_highlight.rgb()
                    } else {
                        colors.background.rgb()
                    })
                    .text_color(if self.show_presets == show_presets && self.show_wallpaper == show_wallpaper {
                        button_label(colors.background_highlight.rgb())
                    } else {
                        colors.foreground.rgb()
                    })
                    .child(label)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, window, cx| {
                            if view.applying || (view.show_presets == show_presets && view.show_wallpaper == show_wallpaper) {
                                return;
                            }
                            view.show_presets = show_presets;
                            view.show_wallpaper = show_wallpaper;
                            view.selected = None;
                            view.message = "Choose a theme. No desktop changes until Apply.".into();
                            if show_wallpaper {
                                view.message = "Choose an image, or keep the current wallpaper. Derivation is optional.".into();
                                view.browse(view.directory.clone(),window,cx);
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
            .w(px(width))
            .h(px(height))
            .p(px(22.))
            .rounded(px(18.))
            .bg(colors.background.rgb())
            .border_1()
            .border_color(colors.border.rgb().opacity(crate::design::BORDER_OPACITY))
            .flex()
            .flex_col()
            .gap(px(14.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(20.)).child("Wallpaper & themes"))
            .child(source_buttons)
            .child(
                theme_content_row()
                    .child(
                        div()
                            .id("theme-mode-list")
                            .w(px(modes_width))
                            .min_w(px(0.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_wrap()
                            .content_start()
                            .gap(px(8.))
                            .children(mode_buttons),
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
                    .child(div().text_size(px(12.)).child(
                        "Preview only until Apply. Per-app setup and reload results appear above.",
                    ))
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
                            .text_color(button_label(if ready {
                                colors.primary.rgb()
                            } else {
                                colors.background_highlight.rgb()
                            }))
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
                (window.viewport_size().height - px(height)) / 2.,
            ),
            size: gpui::Size::new(px(width), px(height)),
        });
        let mut scrim = colors.background.rgb();
        scrim.a = 0.65;
        div()
            .size_full()
            .bg(scrim)
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .text_size(px(12.))
            .text_color(colors.foreground.rgb())
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" if !view.applying => view.close(window),
                    "enter" if !event.is_held => view.apply(window, cx),
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
            .child(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::{button_label, confirmable, theme_content_row};
    use anyhow::Context as _;
    use gpui::{Styled, px};

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
        assert!(super::derive_candidates(preview.bytes.as_ref()).is_err());
        std::fs::remove_file(path)?;
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

    #[test]
    fn wallpaper_browser_lists_directories_and_supported_images_only() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "dojo-wallpaper-browser-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        std::fs::create_dir(&root)?;
        std::fs::create_dir(root.join("folder"))?;
        std::fs::write(root.join("image.JPG"), b"fixture")?;
        std::fs::write(root.join("notes.txt"), b"fixture")?;
        let entries = super::wallpaper_entries(&root)?;
        assert_eq!(
            entries,
            vec![(root.join("folder"), true), (root.join("image.JPG"), false)]
        );
        std::fs::remove_dir_all(root)?;
        Ok(())
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
    fn button_label_contrasts_with_light_and_dark_backgrounds() {
        assert_eq!(button_label(gpui::rgb(0xcba6f7)), gpui::rgb(0x000000));
        assert_eq!(button_label(gpui::rgb(0x45475a)), gpui::rgb(0xffffff));
        assert_eq!(button_label(gpui::rgb(0xffffff)), gpui::rgb(0x000000));
        assert_eq!(button_label(gpui::rgb(0x000000)), gpui::rgb(0xffffff));
    }
    #[test]
    fn applying_requires_an_explicit_ready_selection() {
        assert!(!confirmable(None, 23, false, false));
        assert!(!confirmable(Some(23), 23, false, false));
        assert!(!confirmable(Some(1), 23, true, false));
        assert!(!confirmable(Some(1), 23, false, true));
        assert!(confirmable(Some(1), 23, false, false));
    }
}
