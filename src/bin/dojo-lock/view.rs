//! One lock window per output. It paints the model's scene for its output and
//! turns input into model events; it holds no lock state of its own.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use dojo::lock::{EventKind, Key, Line, OutputId, Panel, Role, Scene};
use dojo::session::paint::{self, Look};
use gpui::{
    App, Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, KeyBinding,
    MouseButton, Pixels, Point, UTF16Selection, Window, actions, canvas, div, prelude::*,
};

use crate::app::Locker;

actions!(dojo_lock, [Submit, Erase, Cancel, ClearLine]);

const CONTEXT: &str = "LockView";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Submit, Some(CONTEXT)),
        KeyBinding::new("backspace", Erase, Some(CONTEXT)),
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("ctrl-u", ClearLine, Some(CONTEXT)),
    ]);
}

/// The scene a window last painted, read by `Locker::sync` to skip unchanged windows.
pub type Painted = Rc<RefCell<Option<Scene>>>;

pub struct LockView {
    output: OutputId,
    locker: Entity<Locker>,
    look: Rc<Look>,
    focus: FocusHandle,
    painted: Painted,
    rendered: bool,
    _viewport: gpui::Subscription,
}

impl LockView {
    pub fn new(
        output: OutputId,
        locker: Entity<Locker>,
        look: Rc<Look>,
        painted: Painted,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> LockView {
        // The compositor's configure is the size that counts, not the display's bounds.
        let viewport = cx.observe_window_bounds(window, |this: &mut LockView, window, cx| {
            let (output, size) = (this.output, window.viewport_size());
            this.locker
                .update(cx, |locker, cx| locker.viewport(output, size, cx));
        });
        LockView {
            output,
            locker,
            look,
            focus: cx.focus_handle(),
            painted,
            rendered: false,
            _viewport: viewport,
        }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn send(&mut self, kind: EventKind, cx: &mut Context<Self>) {
        self.locker
            .update(cx, |locker, cx| locker.dispatch(kind, cx));
    }

    fn key(&mut self, key: Key, cx: &mut Context<Self>) {
        let output = self.output;
        self.send(EventKind::Key(output, key), cx);
    }

    /// Any input on this output. The reducer reads `Pointer` as activity there:
    /// it takes the active output, wakes the power timeline and skips a greeting.
    fn activity(&mut self, cx: &mut Context<Self>) {
        let output = self.output;
        self.send(EventKind::Pointer(output), cx);
    }
}

impl Render for LockView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sized = self.locker.read(cx).output_size(self.output) == Some(window.viewport_size());
        let scene = sized
            .then(|| self.locker.read(cx).scene(self.output))
            .flatten();
        let root = div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .font_family(dojo::power::FONT_FAMILY)
            .font_features(dojo::power::font_features())
            .bg(self.look.palette.background)
            .on_action(cx.listener(|this, _: &Submit, _, cx| this.key(Key::Enter, cx)))
            .on_action(cx.listener(|this, _: &Erase, _, cx| this.key(Key::Backspace, cx)))
            .on_action(cx.listener(|this, _: &Cancel, _, cx| this.key(Key::Escape, cx)))
            .on_action(cx.listener(|this, _: &ClearLine, _, cx| this.key(Key::ClearLine, cx)))
            .on_key_down(cx.listener(|this, _, _, cx| this.activity(cx)))
            .on_modifiers_changed(cx.listener(|this, _, _, cx| this.activity(cx)))
            .on_mouse_move(cx.listener(|this, _, _, cx| this.activity(cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.activity(cx)),
            );
        let Some(scene) = scene else {
            return root;
        };
        if scene.animating {
            window.request_animation_frame();
        }
        if let Role::Active(Panel {
            line: Some(Line::Greeting { text, alpha }),
            ..
        }) = &scene.role
            && *alpha > 0.
            && !self.locker.read(cx).greeting_stored()
        {
            let (locker, text) = (self.locker.clone(), text.clone());
            cx.defer(move |cx| locker.update(cx, |locker, _| locker.greeting_shown(text)));
        }
        if !self.rendered {
            self.rendered = true;
            crate::app::log(&format!(
                "event=first-render output={} viewport={:?} scale={}",
                self.output.0,
                window.viewport_size(),
                window.scale_factor()
            ));
        }
        trace_frame(&scene);
        *self.painted.borrow_mut() = Some(scene.clone());

        let mut children = paint::scene(&scene, &self.look, window.viewport_size());
        let entity = cx.entity();
        let focus = self.focus.clone();
        children.push(
            canvas(
                |_, _, _| {},
                move |bounds, (), window, cx| {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                },
            )
            .absolute()
            .size_full()
            .into_any_element(),
        );
        if scene.dim < 1. {
            children.push(
                div()
                    .absolute()
                    .size_full()
                    .bg(gpui::black().opacity(1. - scene.dim))
                    .into_any_element(),
            );
        }
        root.children(children)
    }
}

/// One line per frame for the nested capture runs (`tools/agent-input/morph-frames.py`).
/// A build without `record-session-actions` logs nothing.
#[cfg(feature = "record-session-actions")]
fn trace_frame(scene: &Scene) {
    let dojo::session::transition::Pose::Moving(layout) = scene.pose else {
        return;
    };
    eprintln!(
        "dojo-lock-trace: frame ring={:.3} text={:.3} open_w={:.1} wallpaper={:.2} quiet={:.2} animating={} unix_ms={}",
        layout.ring,
        layout.text,
        f32::from(layout.opening.size.width),
        layout.wallpaper,
        scene.quiet,
        scene.animating,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis()),
    );
}

#[cfg(not(feature = "record-session-actions"))]
fn trace_frame(_: &Scene) {}

/// Text arrives here, dead keys and compose included. Composition is never
/// shown or kept: only committed text becomes `Key::Char`.
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
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for c in text.chars().filter(|c| !c.is_control()) {
            self.key(Key::Char(c), cx);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activity(cx);
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
