//! The shell's side of the GlobalShortcuts portal: it starts the backend and shows the panel
//! where the user picks the keys an app asked for.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dojo::shortcuts::compositor::NiriLink;
use dojo::shortcuts::panel::{Mode, Model, Mods, Outcome};
use dojo::shortcuts::portal::{Approver, ChoiceRequest, Portal};
use dojo::shortcuts::registry::Choice;
use dojo::shortcuts::store::Store;
use gpui::{
    App, AppContext, Bounds, Context, FocusHandle, IntoElement, KeyDownEvent, MouseButton, Render,
    Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div,
    layer_shell, point, prelude::*, px,
};
use tokio::sync::oneshot;

use crate::config::ThemeColors;

/// How often an open panel checks that its question is still wanted.
const WITHDRAWN_POLL: Duration = Duration::from_millis(250);

struct Question {
    request: ChoiceRequest,
    reply: oneshot::Sender<Option<Vec<Choice>>>,
}

/// Puts the portal's questions to the user through the panel.
struct PanelApprover {
    questions: async_channel::Sender<Question>,
}

#[async_trait]
impl Approver for PanelApprover {
    async fn choose(&self, request: ChoiceRequest) -> Option<Vec<Choice>> {
        let (reply, answer) = oneshot::channel();
        self.questions
            .send(Question { request, reply })
            .await
            .ok()?;
        answer.await.ok().flatten()
    }

    fn interactive(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct ShortcutsUi {
    // Held for its Drop: it stops the backend and frees every grabbed key.
    portal: Option<Portal>,
    panel: Option<WindowHandle<Panel>>,
    waiting: VecDeque<Question>,
}
impl gpui::Global for ShortcutsUi {}

/// Start the portal backend and the loop that shows its questions.
pub fn initialize(cx: &mut App) {
    cx.set_global(ShortcutsUi::default());
    let (questions, incoming) = async_channel::unbounded();
    cx.spawn(async move |cx| {
        let started = cx
            .background_executor()
            .spawn(async move { start_portal(questions) })
            .await;
        match started {
            Ok(portal) => {
                let _ = cx.update(|cx| cx.update_global::<ShortcutsUi, _>(|ui, _| ui.portal = Some(portal)));
            }
            Err(error) => eprintln!("Dojo could not start its global shortcuts portal: {error:#}"),
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Ok(question) = incoming.recv().await {
            cx.update(|cx| enqueue(question, cx));
        }
    })
    .detach();
}

fn start_portal(questions: async_channel::Sender<Question>) -> anyhow::Result<Portal> {
    let (link, events) = NiriLink::connect()?;
    Portal::start(
        None,
        Some(Store::user_default()?),
        Arc::new(link),
        events,
        Arc::new(PanelApprover { questions }),
    )
}

fn enqueue(question: Question, cx: &mut App) {
    cx.update_global::<ShortcutsUi, _>(|ui, _| ui.waiting.push_back(question));
    show_next(cx);
}

/// Open the next waiting question unless one is already open.
fn show_next(cx: &mut App) {
    let open = cx.update_global::<ShortcutsUi, _>(|ui, _| ui.panel.take());
    if let Some(panel) = open {
        if panel.update(cx, |_, _, _| ()).is_ok() {
            cx.update_global::<ShortcutsUi, _>(|ui, _| ui.panel = Some(panel));
            return;
        }
    }
    while let Some(question) = cx.update_global::<ShortcutsUi, _>(|ui, _| ui.waiting.pop_front()) {
        // A withdrawn question has nobody left to answer it.
        if question.reply.is_closed() {
            continue;
        }
        let display = crate::shell::focused_display_id(cx)
            .or_else(|| cx.primary_display().map(|display| display.id()));
        let Some(display) = display else {
            // The reply drops here, which cancels the bind.
            eprintln!("Dojo has no display to show the shortcuts panel on");
            continue;
        };
        match open_panel(question, display, cx) {
            Ok(panel) => {
                cx.update_global::<ShortcutsUi, _>(|ui, _| ui.panel = Some(panel));
                return;
            }
            Err(error) => eprintln!("Dojo could not open the shortcuts panel: {error:#}"),
        }
    }
}

fn open_panel(
    question: Question,
    display_id: gpui::DisplayId,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Panel>> {
    let display = cx
        .displays()
        .into_iter()
        .find(|display| display.id() == display_id)
        .ok_or_else(|| anyhow::anyhow!("the shortcuts panel's output disappeared"))?;
    let Question { request, reply } = question;
    Ok(cx.open_window(
        WindowOptions {
            display_id: Some(display_id),
            window_bounds: Some(WindowBounds::Windowed(display.bounds())),
            titlebar: None,
            app_id: Some("dojo".into()),
            window_background: WindowBackgroundAppearance::Transparent,
            kind: WindowKind::LayerShell(layer_shell::LayerShellOptions {
                namespace: "dojo-shortcuts".into(),
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
                Panel {
                    model: Model::new(&request),
                    focus,
                    reply: Some(reply),
                    reveal: dojo::design::MenuReveal::opening(Instant::now()),
                    _watch: cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor().timer(WITHDRAWN_POLL).await;
                            let alive = this
                                .update_in(cx, |panel: &mut Panel, window, cx| {
                                    let wanted = panel
                                        .reply
                                        .as_ref()
                                        .is_some_and(|reply| !reply.is_closed());
                                    if !wanted {
                                        // The app withdrew the call: leave without answering.
                                        panel.reply = None;
                                        window.remove_window();
                                        cx.defer(show_next);
                                    }
                                    wanted
                                })
                                .unwrap_or(false);
                            if !alive {
                                break;
                            }
                        }
                    }),
                }
            })
        },
    )?)
}

struct Panel {
    model: Model,
    focus: FocusHandle,
    reply: Option<oneshot::Sender<Option<Vec<Choice>>>>,
    reveal: dojo::design::MenuReveal,
    _watch: gpui::Task<()>,
}

impl Panel {
    fn finish(&mut self, outcome: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        let answer = match outcome {
            Outcome::Pending => return,
            Outcome::Done(choices) => Some(choices),
            Outcome::Cancelled => None,
        };
        if let Some(reply) = self.reply.take() {
            let _ = reply.send(answer);
        }
        window.remove_window();
        cx.defer(show_next);
    }

    fn press(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        let outcome = self.model.key(
            &event.keystroke.key,
            Mods {
                ctrl: modifiers.control,
                alt: modifiers.alt,
                shift: modifiers.shift,
                logo: modifiers.platform,
            },
        );
        self.finish(outcome, window, cx);
        cx.notify();
        cx.stop_propagation();
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.global::<ThemeColors>();
        let viewport = window.viewport_size();
        let (progress, moving) = self
            .reveal
            .sample_with_motion(Instant::now(), cx.reduce_motion());
        if moving {
            window.request_animation_frame();
        }
        let width = 360_f32.min((f32::from(viewport.width) - 24.).max(100.));
        let padding = 18.;
        let gap = 8.;
        let row_padding = 8.;
        let line = 20.;
        let notice_height = if self.model.notice.is_some() { gap + line } else { 0. };
        let content_height = padding * 2.
            + 24.
            + (gap + row_padding * 2. + line) * (self.model.rows.len() as f32 + 1.)
            + notice_height
            + gap
            + 16.;
        let height = content_height.min((f32::from(viewport.height) - 80.).max(80.));
        let right = crate::geometry::RIGHT_INSET as f32;
        let bottom = crate::geometry::BOTTOM_INSET as f32;
        let bounds = Bounds::new(
            point(
                viewport.width - px(right + width),
                viewport.height - px(bottom + height * progress),
            ),
            gpui::size(px(width), px(height * progress)),
        );

        let capturing = self.model.mode == Mode::Capturing;
        let mut content = div()
            .flex()
            .flex_col()
            .gap(px(gap))
            .p(px(padding))
            .text_size(px(13.))
            .line_height(px(line))
            .child(
                div()
                    .text_size(px(16.))
                    .line_height(px(24.))
                    .child(self.model.title.clone()),
            );
        for (index, row) in self.model.rows.iter().enumerate() {
            let selected = self.model.selected == index;
            let waiting = selected && capturing;
            let trigger = if waiting {
                "Press a shortcut…".to_owned()
            } else {
                row.trigger
                    .as_ref()
                    .map_or_else(|| "Not set".to_owned(), ToString::to_string)
            };
            content = content.child(
                div()
                    .id(format!("shortcut-row-{index}"))
                    .flex()
                    .justify_between()
                    .gap(px(12.))
                    .px(px(12.))
                    .py(px(row_padding))
                    .rounded(px(10.))
                    .bg(if selected {
                        colors.background_highlight.rgb()
                    } else {
                        colors.background.rgb()
                    })
                    .text_color(if selected {
                        colors.selection_foreground.rgb()
                    } else {
                        colors.foreground.rgb()
                    })
                    .cursor_pointer()
                    .child(div().child(row.description.clone()))
                    .child(div().text_color(if row.trigger.is_some() || waiting {
                        if selected {
                            colors.selection_foreground.rgb()
                        } else {
                            colors.foreground.rgb()
                        }
                    } else {
                        colors.foreground_muted.rgb()
                    }).child(trigger))
                    .on_hover(cx.listener(move |panel, hovered, _, cx| {
                        if *hovered && panel.model.selected != index {
                            panel.model.select(index);
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |panel, _, window, cx| {
                            panel.model.select(index);
                            let outcome = panel.model.activate();
                            panel.finish(outcome, window, cx);
                            cx.notify();
                            cx.stop_propagation();
                        }),
                    ),
            );
        }
        let done = self.model.done_selected();
        let done_index = self.model.rows.len();
        content = content.child(
            div()
                .id("shortcuts-done")
                .px(px(12.))
                .py(px(row_padding))
                .rounded(px(10.))
                .bg(if done {
                    colors.background_highlight.rgb()
                } else {
                    colors.background.rgb()
                })
                .text_color(if done {
                    colors.selection_foreground.rgb()
                } else {
                    colors.foreground.rgb()
                })
                .cursor_pointer()
                .child("Done")
                .on_hover(cx.listener(move |panel, hovered, _, cx| {
                    if *hovered && panel.model.selected != done_index {
                        panel.model.select(done_index);
                        cx.notify();
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |panel, _, window, cx| {
                        panel.model.select(done_index);
                        let outcome = panel.model.activate();
                        panel.finish(outcome, window, cx);
                        cx.stop_propagation();
                    }),
                ),
        );
        if let Some(notice) = &self.model.notice {
            content = content.child(div().text_size(px(11.)).child(notice.clone()));
        }
        content = content.child(
            div()
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(colors.foreground_muted.rgb())
                .child(if capturing {
                    "Press a shortcut · Backspace clears · Esc stops"
                } else {
                    "Enter change · Backspace clear · Esc cancel"
                }),
        );

        div()
            .size_full()
            .font_family(crate::power::FONT_FAMILY)
            .font_features(crate::power::font_features())
            .text_color(colors.foreground.rgb())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|panel, event: &KeyDownEvent, window, cx| {
                if event.is_held {
                    cx.stop_propagation();
                    return;
                }
                panel.press(event, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |panel, event: &gpui::MouseDownEvent, window, cx| {
                    if !bounds.contains(&event.position) {
                        panel.finish(Outcome::Cancelled, window, cx);
                    }
                }),
            )
            .child(dojo::design::bottom_right_shell_extension(
                bounds,
                colors.background.rgb(),
                colors.border.rgb(),
            ))
            .child(
                div()
                    .id("shortcuts-panel")
                    .absolute()
                    .right(px(right))
                    .bottom(px(bottom))
                    .w(px(width))
                    .h(px(height * progress))
                    .opacity(progress)
                    .overflow_y_scroll()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(content),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn an_empty_queue_opens_nothing(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            cx.set_global(ShortcutsUi::default());
            show_next(cx);
            assert!(cx.global::<ShortcutsUi>().panel.is_none());
        });
    }
}
