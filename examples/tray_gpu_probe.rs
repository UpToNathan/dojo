use gpui::{
    App, Bounds, Context, ImageSource, IntoElement, Render, RenderImage, Window, WindowBounds,
    WindowOptions, div, img, prelude::*, px, size,
};
use std::{collections::HashSet, path::Path, sync::Arc};

#[allow(dead_code)]
#[path = "../src/tray.rs"]
mod tray;

struct Probe {
    items: Vec<tray::TrayItem>,
    cached: Option<Vec<Arc<RenderImage>>>,
    cache: tray::TrayImageCache,
    mode: String,
    frame: usize,
    baseline: u64,
}

fn vram_kib() -> u64 {
    let mut seen = HashSet::new();
    std::fs::read_dir("/proc/self/fdinfo")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| {
            let fields: std::collections::HashMap<_, _> = text
                .lines()
                .filter_map(|line| line.split_once(':'))
                .map(|(key, value)| (key, value.trim()))
                .collect();
            if fields.get("drm-driver") != Some(&"amdgpu") {
                return None;
            }
            let id = fields.get("drm-client-id")?.to_string();
            if !seen.insert(id) {
                return None;
            }
            fields
                .get("drm-memory-vram")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
        .sum()
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame += 1;
        if self.frame == 10 {
            self.baseline = vram_kib();
        }
        if self.frame == 310 {
            let final_kib = vram_kib();
            println!(
                "{}",
                serde_json::json!({
                    "mode": self.mode,
                    "measured_frames": 300,
                    "baseline_vram_KiB": self.baseline,
                    "final_vram_KiB": final_kib,
                    "growth_KiB": final_kib.saturating_sub(self.baseline),
                })
            );
            cx.quit();
        } else {
            window.request_animation_frame();
        }
        let images = if self.mode == "cache" || self.mode == "churn" {
            let previous = self.cache.image("250");
            let mut visible = self.items.clone();
            if self.mode == "churn" {
                match self.frame % 3 {
                    0 => {
                        for item in &mut visible {
                            for pixel in
                                Arc::make_mut(&mut item.icon_pixmaps[0].argb).chunks_exact_mut(4)
                            {
                                pixel[1] = 200;
                            }
                        }
                    }
                    1 => visible[0].status = "Passive".into(),
                    _ => {
                        visible.pop();
                    }
                }
            }
            self.cache.update(&visible, window);
            if self.mode == "cache" {
                if let Some(previous) = previous {
                    assert_eq!(self.cache.image("250").unwrap().id, previous.id);
                }
            } else {
                match self.frame % 3 {
                    0 => assert_eq!(
                        &self.cache.image("250").unwrap().as_bytes(0).unwrap()[..4],
                        &[200, 100, 150, 255]
                    ),
                    1 => assert!(self.cache.image("250").is_none()),
                    _ => assert!(self.cache.image("256").is_none()),
                }
            }
            visible
                .iter()
                .filter_map(|item| self.cache.image(&item.key))
                .collect()
        } else {
            self.cached.clone().unwrap_or_else(|| {
                self.items
                    .iter()
                    .map(|item| item.render_image().unwrap())
                    .collect()
            })
        };
        div().flex().children(
            images
                .into_iter()
                .map(|image| img(ImageSource::Render(image)).size(px(32.))),
        )
    }
}

fn main() {
    assert!(
        Path::new("/sys/class/drm/card1/device/mem_info_vram_used").exists(),
        "requires this AMD desktop"
    );
    let mode = std::env::args()
        .nth(1)
        .expect("usage: tray_gpu_probe recreate|reuse|cache|churn");
    let reuse = match mode.as_str() {
        "reuse" => true,
        "recreate" | "cache" | "churn" => false,
        _ => panic!("usage: tray_gpu_probe recreate|reuse|cache|churn"),
    };
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(96.), px(64.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("dojo-tray-gpu-probe".into()),
                focus: false,
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("Tray GPU probe");
                cx.new(|_| {
                    let items: Vec<_> = [250, 256]
                        .into_iter()
                        .map(|dimension| tray::TrayItem {
                            key: dimension.to_string(),
                            service: String::new(),
                            object_path: String::new(),
                            title: String::new(),
                            status: "Active".into(),
                            icon_name: String::new(),
                            icon_pixmaps: vec![tray::TrayPixmap {
                                width: dimension,
                                height: dimension,
                                argb: [255, 50, 100, 150]
                                    .repeat(dimension as usize * dimension as usize)
                                    .into(),
                            }],
                            item_is_menu: false,
                            menu_path: None,
                        })
                        .collect();
                    let cached = reuse.then(|| {
                        items
                            .iter()
                            .map(|item| item.render_image().unwrap())
                            .collect()
                    });
                    Probe {
                        items,
                        cached,
                        cache: tray::TrayImageCache::default(),
                        mode,
                        frame: 0,
                        baseline: 0,
                    }
                })
            },
        )
        .expect("open probe window");
    });
}
