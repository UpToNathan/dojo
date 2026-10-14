//! Named, cancellable output capture through ext-image-copy-capture-v1.
//! Derived from CUA's pinned ext_screencopy.rs; see README.md and provenance.json.
use anyhow::{Context, Result, bail, ensure};
use ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1 as Frame;
use ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1 as CopyManager;
use ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1 as Session;
use ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1 as SourceManager;
use image::RgbaImage;
use std::{
    collections::BTreeMap,
    os::{
        fd::{AsFd, AsRawFd},
        unix::net::UnixStream,
    },
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::{wl_buffer, wl_callback, wl_output, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols::ext::{
    image_capture_source::v1::client::{
        ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1, ext_image_copy_capture_manager_v1,
        ext_image_copy_capture_session_v1,
    },
};

pub struct CaptureRequest<'a> {
    pub wayland_socket: &'a Path,
    pub niri_socket: &'a Path,
    pub output: &'a str,
    pub deadline: Instant,
    pub cancelled: &'a AtomicBool,
    pub limits: Limits,
}
#[derive(Clone, Copy)]
pub struct Limits {
    pub max_dimension: u32,
    pub max_pixels: u64,
    pub max_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_dimension: 32768,
            max_pixels: 32 * 1024 * 1024,
            max_bytes: 128 * 1024 * 1024,
        }
    }
}
#[derive(Debug)]
pub struct CapturedOutput {
    /// PNG in displayed orientation, with no cursor requested.
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub buffer_transform: wl_output::Transform,
    pub presentation_time: Duration,
    pub compositor_pid: u32,
}
#[derive(Default)]
struct State {
    outputs: BTreeMap<u32, (wl_output::WlOutput, Option<String>)>,
    source: Option<SourceManager>,
    copy: Option<CopyManager>,
    shm: Option<wl_shm::WlShm>,
    synced: bool,
    done: bool,
    size: Option<(u32, u32)>,
    formats: Vec<wl_shm::Format>,
    ready: bool,
    failure: Option<String>,
    selected: Option<u32>,
    transform: Option<wl_output::Transform>,
    timestamp: Option<Duration>,
}
fn check(request: &CaptureRequest<'_>) -> Result<()> {
    ensure!(
        !request.cancelled.load(Ordering::Acquire),
        "capture canceled"
    );
    ensure!(
        Instant::now() < request.deadline,
        "capture deadline exceeded"
    );
    Ok(())
}
fn connect(path: &Path, request: &CaptureRequest<'_>) -> Result<UnixStream> {
    check(request)?;
    let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
    socket.set_nonblocking(true)?;
    let addr = socket2::SockAddr::unix(path)?;
    if let Err(error) = socket.connect(&addr) {
        ensure!(
            matches!(
                error.raw_os_error(),
                Some(libc::EINPROGRESS) | Some(libc::EAGAIN)
            ),
            "connect {}: {error}",
            path.display()
        );
        loop {
            check(request)?;
            if poll(socket.as_raw_fd(), libc::POLLOUT, request)? {
                break;
            }
        }
        if let Some(error) = socket.take_error()? {
            return Err(error.into());
        }
    }
    let fd: std::os::fd::OwnedFd = socket.into();
    Ok(UnixStream::from(fd))
}
fn peer(stream: &UnixStream) -> Result<libc::ucred> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
    // Both pointers refer to initialized, correctly sized storage.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    ensure!(
        result == 0 && len as usize == std::mem::size_of_val(&cred),
        "socket peer credentials unavailable"
    );
    ensure!(cred.pid > 0, "invalid socket peer PID");
    Ok(cred)
}
fn poll(fd: i32, events: i16, request: &CaptureRequest<'_>) -> Result<bool> {
    check(request)?;
    let timeout = request
        .deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .clamp(1, 20) as i32;
    let mut pfd = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    // pfd is valid for exactly one pollfd.
    let result = unsafe { libc::poll(&mut pfd, 1, timeout) };
    if result < 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(error.into());
    }
    ensure!(
        pfd.revents & (libc::POLLERR | libc::POLLNVAL | libc::POLLHUP) == 0,
        "capture socket disconnected"
    );
    Ok(pfd.revents & events != 0)
}
fn drive(
    queue: &mut wayland_client::EventQueue<State>,
    state: &mut State,
    request: &CaptureRequest<'_>,
    until: impl Fn(&State) -> bool,
) -> Result<()> {
    loop {
        check(request)?;
        queue.dispatch_pending(state)?;
        if let Some(error) = &state.failure {
            bail!("{error}");
        }
        if until(state) {
            return Ok(());
        }
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut events = libc::POLLIN;
        match queue.flush() {
            Ok(()) => {}
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock =>
            {
                events |= libc::POLLOUT
            }
            Err(error) => return Err(error.into()),
        }
        if poll(guard.connection_fd().as_raw_fd(), events, request)? {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
}
fn allocation(width: u32, height: u32, limits: Limits) -> Result<(i32, usize)> {
    ensure!(
        width > 0 && height > 0 && width <= limits.max_dimension && height <= limits.max_dimension,
        "capture dimensions exceed limits"
    );
    ensure!(
        u64::from(width) * u64::from(height) <= limits.max_pixels,
        "capture pixel limit exceeded"
    );
    let stride = width.checked_mul(4).context("capture stride overflow")?;
    let bytes = u64::from(stride)
        .checked_mul(u64::from(height))
        .context("capture size overflow")?;
    ensure!(
        width <= i32::MAX as u32
            && height <= i32::MAX as u32
            && stride <= i32::MAX as u32
            && bytes <= i32::MAX as u64
            && bytes <= limits.max_bytes as u64,
        "capture buffer exceeds limits"
    );
    Ok((stride as i32, bytes as usize))
}
/// Capture only when both explicitly supplied sockets belong to the same process.
/// The deadline covers connection, discovery, buffer dispatch, and frame readiness.
/// Cancellation is checked at least every 20 ms while waiting on sockets.
pub fn capture_output(request: &CaptureRequest<'_>) -> Result<CapturedOutput> {
    ensure!(!request.output.is_empty(), "capture output name is empty");
    ensure!(
        request.wayland_socket.is_absolute() && request.niri_socket.is_absolute(),
        "capture requires absolute socket paths"
    );
    let niri = connect(request.niri_socket, request)?;
    let identity = peer(&niri)?;
    let wayland = connect(request.wayland_socket, request)?;
    let actual = peer(&wayland)?;
    ensure!(
        identity.pid == actual.pid && identity.uid == actual.uid && identity.gid == actual.gid,
        "Wayland peer does not match selected Niri IPC peer"
    );
    let conn = Connection::from_socket(wayland)?;
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    conn.display().sync(&qh, ());
    let mut state = State::default();
    drive(&mut queue, &mut state, request, |s| s.synced)?;
    state.synced = false;
    conn.display().sync(&qh, ());
    drive(&mut queue, &mut state, request, |s| s.synced)?;
    let matches: Vec<_> = state
        .outputs
        .iter()
        .filter(|(_, (_, name))| name.as_deref() == Some(request.output))
        .collect();
    ensure!(
        matches.len() == 1,
        "named output unavailable or ambiguous; wl_output v4 naming is required"
    );
    let (&global, (output, _)) = matches[0];
    let output = output.clone();
    state.selected = Some(global);
    let source = state
        .source
        .as_ref()
        .context("ext output capture source unavailable")?
        .create_source(&output, &qh, ());
    let session = state
        .copy
        .as_ref()
        .context("ext image copy capture unavailable")?
        .create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &qh,
            (),
        );
    drive(&mut queue, &mut state, request, |s| s.done)?;
    let (width, height) = state.size.context("missing capture dimensions")?;
    let format = [wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888]
        .into_iter()
        .find(|f| state.formats.contains(f))
        .context("no supported capture SHM format")?;
    let (stride, bytes) = allocation(width, height, request.limits)?;
    let file = tempfile::tempfile()?;
    file.set_len(bytes as u64)?;
    // The private file stays alive with the mapping. Only the compositor writes it;
    // read pixels only after the protocol's ready event.
    let mapping = unsafe { memmap2::MmapOptions::new().len(bytes).map(&file)? };
    let pool = state
        .shm
        .as_ref()
        .context("wl_shm unavailable")?
        .create_pool(file.as_fd(), bytes as i32, &qh, ());
    let buffer = pool.create_buffer(0, width as i32, height as i32, stride, format, &qh, ());
    let frame = session.create_frame(&qh, ());
    frame.attach_buffer(&buffer);
    frame.damage_buffer(0, 0, width as i32, height as i32);
    frame.capture();
    drive(&mut queue, &mut state, request, |s| s.ready)?;
    check(request)?;
    let transform = state.transform.context("capture frame omitted transform")?;
    let timestamp = state
        .timestamp
        .context("capture frame omitted presentation time")?;
    let image = decode_checked(
        &mapping,
        width,
        height,
        stride as u32,
        format,
        transform,
        || check(request),
    )?;
    check(request)?;
    let (width, height) = image.dimensions();
    let mut png = Vec::new();
    use image::ImageEncoder;
    let writer = CheckedWriter {
        bytes: &mut png,
        request,
    };
    image::codecs::png::PngEncoder::new(writer).write_image(
        image.as_raw(),
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )?;
    check(request)?;
    ensure!(
        png.len() <= request.limits.max_bytes,
        "encoded capture exceeds byte limit"
    );
    frame.destroy();
    buffer.destroy();
    pool.destroy();
    session.destroy();
    source.destroy();
    // No cleanup roundtrip. Dropping this connection also frees server objects on errors.
    Ok(CapturedOutput {
        png,
        width,
        height,
        buffer_transform: transform,
        presentation_time: timestamp,
        compositor_pid: identity.pid as u32,
    })
}
struct CheckedWriter<'a, 'r> {
    bytes: &'a mut Vec<u8>,
    request: &'a CaptureRequest<'r>,
}
impl std::io::Write for CheckedWriter<'_, '_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        check(self.request).map_err(std::io::Error::other)?;
        if self
            .bytes
            .len()
            .checked_add(buffer.len())
            .is_none_or(|size| size > self.request.limits.max_bytes)
        {
            return Err(std::io::Error::other("encoded capture exceeds byte limit"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
fn decode(
    pixels: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
    transform: wl_output::Transform,
) -> Result<RgbaImage> {
    decode_checked(pixels, width, height, stride, format, transform, || Ok(()))
}
fn decode_checked(
    pixels: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
    transform: wl_output::Transform,
    mut check: impl FnMut() -> Result<()>,
) -> Result<RgbaImage> {
    ensure!(
        matches!(format, wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888),
        "unsupported pixel format"
    );
    ensure!(
        cfg!(target_endian = "little"),
        "capture byte conversion requires little endian"
    );
    ensure!(
        width > 0
            && height > 0
            && u64::from(stride) >= u64::from(width) * 4
            && u64::from(stride) * u64::from(height) <= pixels.len() as u64,
        "invalid capture rows"
    );
    use wl_output::Transform::*;
    let dimensions = match transform {
        Normal | _180 | Flipped | Flipped180 => (width, height),
        _90 | _270 | Flipped90 | Flipped270 => (height, width),
        _ => bail!("unknown capture transform"),
    };
    let mut image = RgbaImage::new(dimensions.0, dimensions.1);
    for y in 0..height {
        check()?;
        for x in 0..width {
            let offset = y as usize * stride as usize + x as usize * 4;
            let alpha = if format == wl_shm::Format::Xrgb8888 {
                255
            } else {
                pixels[offset + 3]
            };
            let channel = |value: u8| {
                if alpha == 0 {
                    0
                } else {
                    ((u32::from(value) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255)
                        as u8
                }
            };
            // Undo Niri's inverse output render transform, using Smithay's
            // buffer-to-logical mapping with discrete pixel bounds.
            let (dx, dy) = match transform {
                Normal => (x, y),
                _90 => (height - 1 - y, x),
                _180 => (width - 1 - x, height - 1 - y),
                _270 => (y, width - 1 - x),
                Flipped => (width - 1 - x, y),
                Flipped90 => (y, x),
                Flipped180 => (x, height - 1 - y),
                Flipped270 => (height - 1 - y, width - 1 - x),
                _ => unreachable!(),
            };
            image.put_pixel(
                dx,
                dy,
                image::Rgba([
                    channel(pixels[offset + 2]),
                    channel(pixels[offset + 1]),
                    channel(pixels[offset]),
                    alpha,
                ]),
            );
        }
    }
    Ok(image)
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
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
            } => match interface.as_str() {
                "wl_output" if version >= 4 => {
                    state
                        .outputs
                        .insert(name, (registry.bind(name, 4, qh, name), None));
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "ext_output_image_capture_source_manager_v1" => {
                    state.source = Some(registry.bind(name, 1, qh, ()))
                }
                "ext_image_copy_capture_manager_v1" => {
                    state.copy = Some(registry.bind(name, 1, qh, ()))
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.remove(&name);
                if state.selected == Some(name) {
                    state.failure = Some("selected output removed".into());
                }
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
        id: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(output) = state.outputs.get_mut(id) {
                output.1 = Some(name);
            }
        }
    }
}
impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            state.synced = true;
        }
    }
}
impl Dispatch<Session, ()> for State {
    fn event(
        state: &mut Self,
        _: &Session,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => state.size = Some((width, height)),
            Event::ShmFormat {
                format: WEnum::Value(format),
            } => state.formats.push(format),
            Event::Done => state.done = true,
            Event::Stopped => state.failure = Some("capture session stopped".into()),
            _ => {}
        }
    }
}
impl Dispatch<Frame, ()> for State {
    fn event(
        state: &mut Self,
        _: &Frame,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::Ready => state.ready = true,
            Event::Failed { reason } => {
                state.failure = Some(format!("capture frame failed: {reason:?}"))
            }
            Event::Transform {
                transform: WEnum::Value(transform),
            } => state.transform = Some(transform),
            Event::PresentationTime {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
            } => {
                if tv_nsec >= 1_000_000_000 {
                    state.failure = Some("invalid capture presentation time".into());
                } else {
                    state.timestamp = Some(Duration::new(
                        (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo),
                        tv_nsec,
                    ));
                }
            }
            _ => {}
        }
    }
}
macro_rules! ignore_events {
    ($($proxy:ty),*) => { $(impl Dispatch<$proxy,()> for State {
        fn event(_:&mut Self,_:&$proxy,_:<$proxy as Proxy>::Event,_:&(),_:&Connection,_:&QueueHandle<Self>) {}
    })* };
}
ignore_events!(
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    SourceManager,
    CopyManager,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1
);

#[cfg(test)]
mod tests {
    use super::*;
    fn request<'a>(cancelled: &'a AtomicBool, path: &'a Path) -> CaptureRequest<'a> {
        CaptureRequest {
            wayland_socket: path,
            niri_socket: path,
            output: "nested",
            deadline: Instant::now() + Duration::from_secs(1),
            cancelled,
            limits: Limits::default(),
        }
    }
    #[test]
    fn packed_rows_and_alpha_are_decoded() {
        let pixels = [
            0, 0, 128, 128, 7, 8, 9, 0, 99, 99, 99, 99, 0, 255, 0, 255, 255, 0, 0, 255, 99, 99, 99,
            99,
        ];
        let image = decode(
            &pixels,
            2,
            2,
            12,
            wl_shm::Format::Argb8888,
            wl_output::Transform::Normal,
        )
        .unwrap();
        assert_eq!(
            image.into_raw(),
            vec![255, 0, 0, 128, 0, 0, 0, 0, 0, 255, 0, 255, 0, 0, 255, 255]
        );
        let image = decode(
            &[1, 2, 3, 0],
            1,
            1,
            4,
            wl_shm::Format::Xrgb8888,
            wl_output::Transform::Normal,
        )
        .unwrap();
        assert_eq!(image.into_raw(), vec![3, 2, 1, 255]);
    }
    #[test]
    fn transforms_have_literal_non_square_expected_pixels() {
        // R-channel labels in buffer order: 1 2 3 / 4 5 6.
        let pixels: Vec<_> = (1..=6).flat_map(|v| [0, 0, v, 255]).collect();
        use wl_output::Transform::*;
        let cases = [
            (Normal, (3, 2), vec![1, 2, 3, 4, 5, 6]),
            (_90, (2, 3), vec![4, 1, 5, 2, 6, 3]),
            (_180, (3, 2), vec![6, 5, 4, 3, 2, 1]),
            (_270, (2, 3), vec![3, 6, 2, 5, 1, 4]),
            (Flipped, (3, 2), vec![3, 2, 1, 6, 5, 4]),
            (Flipped90, (2, 3), vec![1, 4, 2, 5, 3, 6]),
            (Flipped180, (3, 2), vec![4, 5, 6, 1, 2, 3]),
            (Flipped270, (2, 3), vec![6, 3, 5, 2, 4, 1]),
        ];
        for (transform, dimensions, expected) in cases {
            let image = decode(&pixels, 3, 2, 12, wl_shm::Format::Xrgb8888, transform).unwrap();
            assert_eq!(image.dimensions(), dimensions);
            assert_eq!(image.pixels().map(|p| p[0]).collect::<Vec<_>>(), expected);
        }
    }
    #[test]
    fn invalid_buffers_fail_before_allocation() {
        for (w, h) in [(0, 1), (1, 0), (u32::MAX, 1), (32768, 32768)] {
            assert!(allocation(w, h, Limits::default()).is_err());
        }
        assert_eq!(allocation(2, 3, Limits::default()).unwrap(), (8, 24));
        assert!(
            decode(
                &[0; 8],
                2,
                2,
                8,
                wl_shm::Format::Xrgb8888,
                wl_output::Transform::Normal
            )
            .is_err()
        );
        assert!(
            decode(
                &[0; 8],
                2,
                1,
                4,
                wl_shm::Format::Xrgb8888,
                wl_output::Transform::Normal
            )
            .is_err()
        );
        assert!(
            decode(
                &[0; 4],
                1,
                1,
                4,
                wl_shm::Format::Rgb565,
                wl_output::Transform::Normal
            )
            .is_err()
        );
    }
    #[test]
    fn cancellation_and_deadline_do_not_connect() {
        let cancelled = AtomicBool::new(true);
        let mut req = request(&cancelled, Path::new("/missing-capture-socket"));
        assert!(
            capture_output(&req)
                .unwrap_err()
                .to_string()
                .contains("canceled")
        );
        cancelled.store(false, Ordering::Release);
        req.deadline = Instant::now();
        assert!(
            capture_output(&req)
                .unwrap_err()
                .to_string()
                .contains("deadline")
        );
    }
    #[test]
    fn hung_wayland_discovery_is_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("socket");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (niri, _) = listener.accept().unwrap();
            let (wayland, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(150));
            drop((niri, wayland));
        });
        let cancelled = AtomicBool::new(false);
        let mut req = request(&cancelled, &path);
        req.deadline = Instant::now() + Duration::from_millis(70);
        let started = Instant::now();
        assert!(
            capture_output(&req)
                .unwrap_err()
                .to_string()
                .contains("deadline")
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        server.join().unwrap();
    }
    #[test]
    fn cancellation_interrupts_hung_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("socket");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let cancelled = AtomicBool::new(false);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                let (niri, _) = listener.accept().unwrap();
                let (wayland, _) = listener.accept().unwrap();
                done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                drop((niri, wayland));
            });
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(30));
                cancelled.store(true, Ordering::Release);
            });
            let req = request(&cancelled, &path);
            assert!(
                capture_output(&req)
                    .unwrap_err()
                    .to_string()
                    .contains("canceled")
            );
            done_tx.send(()).unwrap();
        });
    }
    #[test]
    fn conversion_and_encoder_observe_cancellation() {
        let pixels = [0; 16];
        assert!(
            decode_checked(
                &pixels,
                2,
                2,
                8,
                wl_shm::Format::Xrgb8888,
                wl_output::Transform::Normal,
                || bail!("canceled")
            )
            .is_err()
        );
        let cancelled = AtomicBool::new(true);
        let req = request(&cancelled, Path::new("unused"));
        let mut bytes = Vec::new();
        use std::io::Write;
        assert!(
            CheckedWriter {
                bytes: &mut bytes,
                request: &req
            }
            .write(&[1, 2, 3])
            .is_err()
        );
        assert!(bytes.is_empty());
    }
    #[test]
    fn encoder_refuses_bytes_before_exceeding_budget() {
        let cancelled = AtomicBool::new(false);
        let mut req = request(&cancelled, Path::new("/unused"));
        req.limits.max_bytes = 2;
        let mut bytes = Vec::new();
        use std::io::Write;
        assert!(
            CheckedWriter {
                bytes: &mut bytes,
                request: &req
            }
            .write(&[1, 2, 3])
            .is_err()
        );
        assert!(bytes.is_empty());
    }
}
