use std::{collections::HashMap, io::Write, os::fd::AsFd};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

fn record(event: serde_json::Value) {
    println!("{event}");
    std::io::stdout().flush().unwrap();
}
struct Window {
    surface: wl_surface::WlSurface,
    _xdg: xdg_surface::XdgSurface,
    _top: xdg_toplevel::XdgToplevel,
    size: (i32, i32),
}
#[derive(Default)]
struct Seat {
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard_window: Option<usize>,
    modifiers: u32,
    pointer_window: Option<usize>,
}
#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    shell: Option<xdg_wm_base::XdgWmBase>,
    windows: Vec<Window>,
    seats: HashMap<u32, Seat>,
    buffers: Vec<wl_buffer::WlBuffer>,
    running: bool,
}
impl State {
    fn window(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.windows.iter().position(|w| w.surface == *surface)
    }
    fn create_windows(&mut self, qh: &QueueHandle<Self>) {
        if !self.windows.is_empty() {
            return;
        }
        let (Some(compositor), Some(shell), Some(_)) = (&self.compositor, &self.shell, &self.shm)
        else {
            return;
        };
        for index in 0..2 {
            let surface = compositor.create_surface(qh, ());
            let xdg = shell.get_xdg_surface(&surface, qh, index);
            let top = xdg.get_toplevel(qh, index);
            top.set_title(format!("Agent input fixture {}", index + 1));
            top.set_app_id("dojo-agent-input-fixture".into());
            top.set_min_size(240, 180);
            surface.commit();
            self.windows.push(Window {
                surface,
                _xdg: xdg,
                _top: top,
                size: (320, 240),
            });
        }
    }
    fn draw(&mut self, index: usize, qh: &QueueHandle<Self>) {
        let window = &self.windows[index];
        let (width, height) = window.size;
        let color: u32 = if index == 0 { 0xff285a84 } else { 0xff8a5930 };
        let mut file = tempfile::tempfile().unwrap();
        let mut bytes = vec![0; (width * height * 4) as usize];
        for chunk in bytes.chunks_exact_mut(4) {
            chunk.copy_from_slice(&color.to_ne_bytes());
        }
        file.write_all(&bytes).unwrap();
        let pool = self
            .shm
            .as_ref()
            .unwrap()
            .create_pool(file.as_fd(), bytes.len() as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            width * 4,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );
        window.surface.attach(Some(&buffer), 0, 0);
        window.surface.damage(0, 0, width, height);
        window.surface.commit();
        pool.destroy();
        self.buffers.push(buffer);
        record(
            serde_json::json!({"event":"configured", "window":index + 1, "width":width, "height":height}),
        );
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                match interface.as_str() {
                    "wl_compositor" => {
                        s.compositor = Some(registry.bind(name, version.min(4), qh, ()))
                    }
                    "wl_shm" => s.shm = Some(registry.bind(name, 1, qh, ())),
                    "xdg_wm_base" => s.shell = Some(registry.bind(name, 1, qh, ())),
                    "wl_seat" => {
                        s.seats.insert(name, Seat::default());
                        registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, name);
                        record(serde_json::json!({"event":"seat_added", "seat":name}));
                    }
                    _ => {}
                }
                s.create_windows(qh);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(seat) = s.seats.remove(&name) {
                    if let Some(device) = seat.keyboard {
                        device.release();
                    }
                    if let Some(device) = seat.pointer {
                        device.release();
                    }
                    record(serde_json::json!({"event":"seat_removed", "seat":name}));
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_seat::WlSeat, u32> for State {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        id: &u32,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let Some(state) = s.seats.get_mut(id) else {
            return;
        };
        match event {
            wl_seat::Event::Name { name } => {
                record(serde_json::json!({"event":"seat_name", "seat":id, "name":name}))
            }
            wl_seat::Event::Capabilities {
                capabilities: WEnum::Value(capabilities),
            } => {
                if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none()
                {
                    state.keyboard = Some(seat.get_keyboard(qh, *id));
                }
                if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                    state.pointer = Some(seat.get_pointer(qh, *id));
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, u32> for State {
    fn event(
        s: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, keys, .. } => {
                let window = s.window(&surface);
                if let Some(seat) = s.seats.get_mut(id) {
                    seat.keyboard_window = window;
                }
                record(
                    serde_json::json!({"event":"keyboard_enter", "seat":id, "window":window.map(|w| w+1), "held_keys":keys}),
                );
            }
            wl_keyboard::Event::Leave { .. } => {
                if let Some(seat) = s.seats.get_mut(id) {
                    seat.keyboard_window = None;
                }
                record(serde_json::json!({"event":"keyboard_leave", "seat":id}));
            }
            wl_keyboard::Event::Key { key, state, .. } => record(
                serde_json::json!({"event":"key", "seat":id, "window":s.seats.get(id).and_then(|s|s.keyboard_window).map(|w|w+1), "key":key, "depressed":s.seats.get(id).map(|s|s.modifiers), "state":format!("{state:?}")}),
            ),
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(seat) = s.seats.get_mut(id) {
                    seat.modifiers = mods_depressed;
                }
                record(
                    serde_json::json!({"event":"modifiers", "seat":id, "depressed":mods_depressed, "latched":mods_latched, "locked":mods_locked, "group":group}),
                );
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, u32> for State {
    fn event(
        s: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                let window = s.window(&surface);
                if let Some(seat) = s.seats.get_mut(id) {
                    seat.pointer_window = window;
                }
                record(
                    serde_json::json!({"event":"pointer_enter", "seat":id, "window":window.map(|w| w+1), "x":surface_x, "y":surface_y}),
                );
            }
            wl_pointer::Event::Leave { .. } => {
                if let Some(seat) = s.seats.get_mut(id) {
                    seat.pointer_window = None;
                }
                record(serde_json::json!({"event":"pointer_leave", "seat":id}));
            }
            wl_pointer::Event::Axis { axis, value, .. } => record(serde_json::json!({
                "event":"axis", "seat":id, "window":s.seats.get(id).and_then(|s|s.pointer_window).map(|w|w+1),
                "axis":format!("{axis:?}"), "value":value
            })),
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => record(
                serde_json::json!({"event":"motion", "seat":id, "x":surface_x, "y":surface_y}),
            ),
            wl_pointer::Event::Button { button, state, .. } => record(
                serde_json::json!({"event":"button", "seat":id, "window":s.seats.get(id).and_then(|s|s.pointer_window).map(|w|w+1), "button":button, "state":format!("{state:?}")}),
            ),
            _ => {}
        }
    }
}
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        shell: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            shell.pong(serial);
        }
    }
}
impl Dispatch<xdg_surface::XdgSurface, usize> for State {
    fn event(
        s: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            s.draw(*index, qh);
        }
    }
}
impl Dispatch<xdg_toplevel::XdgToplevel, usize> for State {
    fn event(
        s: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                if width > 0 {
                    s.windows[*index].size.0 = width;
                }
                if height > 0 {
                    s.windows[*index].size.1 = height;
                }
            }
            xdg_toplevel::Event::Close => s.running = false,
            _ => {}
        }
    }
}
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::connect_to_env()?;
    let mut queue = connection.new_event_queue();
    connection.display().get_registry(&queue.handle(), ());
    let mut state = State {
        running: true,
        ..State::default()
    };
    while state.running {
        queue.blocking_dispatch(&mut state)?;
    }
    Ok(())
}
