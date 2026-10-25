//! Takes the whole pointer and keyboard of this desktop for as long as it runs, and writes what
//! the person does as events, one JSON line each (`dojo_mesh::remote::Event`). `dojo-mesh remote`
//! pipes them to a peer.
//!
//! It covers the focused output with a transparent layer-shell surface that has the keyboard
//! exclusively, locks the pointer to it (so motion keeps coming at the screen's edge) and hides the
//! cursor. Nothing reaches local windows meanwhile. It ends, giving everything back, on Super+F11 or
//! Ctrl+Alt+Shift+Esc, when the compositor takes the surface or the pointer lock away, or when its
//! output is closed.

use std::collections::BTreeSet;
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};

use dojo_mesh::remote::{Event, ReleaseChords};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::wp::pointer_constraints::zv1::client::zwp_locked_pointer_v1::{self, ZwpLockedPointerV1};
use wayland_protocols::wp::pointer_constraints::zv1::client::zwp_pointer_constraints_v1::{Lifetime, ZwpPointerConstraintsV1};
use wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1;
use wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_v1::{self, ZwpRelativePointerV1};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{self, ZwlrLayerShellV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1};

struct State {
    surface: wl_surface::WlSurface,
    viewport: WpViewport,
    buffer: wl_buffer::WlBuffer,
    _seat: wl_seat::WlSeat,
    constraints: ZwpPointerConstraintsV1,
    relative_manager: ZwpRelativePointerManagerV1,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    locked: Option<ZwpLockedPointerV1>,
    relative: Option<ZwpRelativePointerV1>,
    mapped: bool,
    chords: ReleaseChords,
    keys_down: BTreeSet<u32>,
    buttons_down: BTreeSet<u32>,
    stdout: std::io::Stdout,
    done: bool,
}

impl State {
    fn emit(&mut self, event: Event) {
        let mut out = self.stdout.lock();
        // A closed pipe means the other end is gone: nothing is listening, so input comes home.
        if writeln!(out, "{}", event.encode()).and_then(|()| out.flush()).is_err() {
            self.done = true;
        }
    }
}

/// A 1x1 ARGB buffer of zeros: fully transparent, and the viewport stretches it over the output.
fn transparent_pixel(shm: &wl_shm::WlShm, qh: &QueueHandle<State>) -> Result<wl_buffer::WlBuffer, String> {
    let descriptor = unsafe { libc::memfd_create(c"dojo-input-capture".as_ptr(), libc::MFD_CLOEXEC) };
    if descriptor < 0 {
        return Err("could not create a shared-memory file".into());
    }
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    if unsafe { libc::ftruncate(descriptor.as_raw_fd(), 4) } != 0 {
        return Err("could not size the shared-memory file".into());
    }
    let pool = shm.create_pool(descriptor.as_fd(), 4, qh, ());
    let buffer = pool.create_buffer(0, 1, 1, 4, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buffer)
}

fn run() -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|error| format!("no Wayland session: {error}"))?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection).map_err(|error| error.to_string())?;
    let qh = queue.handle();
    let missing = |name: &str, error: wayland_client::globals::BindError| format!("this compositor lacks {name}: {error}");
    let compositor = globals.bind::<wl_compositor::WlCompositor, _, _>(&qh, 1..=4, ()).map_err(|e| missing("wl_compositor", e))?;
    let shm = globals.bind::<wl_shm::WlShm, _, _>(&qh, 1..=1, ()).map_err(|e| missing("wl_shm", e))?;
    let layer_shell = globals.bind::<ZwlrLayerShellV1, _, _>(&qh, 1..=4, ()).map_err(|e| missing("zwlr_layer_shell_v1", e))?;
    let viewporter = globals.bind::<WpViewporter, _, _>(&qh, 1..=1, ()).map_err(|e| missing("wp_viewporter", e))?;
    let seat = globals.bind::<wl_seat::WlSeat, _, _>(&qh, 5..=7, ()).map_err(|e| missing("wl_seat", e))?;
    let constraints = globals.bind::<ZwpPointerConstraintsV1, _, _>(&qh, 1..=1, ()).map_err(|e| missing("zwp_pointer_constraints_v1", e))?;
    let relative_manager = globals.bind::<ZwpRelativePointerManagerV1, _, _>(&qh, 1..=1, ()).map_err(|e| missing("zwp_relative_pointer_manager_v1", e))?;

    let surface = compositor.create_surface(&qh, ());
    let viewport = viewporter.get_viewport(&surface, &qh, ());
    let layer = layer_shell.get_layer_surface(&surface, None, zwlr_layer_shell_v1::Layer::Overlay, "dojo-input-capture".into(), &qh, ());
    layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.set_size(0, 0);
    surface.commit();
    let buffer = transparent_pixel(&shm, &qh)?;

    let mut state = State {
        surface,
        viewport,
        buffer,
        _seat: seat,
        constraints,
        relative_manager,
        pointer: None,
        keyboard: None,
        locked: None,
        relative: None,
        mapped: false,
        chords: ReleaseChords::default(),
        keys_down: BTreeSet::new(),
        buttons_down: BTreeSet::new(),
        stdout: std::io::stdout(),
        done: false,
    };
    while !state.done {
        queue.blocking_dispatch(&mut state).map_err(|error| format!("lost the Wayland connection: {error}"))?;
    }
    // Everything still held is let go, so the peer is not left with a stuck key or button.
    for code in std::mem::take(&mut state.keys_down) {
        state.emit(Event::Key { code, pressed: false });
    }
    for code in std::mem::take(&mut state.buttons_down) {
        state.emit(Event::Button { code, pressed: false });
    }
    layer.destroy();
    state.surface.destroy();
    let _ = connection.roundtrip();
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dojo-input-capture: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(state: &mut Self, layer: &ZwlrLayerSurfaceV1, event: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer.ack_configure(serial);
                state.viewport.set_destination(width.max(1) as i32, height.max(1) as i32);
                if !state.mapped {
                    state.surface.attach(Some(&state.buffer), 0, 0);
                    state.mapped = true;
                }
                state.surface.damage_buffer(0, 0, 1, 1);
                state.surface.commit();
            }
            // The compositor took the surface away (the screen locked, or the output went).
            zwlr_layer_surface_v1::Event::Closed => state.done = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &wl_seat::WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(capabilities) } = event {
            if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(state: &mut Self, pointer: &wl_pointer::WlPointer, event: wl_pointer::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        match event {
            wl_pointer::Event::Enter { serial, .. } => {
                // No cursor while the input is somewhere else, and the pointer stays put so motion never stops at an edge.
                pointer.set_cursor(serial, None, 0, 0);
                if state.locked.is_none() {
                    state.locked = Some(state.constraints.lock_pointer(&state.surface, pointer, None, Lifetime::Persistent, qh, ()));
                    state.relative = Some(state.relative_manager.get_relative_pointer(pointer, qh, ()));
                }
            }
            wl_pointer::Event::Button { button, state: WEnum::Value(button_state), .. } => {
                let pressed = button_state == wl_pointer::ButtonState::Pressed;
                if pressed {
                    state.buttons_down.insert(button);
                } else {
                    state.buttons_down.remove(&button);
                }
                state.emit(Event::Button { code: button, pressed });
            }
            wl_pointer::Event::Axis { axis: WEnum::Value(axis), value, .. } => {
                let event = match axis {
                    wl_pointer::Axis::HorizontalScroll => Event::Axis { h: value, v: 0. },
                    _ => Event::Axis { h: 0., v: value },
                };
                state.emit(event);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(state: &mut Self, _: &wl_keyboard::WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            // evdev codes are sent as they are; the peer has its own keymap, so this one is not needed.
            wl_keyboard::Event::Key { key, state: WEnum::Value(key_state), .. } => {
                let pressed = key_state == wl_keyboard::KeyState::Pressed;
                if pressed {
                    state.keys_down.insert(key);
                } else {
                    state.keys_down.remove(&key);
                }
                if state.chords.feed(key, pressed) {
                    // The chord itself is not sent: the peer never sees the key that brings input home.
                    if pressed {
                        state.keys_down.remove(&key);
                    }
                    state.done = true;
                    return;
                }
                state.emit(Event::Key { code: key, pressed });
            }
            // Keyboard focus moved elsewhere (a lock screen, say): the session is over.
            wl_keyboard::Event::Leave { .. } => state.done = true,
            _ => {}
        }
    }
}

impl Dispatch<ZwpRelativePointerV1, ()> for State {
    fn event(state: &mut Self, _: &ZwpRelativePointerV1, event: zwp_relative_pointer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let zwp_relative_pointer_v1::Event::RelativeMotion { dx, dy, .. } = event {
            state.emit(Event::Motion { dx, dy });
        }
    }
}

impl Dispatch<ZwpLockedPointerV1, ()> for State {
    fn event(state: &mut Self, _: &ZwpLockedPointerV1, event: zwp_locked_pointer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        // The compositor ended the lock: it is no longer ours to hold.
        if let zwp_locked_pointer_v1::Event::Unlocked = event {
            state.done = true;
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

macro_rules! ignore_events {
    ($($interface:ty),* $(,)?) => {$(
        impl Dispatch<$interface, ()> for State {
            fn event(_: &mut Self, _: &$interface, _: <$interface as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    wl_surface::WlSurface,
    ZwlrLayerShellV1,
    WpViewporter,
    WpViewport,
    ZwpPointerConstraintsV1,
    ZwpRelativePointerManagerV1,
);
