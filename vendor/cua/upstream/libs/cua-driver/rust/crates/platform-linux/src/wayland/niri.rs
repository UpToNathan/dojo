//! Read-only Niri queries. Native IDs never enter foreign-toplevel matching.

use crate::x11::WindowInfo;
use anyhow::{bail, ensure, Context, Result};
use niri_ipc::{LogicalOutput, Reply, Request, Response};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_REPLY_BYTES: u64 = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
struct PeerIdentity {
    pid: u32,
    uid: u32,
    start_ticks: u64,
}

fn process_start(pid: u32) -> Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields = stat.rsplit_once(')').context("invalid process stat")?.1;
    fields
        .split_whitespace()
        .nth(19)
        .context("missing process start")?
        .parse()
        .context("invalid process start")
}

fn peer_identity(stream: &UnixStream) -> Result<PeerIdentity> {
    let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SO_PEERCRED initializes the checked ucred buffer on success.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    ensure!(
        length as usize == std::mem::size_of::<libc::ucred>(),
        "invalid peer credential length"
    );
    // The successful size-checked query initialized every field.
    let credentials = unsafe { credentials.assume_init() };
    let pid = u32::try_from(credentials.pid).context("invalid compositor pid")?;
    ensure!(pid > 0, "missing compositor pid");
    ensure!(
        credentials.uid == unsafe { libc::geteuid() },
        "Niri socket peer belongs to another user"
    );
    Ok(PeerIdentity {
        pid,
        uid: credentials.uid,
        start_ticks: process_start(pid)?,
    })
}

struct Connection {
    stream: BufReader<UnixStream>,
    peer: PeerIdentity,
    deadline: Instant,
}

impl Connection {
    fn connect(path: &Path) -> Result<Self> {
        let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
        socket.set_nonblocking(true)?;
        socket.connect(&socket2::SockAddr::unix(path)?)?;
        socket.set_nonblocking(false)?;
        let stream: UnixStream = socket.into();
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        let peer = peer_identity(&stream)?;
        Ok(Self {
            stream: BufReader::new(stream),
            peer,
            deadline: Instant::now() + TIMEOUT,
        })
    }

    fn query(&mut self, request: &Request) -> Result<Response> {
        ensure!(
            process_start(self.peer.pid)? == self.peer.start_ticks,
            "compositor generation changed"
        );
        let mut bytes = serde_json::to_vec(request)?;
        bytes.push(b'\n');
        self.stream.get_mut().write_all(&bytes)?;
        let max_reply_bytes = if matches!(request, Request::CaptureScene { .. }) {
            48 * 1024 * 1024
        } else {
            MAX_REPLY_BYTES
        };
        let mut reply = Vec::new();
        loop {
            let remaining = self
                .deadline
                .checked_duration_since(Instant::now())
                .context("Niri observation deadline exceeded")?;
            self.stream.get_ref().set_read_timeout(Some(remaining))?;
            let available = self.stream.fill_buf()?;
            if available.is_empty() {
                break;
            }
            let count = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            ensure!(
                reply.len() as u64 + count as u64 <= max_reply_bytes,
                "Niri reply exceeds size limit"
            );
            reply.extend_from_slice(&available[..count]);
            self.stream.consume(count);
            if reply.last() == Some(&b'\n') {
                break;
            }
        }
        ensure!(reply.last() == Some(&b'\n'), "incomplete Niri reply");
        ensure!(
            process_start(self.peer.pid)? == self.peer.start_ticks,
            "compositor generation changed"
        );
        serde_json::from_slice::<Reply>(&reply)?.map_err(anyhow::Error::msg)
    }
}

pub mod capture;

pub fn is_session() -> bool {
    std::env::var_os("NIRI_SOCKET").is_some()
}

pub(crate) fn mutation_refusal() -> Option<cua_driver_core::protocol::ToolResult> {
    is_session().then(|| {
        cua_driver_core::protocol::ToolResult::error(
            "Niri adapter supports read-only queries; native mutation backend is unavailable"
                .to_owned(),
        )
        .with_structured(serde_json::json!({"code":"facility_unavailable","effect":"refused"}))
    })
}

pub(crate) fn refuse_input() -> Result<()> {
    ensure!(!is_session(), "facility_unavailable: Niri adapter supports read-only queries; input backend is unavailable");
    Ok(())
}

fn connection() -> Result<Connection> {
    static BINDING: OnceLock<Mutex<Option<(PathBuf, PeerIdentity)>>> = OnceLock::new();
    let path = PathBuf::from(std::env::var_os("NIRI_SOCKET").context("NIRI_SOCKET is absent")?);
    let connection = Connection::connect(&path)?;
    let mut binding = BINDING
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| anyhow::anyhow!("Niri binding lock poisoned"))?;
    match binding.as_ref() {
        Some((expected_path, expected_peer)) => ensure!(
            expected_path == &path && expected_peer == &connection.peer,
            "Niri compositor binding changed; restart the driver process"
        ),
        None => *binding = Some((path, connection.peer.clone())),
    }
    Ok(connection)
}

#[derive(Clone, Debug)]
pub struct DesktopFrame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl DesktopFrame {
    pub fn to_layout(&self, x: i32, y: i32) -> (i32, i32) {
        (self.x.saturating_add(x), self.y.saturating_add(y))
    }
}

fn outputs(connection: &mut Connection) -> Result<Vec<(String, LogicalOutput)>> {
    let Response::Outputs(outputs) = connection.query(&Request::Outputs)? else {
        bail!("unexpected Niri outputs reply")
    };
    ensure!(outputs.len() <= 32, "too many Niri outputs");
    let mut names = HashSet::new();
    let mut logical = Vec::new();
    for (name, output) in outputs {
        ensure!(
            name == output.name && names.insert(name.clone()),
            "ambiguous Niri output identity"
        );
        if let Some(layout) = output.logical {
            ensure!(
                output.current_mode.is_some(),
                "mapped output has no active mode"
            );
            ensure!(
                layout.width > 0
                    && layout.height > 0
                    && layout.scale.is_finite()
                    && layout.scale > 0.0,
                "invalid Niri output geometry"
            );
            logical.push((name, layout));
        }
    }
    ensure!(!logical.is_empty(), "Niri has no active logical output");
    Ok(logical)
}

fn frame_from_outputs(outputs: Vec<LogicalOutput>) -> Result<DesktopFrame> {
    let left = outputs
        .iter()
        .map(|o| i64::from(o.x))
        .min()
        .context("no output")?;
    let top = outputs
        .iter()
        .map(|o| i64::from(o.y))
        .min()
        .context("no output")?;
    let right = outputs
        .iter()
        .map(|o| i64::from(o.x) + i64::from(o.width))
        .max()
        .unwrap();
    let bottom = outputs
        .iter()
        .map(|o| i64::from(o.y) + i64::from(o.height))
        .max()
        .unwrap();
    Ok(DesktopFrame {
        x: i32::try_from(left)?,
        y: i32::try_from(top)?,
        width: u32::try_from(right - left)?,
        height: u32::try_from(bottom - top)?,
    })
}

pub fn desktop_frame() -> Result<DesktopFrame> {
    frame_from_outputs(
        outputs(&mut connection()?)?
            .into_iter()
            .map(|(_, layout)| layout)
            .collect(),
    )
}

pub fn screen_report() -> Result<(DesktopFrame, f64, serde_json::Value)> {
    let outputs = outputs(&mut connection()?)?;
    let frame = frame_from_outputs(outputs.iter().map(|(_, layout)| *layout).collect())?;
    let scale = outputs
        .iter()
        .map(|(_, layout)| layout.scale)
        .fold(f64::MIN, f64::max);
    let monitors = outputs
        .iter()
        .map(|(name, layout)| {
            serde_json::json!({
                "name": name, "powered": true, "width": layout.width, "height": layout.height,
                "scale": layout.scale, "frame_x": i64::from(layout.x) - i64::from(frame.x),
                "frame_y": i64::from(layout.y) - i64::from(frame.y), "layout_x": layout.x,
                "layout_y": layout.y,
            })
        })
        .collect::<Vec<_>>();
    Ok((frame, scale, serde_json::Value::Array(monitors)))
}

fn exact_integer(value: f64) -> Result<i32> {
    ensure!(
        value.is_finite()
            && value.fract() == 0.0
            && value >= f64::from(i32::MIN)
            && value <= f64::from(i32::MAX),
        "Niri geometry {value} cannot be represented exactly by the driver integer coordinates"
    );
    Ok(value as i32)
}

fn list_from_connection(connection: &mut Connection) -> Result<Vec<WindowInfo>> {
    let Response::Windows(windows) = connection.query(&Request::Windows)? else {
        bail!("unexpected Niri windows reply")
    };
    let Response::Workspaces(workspaces) = connection.query(&Request::Workspaces)? else {
        bail!("unexpected Niri workspaces reply")
    };
    let mut workspace_outputs = HashMap::new();
    for workspace in workspaces {
        ensure!(
            workspace_outputs
                .insert(workspace.id, workspace.output)
                .is_none(),
            "duplicate Niri workspace identity"
        );
    }
    let Response::Outputs(outputs) = connection.query(&Request::Outputs)? else {
        bail!("unexpected Niri outputs reply")
    };
    let mut output_layouts = HashMap::new();
    ensure!(
        outputs.len() <= 32 && windows.len() <= 16384,
        "Niri observation exceeds inventory limit"
    );
    let mut placements = HashMap::new();
    for (name, output) in outputs {
        ensure!(name == output.name, "Niri output name mismatch");
        let Some(logical) = output.logical else {
            continue;
        };
        ensure!(
            output.current_mode.is_some(),
            "mapped output has no active mode"
        );
        output_layouts.insert(name.clone(), logical);
        let Response::WindowPlacements(scene) = connection.query(&Request::WindowPlacements {
            output: name.clone(),
        })?
        else {
            bail!("unexpected Niri placements reply")
        };
        ensure!(
            scene.compositor_pid == connection.peer.pid,
            "Niri placement compositor is not the socket peer"
        );
        ensure!(
            scene.output == name && scene.logical_output == logical,
            "Niri output changed during observation"
        );
        for placement in scene.windows {
            ensure!(
                !placements.contains_key(&placement.id),
                "window spans outputs; exact single-output geometry is unavailable"
            );
            let (x, y, width, height) = placement.rect;
            let rect = (
                exact_integer((x + f64::from(logical.x)).floor())?,
                exact_integer((y + f64::from(logical.y)).floor())?,
                u32::try_from(exact_integer(width.ceil())?)?,
                u32::try_from(exact_integer(height.ceil())?)?,
            );
            ensure!(rect.2 > 0 && rect.3 > 0, "empty Niri window geometry");
            ensure!(
                placement.output_clip.2 > 0.0 && placement.output_clip.3 > 0.0,
                "empty rendered Niri window clip"
            );
            placements.insert(placement.id, (placement.pid, rect));
        }
    }
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for window in windows {
        ensure!(seen.insert(window.id), "duplicate Niri window identity");
        let (is_on_screen, (x, y, width, height)) =
            if let Some((placement_pid, rect)) = placements.remove(&window.id) {
                ensure!(
                    placement_pid == window.pid,
                    "Niri window process changed during observation"
                );
                (true, rect)
            } else {
                let output_name = window
                    .workspace_id
                    .and_then(|id| workspace_outputs.get(&id))
                    .and_then(Option::as_ref)
                    .context("Niri window has no workspace output")?;
                let output = output_layouts
                    .get(output_name)
                    .context("Niri window workspace output is unavailable")?;
                let (tile_x, tile_y) = window
                    .layout
                    .tile_pos_in_workspace_view
                    .context("facility_unavailable: Niri window has no layout position")?;
                let (offset_x, offset_y) = window.layout.window_offset_in_tile;
                let (width, height) = window.layout.window_size;
                ensure!(width > 0 && height > 0, "empty Niri window layout geometry");
                (
                    false,
                    (
                        exact_integer((f64::from(output.x) + tile_x + offset_x).floor())?,
                        exact_integer((f64::from(output.y) + tile_y + offset_y).floor())?,
                        u32::try_from(width)?,
                        u32::try_from(height)?,
                    ),
                )
            };
        let pid = window.pid.map(u32::try_from).transpose()?;
        ensure!(pid != Some(0), "invalid Niri window pid");
        result.push(WindowInfo {
            xid: window.id,
            pid,
            app_name: window.app_id.unwrap_or_default(),
            title: window.title.unwrap_or_default(),
            is_on_screen,
            z_index: None,
            x,
            y,
            width,
            height,
        });
    }
    ensure!(
        placements.is_empty(),
        "Niri window inventory changed during observation"
    );
    Ok(result)
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    list_from_connection(&mut connection()?)
}

pub fn window_geometry(id: u64) -> Result<(i32, i32, u32, u32)> {
    let window = list_windows()?
        .into_iter()
        .find(|w| w.xid == id)
        .context("Niri window identity is absent")?;
    Ok((window.x, window.y, window.width, window.height))
}

#[cfg(test)]
mod tests;

pub(crate) mod native_input;
