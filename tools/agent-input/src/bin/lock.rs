use std::{
    io::{BufRead, Write},
    os::fd::AsFd,
    sync::mpsc,
    thread,
    time::Duration,
};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1, ext_session_lock_surface_v1, ext_session_lock_v1,
};

#[derive(Default)]
struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    manager: Option<ext_session_lock_manager_v1::ExtSessionLockManagerV1>,
    lock: Option<ext_session_lock_v1::ExtSessionLockV1>,
    outputs: Vec<wl_output::WlOutput>,
    surfaces: Vec<wl_surface::WlSurface>,
    buffers: Vec<wl_buffer::WlBuffer>,
    locked: bool,
    finished: bool,
}
fn record(event: &str) {
    println!("{{\"event\":\"{event}\"}}");
    std::io::stdout().flush().unwrap();
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => s.compositor = Some(registry.bind(name, version.min(4), qh, ())),
                "wl_shm" => s.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_output" => s.outputs.push(registry.bind(name, version.min(3), qh, ())),
                "ext_session_lock_manager_v1" => s.manager = Some(registry.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}
impl Dispatch<ext_session_lock_v1::ExtSessionLockV1, ()> for State {
    fn event(
        s: &mut Self,
        _: &ext_session_lock_v1::ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => {
                s.locked = true;
                record("locked");
            }
            ext_session_lock_v1::Event::Finished => {
                s.finished = true;
                record("finished");
            }
            _ => {}
        }
    }
}
impl Dispatch<ext_session_lock_surface_v1::ExtSessionLockSurfaceV1, usize> for State {
    fn event(
        s: &mut Self,
        lock_surface: &ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        index: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            lock_surface.ack_configure(serial);
            let mut file = tempfile::tempfile().unwrap();
            let pixel: u32 = 0xff202020;
            let mut bytes = vec![0; (width * height * 4) as usize];
            for chunk in bytes.chunks_exact_mut(4) {
                chunk.copy_from_slice(&pixel.to_ne_bytes());
            }
            file.write_all(&bytes).unwrap();
            let pool =
                s.shm
                    .as_ref()
                    .unwrap()
                    .create_pool(file.as_fd(), bytes.len() as i32, qh, ());
            let buffer = pool.create_buffer(
                0,
                width as i32,
                height as i32,
                width as i32 * 4,
                wl_shm::Format::Argb8888,
                qh,
                (),
            );
            s.surfaces[*index].attach(Some(&buffer), 0, 0);
            s.surfaces[*index].damage(0, 0, width as i32, height as i32);
            s.surfaces[*index].commit();
            pool.destroy();
            s.buffers.push(buffer);
        }
    }
}
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore wl_output::WlOutput);
delegate_noop!(State: ignore ext_session_lock_manager_v1::ExtSessionLockManagerV1);
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::connect_to_env()?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state)?;
    let lock = state
        .manager
        .as_ref()
        .ok_or("session-lock protocol unavailable")?
        .lock(&qh, ());
    for output in &state.outputs {
        let surface = state
            .compositor
            .as_ref()
            .ok_or("compositor unavailable")?
            .create_surface(&qh, ());
        lock.get_lock_surface(&surface, output, &qh, state.surfaces.len());
        state.surfaces.push(surface);
    }
    state.lock = Some(lock);
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        let _ = send.send(());
    });
    while !state.finished {
        queue.roundtrip(&mut state)?;
        if receive.try_recv().is_ok() {
            if state.locked {
                state.lock.as_ref().unwrap().unlock_and_destroy();
                record("unlocked");
            } else {
                state.lock.as_ref().unwrap().destroy();
            }
            queue.roundtrip(&mut state)?;
            connection.flush()?;
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Err("compositor refused session lock".into())
}
