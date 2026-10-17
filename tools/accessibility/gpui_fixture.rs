use gpui::{
    App, Bounds, Context, Role, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use gpui_platform::application;

struct FixtureWindow {
    name: &'static str,
    button_label: &'static str,
    invoked: bool,
}

impl Render for FixtureWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let button_accessible_name = if self.invoked {
            format!("{} invoked", self.button_label)
        } else {
            self.button_label.to_owned()
        };
        div()
            .id("fixture-root")
            .role(Role::Application)
            .aria_label(self.name)
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .child(
                div()
                    .id("fixture-heading")
                    .role(Role::Heading)
                    .aria_label(self.name)
                    .child(self.name),
            )
            .child(
                div()
                    .id("fixture-button")
                    .role(Role::Button)
                    .focusable()
                    .tab_stop(true)
                    .aria_label(button_accessible_name.clone())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.invoked = true;
                        cx.notify();
                    }))
                    .child(button_accessible_name),
            )
    }
}

fn main() {
    application().run(|cx: &mut App| {
        for (name, button_label, x) in [
            ("Dojo Fixture Alpha", "Alpha action", 80.0),
            ("Dojo Fixture Beta", "Beta action", 660.0),
        ] {
            let bounds = Bounds::centered(None, size(px(480.0), px(320.0)), cx);
            let mut bounds = bounds;
            bounds.origin.x = px(x);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: None,
                    ..Default::default()
                },
                |window, cx| {
                    window.set_window_title(name);
                    cx.new(|_| FixtureWindow {
                        name,
                        button_label,
                        invoked: false,
                    })
                },
            )
            .expect("open fixture window");
        }
        cx.activate(true);
    });
}
