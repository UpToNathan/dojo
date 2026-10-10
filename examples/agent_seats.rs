use gpui::{
    App, Bounds, Context, FocusHandle, IntoElement, KeyDownEvent, KeyUpEvent, MouseButton,
    MouseDownEvent, MouseUpEvent, Render, ScrollDelta, ScrollWheelEvent, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use std::io::Write;

struct InputWindow {
    number: usize,
    focus: FocusHandle,
    text: String,
}

fn record(event: serde_json::Value) {
    println!("{event}");
    std::io::stdout().flush().unwrap();
}

impl Render for InputWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(if self.number == 1 { 0x285a84 } else { 0x8a5930 }))
            .text_color(rgb(0xffffff))
            .p_4()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|view, event: &KeyDownEvent, _, cx| {
                record(serde_json::json!({"event":"key_down", "window":view.number,
                    "key":event.keystroke.key, "key_char":event.keystroke.key_char,
                    "modifiers":event.keystroke.modifiers, "held":event.is_held}));
                view.text.push_str(&event.keystroke.key);
                view.text.push(' ');
                cx.notify();
            }))
            .on_key_up(cx.listener(|view, event: &KeyUpEvent, _, _| {
                record(serde_json::json!({"event":"key_up", "window":view.number,
                    "key":event.keystroke.key}));
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, window, cx| {
                    window.focus(&view.focus, cx);
                    record(
                        serde_json::json!({"event":"mouse_down", "window":view.number,
                    "x":f32::from(event.position.x), "y":f32::from(event.position.y)}),
                    );
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _, _| {
                    record(serde_json::json!({"event":"mouse_up", "window":view.number}));
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _: &MouseUpEvent, _, _| {
                    record(serde_json::json!({"event":"mouse_up", "window":view.number, "outside":true}));
                }),
            )
            .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _, _| {
                let (kind, x, y) = match event.delta {
                    ScrollDelta::Pixels(point) => {
                        ("pixels", f32::from(point.x), f32::from(point.y))
                    }
                    ScrollDelta::Lines(point) => ("lines", point.x, point.y),
                };
                record(serde_json::json!({"event":"scroll", "window":view.number,
                    "kind":kind, "x":x, "y":y, "modifiers":event.modifiers}));
            }))
            .child(format!("Window {}\n{}", self.number, self.text))
    }
}

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        for number in 1..=2 {
            let bounds = Bounds::centered(None, size(px(320.), px(240.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    app_id: Some("dojo-agent-input-fixture".into()),
                    ..Default::default()
                },
                move |window, cx| {
                    window.set_window_title(&format!("Agent input fixture {number}"));
                    cx.new(|cx| {
                        let focus = cx.focus_handle();
                        window.focus(&focus, cx);
                        InputWindow {
                            number,
                            focus,
                            text: String::new(),
                        }
                    })
                },
            )
            .expect("open input fixture window");
        }
    });
}
