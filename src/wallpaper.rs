use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context as _;
use gpui::{
    App, AppContext, Context, DisplayId, Render, Subscription, Window, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, div, img, layer_shell, prelude::*, px,
};

struct WallpaperView {
    path: PathBuf,
}

impl Render for WallpaperView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        window.set_input_region(Some(&[]));
        div().size_full().child(
            img(self.path.clone())
                .size_full()
                .object_fit(gpui::ObjectFit::Cover),
        )
    }
}

struct WallpaperController {
    path: Option<PathBuf>,
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
            .filter(|id| !ids.contains(id) || self.path.is_none())
            .copied()
            .collect();
        for id in removed {
            if let Some(window) = self.windows.remove(&id) {
                window.update(cx, |_, window, _| window.remove_window())?;
            }
        }
        let Some(path) = self.path.clone() else {
            return Ok(());
        };
        for display in displays {
            if let Some(window) = self.windows.get(&display.id()) {
                window.update(cx, |view, _, cx| {
                    if view.path != path {
                        view.path = path.clone();
                        cx.notify();
                    }
                })?;
            } else {
                let image = path.clone();
                let window = cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(display.bounds())),
                        display_id: Some(display.id()),
                        titlebar: None,
                        app_id: Some("dojo".into()),
                        window_background: gpui::WindowBackgroundAppearance::Transparent,
                        kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                            namespace: format!("dojo-wallpaper-{}", u64::from(display.id())),
                            layer: layer_shell::Layer::Background,
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
                    |_, cx| cx.new(|_| WallpaperView { path: image }),
                )?;
                self.windows.insert(display.id(), window);
            }
        }
        Ok(())
    }
}

pub fn start(cx: &mut App) -> anyhow::Result<()> {
    let path = dojo::theme::Paths::user_default()?
        .root
        .join("current/wallpaper.txt");
    let controller = cx.new(|_| WallpaperController {
        path: None,
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
        loop {
            let file = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    match std::fs::read_to_string(file) {
                        Ok(path) => {
                            let path = PathBuf::from(path.trim());
                            anyhow::ensure!(
                                path.is_absolute() && path.is_file(),
                                "selected wallpaper must be an existing absolute file"
                            );
                            Ok(Some(path))
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                        Err(error) => Err(error).context("reading selected wallpaper"),
                    }
                })
                .await;
            match result {
                Ok(path) => {
                    if let Err(error) = task_controller.update(cx, |controller, cx| {
                        controller.path = path;
                        controller.reconcile(cx)
                    }) {
                        eprintln!("Dojo wallpaper update failed: {error:#}");
                    }
                }
                Err(error) => eprintln!("Dojo retained its previous wallpaper: {error:#}"),
            }
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
        }
    })
    .detach();
    Ok(())
}
