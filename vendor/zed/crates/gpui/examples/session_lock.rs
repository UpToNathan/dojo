//! Locks the session with `ext-session-lock-v1`, one window per display, until `test` is typed
//! and submitted with Enter. Run it in a nested compositor: a wrong password keeps you locked.
#![cfg_attr(target_family = "wasm", no_main)]

fn run_example() {
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    example::main();

    #[cfg(not(all(target_os = "linux", feature = "wayland")))]
    panic!("This example requires the `wayland` feature and a linux system.");
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
mod example {
    use std::{
        ops::Range,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use gpui::{
        App, Bounds, Context, DisplayEvent, DisplayId, ElementInputHandler, Entity,
        EntityInputHandler, FocusHandle, KeyBinding, Pixels, Point, SessionLockEvent,
        UTF16Selection, Window, WindowBounds, WindowKind, WindowOptions, actions, canvas, div,
        point, prelude::*, px, rgb, size, white,
    };
    use gpui_platform::application;

    actions!(session_lock, [Submit, Backspace]);

    /// Shared by every lock window, so each display shows the same dots.
    struct Password {
        text: String,
        marked: Option<Range<usize>>,
        wrong: bool,
    }

    struct LockView {
        password: Entity<Password>,
        focus_handle: FocusHandle,
    }

    impl LockView {
        fn new(password: Entity<Password>, cx: &mut Context<Self>) -> Self {
            cx.observe(&password, |_, _, cx| cx.notify()).detach();
            cx.spawn(async move |this, cx| {
                loop {
                    let seconds = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.as_secs());
                    cx.background_executor()
                        .timer(Duration::from_secs(60 - seconds % 60))
                        .await;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            })
            .detach();
            Self {
                password,
                focus_handle: cx.focus_handle(),
            }
        }

        fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
            let correct = self.password.read(cx).text == "test";
            eprintln!("event=submit correct={correct}");
            if correct {
                cx.unlock_session();
                cx.quit();
            } else {
                self.password.update(cx, |password, cx| {
                    password.text.clear();
                    password.marked = None;
                    password.wrong = true;
                    cx.notify();
                });
            }
        }

        fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
            self.password.update(cx, |password, cx| {
                password.text.pop();
                cx.notify();
            });
        }
    }

    impl EntityInputHandler for LockView {
        fn text_for_range(
            &mut self,
            _: Range<usize>,
            _: &mut Option<Range<usize>>,
            _: &mut Window,
            _: &mut Context<Self>,
        ) -> Option<String> {
            None
        }

        fn selected_text_range(
            &mut self,
            _: bool,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) -> Option<UTF16Selection> {
            let end = self.password.read(cx).text.encode_utf16().count();
            Some(UTF16Selection {
                range: end..end,
                reversed: false,
            })
        }

        fn marked_text_range(
            &self,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) -> Option<Range<usize>> {
            self.password.read(cx).marked.clone()
        }

        fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
            self.password
                .update(cx, |password, _| password.marked = None);
        }

        fn replace_text_in_range(
            &mut self,
            _: Option<Range<usize>>,
            text: &str,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.password.update(cx, |password, cx| {
                if let Some(marked) = password.marked.take() {
                    password.text.truncate(marked.start);
                }
                password.text.push_str(text);
                password.wrong = false;
                eprintln!("event=text-input chars={}", password.text.chars().count());
                cx.notify();
            });
        }

        fn replace_and_mark_text_in_range(
            &mut self,
            _: Option<Range<usize>>,
            text: &str,
            _: Option<Range<usize>>,
            _: &mut Window,
            cx: &mut Context<Self>,
        ) {
            self.password.update(cx, |password, cx| {
                let start = password
                    .marked
                    .take()
                    .map_or(password.text.len(), |marked| marked.start);
                password.text.truncate(start);
                password.text.push_str(text);
                password.marked = Some(start..password.text.len());
                cx.notify();
            });
        }

        fn bounds_for_range(
            &mut self,
            _: Range<usize>,
            bounds: Bounds<Pixels>,
            _: &mut Window,
            _: &mut Context<Self>,
        ) -> Option<Bounds<Pixels>> {
            Some(bounds)
        }

        fn character_index_for_point(
            &mut self,
            _: Point<Pixels>,
            _: &mut Window,
            _: &mut Context<Self>,
        ) -> Option<usize> {
            None
        }
    }

    impl Render for LockView {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());
            let password = self.password.read(cx);
            let dots = "\u{25cf}".repeat(password.text.chars().count());
            let hint = if password.wrong {
                "Wrong password"
            } else {
                "Type 'test' and press Enter"
            };
            let viewport = window.viewport_size();
            let entity = cx.entity();
            let focus_handle = self.focus_handle.clone();
            div()
                .key_context("LockView")
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::submit))
                .on_action(cx.listener(Self::backspace))
                .size_full()
                .bg(rgb(0x101018))
                .text_color(white())
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_4()
                .child(div().text_size(px(96.)).child(format!(
                    "{:02}:{:02}",
                    (now / 3600) % 24,
                    (now / 60) % 60
                )))
                .child(
                    div()
                        .w(px(360.))
                        .h(px(48.))
                        .px_3()
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(rgb(if password.wrong { 0xf7768e } else { 0x7aa2f7 }))
                        .rounded_md()
                        .text_size(px(24.))
                        .child(dots)
                        .child(canvas(
                            |_, _, _| {},
                            move |bounds, (), window, cx| {
                                window.handle_input(
                                    &focus_handle,
                                    ElementInputHandler::new(bounds, entity),
                                    cx,
                                );
                            },
                        )),
                )
                .child(div().text_size(px(16.)).child(format!(
                    "{hint} ({}x{})",
                    f32::from(viewport.width),
                    f32::from(viewport.height)
                )))
        }
    }

    fn open_lock_window(display: DisplayId, password: &Entity<Password>, cx: &mut App) {
        let password = password.clone();
        let result = cx.open_window(
            WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(800.), px(600.)),
                })),
                display_id: Some(display),
                app_id: Some("gpui-session-lock-example".to_string()),
                kind: WindowKind::SessionLock,
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| LockView::new(password, cx));
                let focus_handle = view.read(cx).focus_handle.clone();
                window.focus(&focus_handle, cx);
                view
            },
        );
        match result {
            Ok(_) => eprintln!("event=window-opened display={display:?}"),
            Err(error) => {
                eprintln!("event=window-failed display={display:?} error={error:#}");
                // On Linux a quit requested before the event loop starts is lost.
                cx.spawn(async |cx| cx.update(|cx| cx.quit())).detach();
            }
        }
    }

    pub fn main() {
        application().run(|cx: &mut App| {
            cx.bind_keys([
                KeyBinding::new("enter", Submit, Some("LockView")),
                KeyBinding::new("backspace", Backspace, Some("LockView")),
            ]);
            cx.on_session_lock_event(|event, cx| {
                eprintln!("event=session-lock state={event:?}");
                if event == SessionLockEvent::Finished {
                    cx.quit();
                }
            })
            .detach();

            let password = cx.new(|_| Password {
                text: String::new(),
                marked: None,
                wrong: false,
            });
            cx.observe_displays({
                let password = password.clone();
                move |event, cx| match event {
                    DisplayEvent::Added(display) => open_lock_window(display, &password, cx),
                    DisplayEvent::Removed(display) => {
                        eprintln!("event=display-removed display={display:?}")
                    }
                    _ => {}
                }
            })
            .detach();
            for display in cx.displays() {
                open_lock_window(display.id(), &password, cx);
            }
        });
    }
}
