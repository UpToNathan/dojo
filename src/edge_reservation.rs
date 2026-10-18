//! Exclusive zones for the desktop frame's four edges, held without a GPU surface.
//!
//! The frame reserves its edges with layer-shell surfaces whose only job is an exclusive zone.
//! As GPUI windows each carried a swapchain and cost about 27 ms of the main thread at start-up
//! (the first, with the one-time GPU set-up, about 108 ms). These are the same surfaces made
//! directly: a transparent, input-less 1x1 shared-memory buffer scaled to the strip by a
//! viewport. They are created on a thread with its own Wayland connection, which connects
//! while GPUI initializes, so the main thread never waits for them.
//!
//! Anything missing (the connection, a protocol, the output) leaves the caller to fall back to
//! GPUI windows, or reports the reservation inactive so the frame is opened again.

use std::collections::HashMap;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use uuid::Uuid;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_region, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{self, ZwlrLayerShellV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1,
};

/// How long a reservation waits for its output to be announced before it gives up.
const OUTPUT_WAIT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    fn name(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Bottom => "bottom",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }

    /// Anchored to the edge and both perpendicular ones, which is the one shape whose exclusive
    /// zone needs no `set_exclusive_edge` (a request that older compositors lack).
    fn anchor(self) -> Anchor {
        match self {
            Edge::Top => Anchor::Top | Anchor::Left | Anchor::Right,
            Edge::Bottom => Anchor::Bottom | Anchor::Left | Anchor::Right,
            Edge::Left => Anchor::Left | Anchor::Top | Anchor::Bottom,
            Edge::Right => Anchor::Right | Anchor::Top | Anchor::Bottom,
        }
    }

    /// The requested size; a zero lets the compositor span the anchored axis.
    fn size(self, thickness: u32) -> (u32, u32) {
        match self {
            Edge::Top | Edge::Bottom => (0, thickness),
            Edge::Left | Edge::Right => (thickness, 0),
        }
    }
}

/// The uuid GPUI gives a display, which is derived from the output's name.
pub fn display_uuid(output_name: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_DNS, output_name.as_bytes())
}

enum Command {
    Reserve {
        id: u64,
        output: Uuid,
        strips: Vec<(Edge, u32)>,
        alive: Arc<AtomicBool>,
    },
    Release(u64),
}

struct Service {
    commands: Sender<Command>,
    wake: Arc<OwnedFd>,
    next_id: std::sync::atomic::AtomicU64,
}

static SERVICE: OnceLock<Option<Service>> = OnceLock::new();

/// Starts connecting in the background so the first reservation does not wait for it.
pub fn prewarm() {
    let _ = service();
}

fn service() -> Option<&'static Service> {
    SERVICE
        .get_or_init(|| match Service::start() {
            Ok(service) => Some(service),
            Err(error) => {
                eprintln!("Dojo reserves its edges with windows instead: {error:#}");
                None
            }
        })
        .as_ref()
}

/// The edges of one output, held until dropped. `None` when the direct path is unavailable.
pub fn reserve(output: Uuid, strips: &[(Edge, u32)]) -> Option<Reservation> {
    let service = service()?;
    let id = service
        .next_id
        .fetch_add(1, Ordering::Relaxed);
    let alive = Arc::new(AtomicBool::new(true));
    service
        .commands
        .send(Command::Reserve {
            id,
            output,
            strips: strips.to_vec(),
            alive: alive.clone(),
        })
        .ok()?;
    service.wake();
    Some(Reservation { id, alive })
}

pub struct Reservation {
    id: u64,
    alive: Arc<AtomicBool>,
}

impl Reservation {
    /// False once the compositor closed a strip (its output went away), the output never
    /// appeared, or the connection failed.
    pub fn is_active(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(Some(service)) = SERVICE.get() {
            let _ = service.commands.send(Command::Release(self.id));
            service.wake();
        }
    }
}

impl Service {
    fn start() -> anyhow::Result<Self> {
        let connection = Connection::connect_to_env()?;
        let (commands, receiver) = channel();
        let wake = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        anyhow::ensure!(wake >= 0, "could not create a wake-up descriptor");
        let wake = Arc::new(unsafe { OwnedFd::from_raw_fd(wake) });
        let thread_wake = wake.clone();
        std::thread::Builder::new()
            .name("dojo-edge-reservations".into())
            .spawn(move || {
                if let Err(error) = run(connection, receiver, &thread_wake) {
                    eprintln!("Dojo edge reservations stopped: {error:#}");
                }
            })?;
        Ok(Self {
            commands,
            wake,
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    fn wake(&self) {
        let one = 1u64.to_ne_bytes();
        unsafe { libc::write(self.wake.as_raw_fd(), one.as_ptr().cast(), one.len()) };
    }
}

struct Output {
    global: u32,
    output: wl_output::WlOutput,
    name: Option<String>,
}

struct Strip {
    surface: wl_surface::WlSurface,
    layer: ZwlrLayerSurfaceV1,
    viewport: WpViewport,
    requested: (u32, u32),
    mapped: bool,
}

impl Drop for Strip {
    fn drop(&mut self) {
        self.viewport.destroy();
        self.layer.destroy();
        self.surface.destroy();
    }
}

struct Held {
    alive: Arc<AtomicBool>,
    strips: Vec<Strip>,
}

struct Pending {
    id: u64,
    output: Uuid,
    strips: Vec<(Edge, u32)>,
    alive: Arc<AtomicBool>,
    since: Instant,
}

#[derive(Clone, Copy)]
struct StripKey {
    reservation: u64,
    index: usize,
}

struct State {
    compositor: wl_compositor::WlCompositor,
    layer_shell: ZwlrLayerShellV1,
    viewporter: WpViewporter,
    /// One transparent pixel, shared by every strip.
    buffer: wl_buffer::WlBuffer,
    outputs: Vec<Output>,
    held: HashMap<u64, Held>,
    pending: Vec<Pending>,
}

impl State {
    fn output_for(&self, wanted: Uuid) -> Option<&Output> {
        self.outputs.iter().find(|output| {
            output
                .name
                .as_deref()
                .is_some_and(|name| display_uuid(name) == wanted)
        })
    }

    /// Creates the strips of every waiting reservation whose output is known, and gives up on
    /// the ones whose output never appeared.
    fn place_pending(&mut self, qh: &QueueHandle<Self>) {
        let waiting = std::mem::take(&mut self.pending);
        for pending in waiting {
            if !pending.alive.load(Ordering::Acquire) {
                continue;
            }
            if let Some(output) = self.output_for(pending.output) {
                let name = output.name.clone().unwrap_or_default();
                let strips = pending
                    .strips
                    .iter()
                    .enumerate()
                    .map(|(index, &(edge, thickness))| {
                        self.create_strip(
                            qh,
                            &output.output,
                            &name,
                            StripKey { reservation: pending.id, index },
                            edge,
                            thickness,
                        )
                    })
                    .collect();
                self.held.insert(pending.id, Held { alive: pending.alive, strips });
            } else if pending.since.elapsed() > OUTPUT_WAIT {
                eprintln!("Dojo could not find the output to reserve its edges on.");
                pending.alive.store(false, Ordering::Release);
            } else {
                self.pending.push(pending);
            }
        }
    }

    fn create_strip(
        &self,
        qh: &QueueHandle<Self>,
        output: &wl_output::WlOutput,
        output_name: &str,
        key: StripKey,
        edge: Edge,
        thickness: u32,
    ) -> Strip {
        let surface = self.compositor.create_surface(qh, ());
        let layer = self.layer_shell.get_layer_surface(
            &surface,
            Some(output),
            zwlr_layer_shell_v1::Layer::Top,
            format!("dojo-reservation-{}-{output_name}", edge.name()),
            qh,
            key,
        );
        let requested = edge.size(thickness);
        layer.set_size(requested.0, requested.1);
        layer.set_anchor(edge.anchor());
        layer.set_exclusive_zone(thickness as i32);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        // An empty input region: the strip never takes a click from the frame beneath it.
        let region = self.compositor.create_region(qh, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        let viewport = self.viewporter.get_viewport(&surface, qh, ());
        // The first commit carries no buffer; the surface maps when the configure is answered.
        surface.commit();
        Strip { surface, layer, viewport, requested, mapped: false }
    }

    fn handle(&mut self, command: Command, qh: &QueueHandle<Self>) {
        match command {
            Command::Reserve { id, output, strips, alive } => {
                self.pending.push(Pending {
                    id,
                    output,
                    strips,
                    alive,
                    since: Instant::now(),
                });
            }
            Command::Release(id) => {
                self.held.remove(&id);
                self.pending.retain(|pending| pending.id != id);
            }
        }
        self.place_pending(qh);
    }
}

/// A 1x1 ARGB buffer of zeros: fully transparent, and the viewport scales it to any strip.
fn transparent_pixel(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<State>,
) -> anyhow::Result<wl_buffer::WlBuffer> {
    let descriptor = unsafe { libc::memfd_create(c"dojo-edge-reservation".as_ptr(), libc::MFD_CLOEXEC) };
    anyhow::ensure!(descriptor >= 0, "could not create a shared-memory file");
    let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
    // A fresh file reads as zeros once it has a size.
    anyhow::ensure!(
        unsafe { libc::ftruncate(descriptor.as_raw_fd(), 4) } == 0,
        "could not size the shared-memory file"
    );
    let pool = shm.create_pool(descriptor.as_fd(), 4, qh, ());
    let buffer = pool.create_buffer(0, 1, 1, 4, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buffer)
}

fn run(
    connection: Connection,
    commands: Receiver<Command>,
    wake: &OwnedFd,
) -> anyhow::Result<()> {
    let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
    let qh = queue.handle();
    let compositor = globals
        .bind::<wl_compositor::WlCompositor, _, _>(&qh, 1..=4, ())
        .map_err(|error| anyhow::anyhow!("wl_compositor: {error}"))?;
    let layer_shell = globals
        .bind::<ZwlrLayerShellV1, _, _>(&qh, 1..=4, ())
        .map_err(|error| anyhow::anyhow!("zwlr_layer_shell_v1: {error}"))?;
    let viewporter = globals
        .bind::<WpViewporter, _, _>(&qh, 1..=1, ())
        .map_err(|error| anyhow::anyhow!("wp_viewporter: {error}"))?;
    let shm = globals
        .bind::<wl_shm::WlShm, _, _>(&qh, 1..=1, ())
        .map_err(|error| anyhow::anyhow!("wl_shm: {error}"))?;
    let buffer = transparent_pixel(&shm, &qh)?;
    let announced: Vec<(u32, u32)> = globals.contents().with_list(|list| {
        list.iter()
            .filter(|global| global.interface == "wl_output")
            .map(|global| (global.name, global.version))
            .collect()
    });
    let outputs = announced
        .into_iter()
        .map(|(global, version)| Output {
            global,
            output: globals.registry().bind(global, version.min(4), &qh, global),
            name: None,
        })
        .collect();
    let mut state = State {
        compositor,
        layer_shell,
        viewporter,
        buffer,
        outputs,
        held: HashMap::new(),
        pending: Vec::new(),
    };
    // Output names arrive in the events that follow the binds.
    queue.roundtrip(&mut state)?;

    loop {
        loop {
            match commands.try_recv() {
                Ok(command) => state.handle(command, &qh),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        state.place_pending(&qh);
        connection.flush()?;
        queue.dispatch_pending(&mut state)?;
        state.place_pending(&qh);
        connection.flush()?;
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut descriptors = [
            libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        let timeout = if state.pending.is_empty() { -1 } else { 500 };
        let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), 2, timeout) };
        if ready < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(std::io::Error::last_os_error().into());
        }
        if descriptors[0].revents & libc::POLLIN != 0 {
            guard.read()?;
        } else {
            drop(guard);
        }
        if descriptors[1].revents & libc::POLLIN != 0 {
            let mut count = [0u8; 8];
            unsafe { libc::read(wake.as_raw_fd(), count.as_mut_ptr().cast(), count.len()) };
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        for held in self.held.values() {
            held.alive.store(false, Ordering::Release);
        }
        for pending in &self.pending {
            pending.alive.store(false, Ordering::Release);
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } if interface == "wl_output" => {
                let output = registry.bind(name, version.min(4), qh, name);
                state.outputs.push(Output { global: name, output, name: None });
            }
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.retain(|output| output.global != name);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event
            && let Some(output) = state.outputs.iter_mut().find(|output| output.global == *global)
        {
            output.name = Some(name);
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, StripKey> for State {
    fn event(
        state: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        key: &StripKey,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer.ack_configure(serial);
                let buffer = state.buffer.clone();
                let Some(strip) = state
                    .held
                    .get_mut(&key.reservation)
                    .and_then(|held| held.strips.get_mut(key.index))
                else {
                    return;
                };
                let width = if width == 0 { strip.requested.0 } else { width };
                let height = if height == 0 { strip.requested.1 } else { height };
                strip.viewport.set_destination(width.max(1) as i32, height.max(1) as i32);
                if !strip.mapped {
                    strip.surface.attach(Some(&buffer), 0, 0);
                    strip.mapped = true;
                }
                strip.surface.damage_buffer(0, 0, 1, 1);
                strip.surface.commit();
            }
            zwlr_layer_surface_v1::Event::Closed => {
                if let Some(held) = state.held.remove(&key.reservation) {
                    held.alive.store(false, Ordering::Release);
                }
            }
            _ => {}
        }
    }
}

macro_rules! ignore_events {
    ($($interface:ty),* $(,)?) => {$(
        impl Dispatch<$interface, ()> for State {
            fn event(
                _: &mut Self,
                _: &$interface,
                _: <$interface as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}

ignore_events!(
    wl_compositor::WlCompositor,
    wl_surface::WlSurface,
    wl_region::WlRegion,
    wl_buffer::WlBuffer,
    ZwlrLayerShellV1,
    WpViewporter,
    WpViewport,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strip_anchors_its_edge_and_both_perpendicular_ones() {
        assert_eq!(Edge::Top.anchor(), Anchor::Top | Anchor::Left | Anchor::Right);
        assert_eq!(Edge::Bottom.anchor(), Anchor::Bottom | Anchor::Left | Anchor::Right);
        assert_eq!(Edge::Left.anchor(), Anchor::Left | Anchor::Top | Anchor::Bottom);
        assert_eq!(Edge::Right.anchor(), Anchor::Right | Anchor::Top | Anchor::Bottom);
    }

    #[test]
    fn a_strip_asks_for_its_thickness_and_lets_the_compositor_span_the_rest() {
        assert_eq!(Edge::Top.size(24), (0, 24));
        assert_eq!(Edge::Bottom.size(12), (0, 12));
        assert_eq!(Edge::Left.size(64), (64, 0));
        assert_eq!(Edge::Right.size(12), (12, 0));
    }

    #[test]
    fn outputs_match_the_uuid_gpui_derives_from_their_name() {
        assert_eq!(display_uuid("eDP-1"), display_uuid("eDP-1"));
        assert_ne!(display_uuid("eDP-1"), display_uuid("DP-2"));
        assert_eq!(
            display_uuid("eDP-1"),
            Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"eDP-1")
        );
    }
}
