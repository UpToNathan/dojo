use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use dojo_accessibility::{Observation, ObserveRequest, Options, WindowRoot};
use niri_ipc::{AccessibilityTarget, Request, Response, socket::Socket};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct NativeObservation {
    pub requested_window_id: u64,
    pub binding_generation: Option<u64>,
    pub semantic_complete: bool,
    pub element_refs: std::collections::BTreeMap<usize, String>,
    #[serde(skip)]
    pub target: AccessibilityTarget,
    #[serde(skip)]
    pub input_stable: bool,
    #[serde(flatten)]
    pub tree: Observation,
}

pub(super) fn observe(
    socket: &mut Socket,
    window_id: u64,
    query: Option<String>,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<NativeObservation> {
    let before = target(socket, window_id, deadline, cancelled)?;
    observe_target(socket, before, query, deadline, cancelled)
}

/// Verification never substitutes an application's other windows for the
/// requested window, even when an application-wide walk would be available.
pub(super) fn observe_bound(
    socket: &mut Socket,
    window_id: u64,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<Option<NativeObservation>> {
    let before = target(socket, window_id, deadline, cancelled)?;
    if before.endpoint.is_none() {
        return Ok(None);
    }
    observe_target(socket, before, None, deadline, cancelled).map(Some)
}

fn observe_target(
    socket: &mut Socket,
    before: AccessibilityTarget,
    query: Option<String>,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<NativeObservation> {
    let window_id = before.window_id;
    let pid: u32 = before
        .pid
        .context("window has no process identity")?
        .try_into()?;
    let start = super::screenshot::process_start_time(pid)?;
    let root = before
        .endpoint
        .as_ref()
        .map(|endpoint| WindowRoot::new(endpoint.bus_name.clone(), endpoint.object_path.clone()))
        .transpose()?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    let walk_deadline = deadline - (remaining / 4).min(std::time::Duration::from_millis(500));
    let request = ObserveRequest {
        pid,
        process_start_time: start,
        deadline: walk_deadline,
        cancelled,
        options: Options {
            max_nodes: 500,
            max_text_chars: 1024,
            query,
            ..Default::default()
        },
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let tree = runtime.block_on(async {
        match &root {
            Some(root) => dojo_accessibility::observe_window(&request, root).await,
            None => dojo_accessibility::observe(&request).await,
        }
    })?;
    ensure!(
        super::screenshot::process_start_time(pid)? == start,
        "accessibility process generation changed"
    );
    let after = target(socket, window_id, deadline, cancelled)?;
    ensure!(
        same_binding(&before, &after),
        "native accessibility target changed during observation"
    );
    Ok(NativeObservation {
        target: before.clone(),
        input_stable: before == after,
        element_refs: Default::default(),
        requested_window_id: window_id,
        binding_generation: before.endpoint.as_ref().map(|_| before.generation),
        semantic_complete: tree.available
            && tree.window_scoped
            && tree.complete
            && !tree.query_filtered
            && !tree.nodes.iter().any(|node| node.redacted),
        tree,
    })
}

pub(super) fn target(
    socket: &mut Socket,
    window_id: u64,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<AccessibilityTarget> {
    ensure!(
        !cancelled.load(Ordering::Acquire),
        "accessibility observation canceled"
    );
    let remaining = deadline.saturating_duration_since(Instant::now());
    ensure!(!remaining.is_zero(), "accessibility deadline exceeded");
    let timeout = remaining.min(std::time::Duration::from_millis(500));
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    let Response::AccessibilityTarget(target) = socket
        .send(Request::AccessibilityTarget { window_id })?
        .map_err(anyhow::Error::msg)?
    else {
        bail!("unexpected accessibility target response");
    };
    ensure!(
        target.window_id == window_id,
        "accessibility target window mismatch"
    );
    Ok(target)
}

fn same_binding(a: &AccessibilityTarget, b: &AccessibilityTarget) -> bool {
    a.window_id == b.window_id
        && a.pid == b.pid
        && a.generation == b.generation
        && a.annotation_generation == b.annotation_generation
        && a.endpoint == b.endpoint
}
