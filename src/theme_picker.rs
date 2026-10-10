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
    candidates: Vec<Candidate>,
    appearance: Appearance,
}

fn analyze() -> anyhow::Result<Analysis> {
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
    analyze_image(path)
}

fn analyze_image(path: PathBuf) -> anyhow::Result<Analysis> {
    let path = path.canonicalize()?;
    anyhow::ensure!(path.is_file(), "wallpaper must be a regular image file");
    let mut file = std::fs::File::open(&path)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(50 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 50 * 1024 * 1024, "wallpaper exceeds 50 MiB");
    let candidates = dojo::extraction::extract_all_pairs(&bytes)?
        .into_iter()
        .map(|(mode, dark, light)| Candidate {
            mode,
            dark: SemanticPalette::derive(&dark, Appearance::Dark),
            light: SemanticPalette::derive(&light, Appearance::Light),
        })
        .collect();
    Ok(Analysis {
        path,
        bytes: Arc::new(bytes),
        candidates,
        appearance: dojo::theme::system_appearance().unwrap_or(Appearance::Dark),
    })
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
                .update(cx, |_, window, _| window.remove_window())
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
                    cx.spawn_in(window, async move |this, cx| {
                        let result = cx.background_executor().spawn(async { analyze() }).await;
                        if let Err(error) = this.update_in(cx, |view: &mut PickerView, _, cx| {
                            if view.analysis_revision != 0 {
                                return;
                            }
                            view.loading = false;
                            match result {
                                Ok(analysis) => {
                                    if !view.show_presets {
                                        view.appearance = analysis.appearance;
                                    }
                                    view.analysis = Some(Arc::new(analysis));
                                    view.message =
                                        "Choose a theme. No desktop changes until Apply.".into();
                                }
                                Err(error) => view.message = format!("{error:#}"),
                            }
                            cx.notify();
                        }) {
                            eprintln!("Theme picker closed during analysis: {error:#}");
                        }
                    })
                    .detach();
                    PickerView {
                        focus,
                        analysis: None,
                        selected: None,
                        presets,
                        show_presets: true,
                        appearance: Appearance::Dark,
                        loading: true,
                        applying: false,
                        message: "Choose a preset, or derive colors from wallpaper.".into(),
                        panel: None,
                        show_wallpaper: false,
                        directory,
                        entries: Vec::new(),
                        browsing: false,
                        analysis_revision: 0,
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
        self.message = "Loading image and preparing derivation previews…".into();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move {analyze_image(path)}).await;
            if let Err(error) = this.update_in(cx, |view, _, cx| {
                if view.analysis_revision != revision {return;}
                view.loading = false;
                match result {
                    Ok(analysis) => {view.analysis=Some(Arc::new(analysis));view.message="Preview only. Apply wallpaper keeps your theme; Derive creates a new palette.".into();},
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
                img(analysis.path.clone())
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
                    .rounded(px(6.))
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
                    Mode::ALL.len()
                },
                !self.show_presets && self.loading,
                self.applying,
            )
        };
        let mut source_buttons = div().flex().gap(px(8.));
        for (show_wallpaper, show_presets, label) in [
            (true, false, "Wallpaper"),
            (false, false, "Derive · 23 modes"),
            (false, true, "Presets · 36 themes"),
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
                    .rounded(px(6.))
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
            .border_color(colors.border.rgb())
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
                            .rounded(px(8.))
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
            .text_size(px(12.))
            .text_color(colors.foreground.rgb())
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" if !view.applying => window.remove_window(),
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
                        window.remove_window();
                    }
                }),
            )
            .child(panel)
    }
}

#[cfg(test)]
mod tests {
    use super::{button_label, confirmable, theme_content_row};
    use gpui::{Styled, px};

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
