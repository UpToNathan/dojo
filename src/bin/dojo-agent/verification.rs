use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use dojo_computer_verification::{
    ObservationSnapshot, PredicateOutcome, Stability, UnknownReason, VerificationStatus,
    VerifyStateInput, VerifyStateOutput, evaluate_predicates,
};
use niri_ipc::{Request, Response, socket::Socket};
use serde_json::json;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(super) fn verify(
    socket_path: Option<&Path>,
    input: &VerifyStateInput,
    cancelled: &AtomicBool,
) -> Result<VerifyStateOutput> {
    let pid: u32 = input.pid.try_into().context("pid exceeds process limits")?;
    let identity = process_identity(pid)?;
    let mut socket = super::desktop_socket(socket_path)?;
    verify_with(input, cancelled, |budget| {
        let socket_budget = budget.min(Duration::from_millis(500));
        socket.set_read_timeout(Some(socket_budget))?;
        socket.set_write_timeout(Some(socket_budget))?;
        observe(
            &mut socket,
            pid,
            input.window_id,
            &identity,
            input
                .expect
                .iter()
                .any(|p| p.window.as_ref().is_some_and(|w| w.bounds.is_some())),
            input.expect.iter().any(|p| p.element.is_some()),
            Instant::now() + budget,
            cancelled,
        )
    })
}

pub(super) fn verify_with(
    input: &VerifyStateInput,
    cancelled: &AtomicBool,
    mut observe: impl FnMut(Duration) -> Result<ObservationSnapshot>,
) -> Result<VerifyStateOutput> {
    let mut stability = Stability::new(input)?;
    let started = Instant::now();
    let timeout = input.timeout_ms.unwrap_or(5_000);
    let deadline = started + Duration::from_millis(if timeout == 0 { 500 } else { timeout });
    let mut outcomes;
    loop {
        ensure!(!cancelled.load(Ordering::Acquire), "verification cancelled");
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            outcomes = unavailable(input, "observation deadline exceeded");
        } else {
            let budget = remaining;
            outcomes = match observe(budget) {
                Ok(snapshot) if Instant::now() <= deadline => {
                    evaluate_predicates(&input.expect, &snapshot)
                }
                Ok(_) => unavailable(input, "observation arrived after deadline"),
                Err(error) => {
                    // A timed-out stream may contain a late response. Do not reuse it.
                    let outcomes = unavailable(input, &format!("{error:#}"));
                    stability.sample(&outcomes);
                    return Ok(stability.finish(outcomes, elapsed(started)));
                }
            };
        }
        if stability.sample(&outcomes) || timeout == 0 || Instant::now() >= deadline {
            break;
        }
        let poll_until = (Instant::now() + POLL_INTERVAL).min(deadline);
        while Instant::now() < poll_until {
            ensure!(!cancelled.load(Ordering::Acquire), "verification cancelled");
            std::thread::sleep(
                poll_until
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            );
        }
        // Do not start a final acquisition with only a fraction of the polling
        // interval left, replacing useful evidence with a deadline artifact.
        if deadline.saturating_duration_since(Instant::now()) < POLL_INTERVAL {
            break;
        }
    }
    ensure!(!cancelled.load(Ordering::Acquire), "verification cancelled");
    Ok(stability.finish(outcomes, elapsed(started)))
}

fn elapsed(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn process_identity(pid: u32) -> Result<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let (_, fields) = stat.rsplit_once(')').context("invalid process identity")?;
    Ok(fields
        .split_whitespace()
        .nth(19)
        .context("missing process identity")?
        .to_owned())
}

fn observe(
    socket: &mut Socket,
    pid: u32,
    window_id: u64,
    identity: &str,
    need_bounds: bool,
    need_elements: bool,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<ObservationSnapshot> {
    ensure!(
        process_identity(pid)? == identity,
        "process generation changed"
    );
    let Response::Windows(windows) = socket.send(Request::Windows)?.map_err(anyhow::Error::msg)?
    else {
        bail!("unexpected windows response");
    };
    let window = windows.into_iter().find(|window| window.id == window_id);
    if let Some(window) = &window {
        ensure!(
            window.pid == Some(i32::try_from(pid)?),
            "window process identity mismatch"
        );
    }
    let mut observed = window
        .as_ref()
        .map(|window| json!({"window_id": window.id, "pid": pid, "title": window.title}));
    if need_bounds {
        if let Some(window) = &window {
            let Response::Workspaces(workspaces) = socket
                .send(Request::Workspaces)?
                .map_err(anyhow::Error::msg)?
            else {
                bail!("unexpected workspaces response");
            };
            if let Some(output) = workspaces
                .iter()
                .find(|w| Some(w.id) == window.workspace_id)
                .and_then(|w| w.output.clone())
            {
                let peer = socket.peer_process_id()?;
                let Response::WindowPlacements(scene) = socket
                    .send(Request::WindowPlacements { output })?
                    .map_err(anyhow::Error::msg)?
                else {
                    bail!("unexpected window placements response");
                };
                ensure!(
                    scene.compositor_pid == peer,
                    "scene compositor identity mismatch"
                );
                if let Some(rendered) = scene
                    .windows
                    .iter()
                    .find(|w| w.id == window_id && w.pid == window.pid && !w.blocked_out)
                {
                    let (x, y, width, height) = rendered.rect;
                    observed.as_mut().unwrap()["bounds"] =
                        json!({"x": x, "y": y, "width": width, "height": height});
                }
            }
        }
    }
    ensure!(
        process_identity(pid)? == identity,
        "process generation changed during observation"
    );
    let accessibility = if need_elements && window.is_some() {
        super::accessibility::observe_bound(socket, window_id, deadline, cancelled)?
    } else {
        None
    };
    if let Some(accessibility) = &accessibility {
        ensure!(
            accessibility.tree.pid == pid,
            "accessibility window process mismatch"
        );
    }
    ensure!(
        process_identity(pid)? == identity,
        "process generation changed during accessibility observation"
    );
    let trusted = accessibility
        .as_ref()
        .is_some_and(|a| a.tree.available && a.tree.window_scoped);
    Ok(ObservationSnapshot {
        source: dojo_computer_verification::ObservationSource::Native,
        // Bounds are output-relative logical coordinates from the actual rendered scene.
        window: observed,
        elements: accessibility
            .as_ref()
            .filter(|_| trusted)
            .map(|a| semantic_elements(&a.tree)),
        element_source_trusted: trusted,
        elements_complete: accessibility.as_ref().is_some_and(|a| a.semantic_complete),
    })
}

fn semantic_elements(tree: &dojo_accessibility::Observation) -> Vec<serde_json::Value> {
    let mut web_nodes = std::collections::HashSet::new();
    tree.nodes
        .iter()
        .filter_map(|node| {
            let normalized = node.role.to_ascii_lowercase().replace(['_', ' '], "");
            let web = normalized.contains("document")
                || normalized.contains("webarea")
                || normalized == "embedded"
                || node
                    .parent
                    .is_some_and(|parent| web_nodes.contains(&parent));
            if web {
                web_nodes.insert(node.index);
            }
            if node.redacted {
                return None;
            }
            Some(
                json!({"element_index": node.index, "parent_index": node.parent,
            "role": node.role, "label": node.name.as_ref().or(node.text.as_ref()),
            "value": node.value, "enabled": node.enabled, "selected": node.selected,
            "in_web_content": web}),
            )
        })
        .collect()
}

fn unavailable(input: &VerifyStateInput, reason: &str) -> Vec<PredicateOutcome> {
    input
        .expect
        .iter()
        .enumerate()
        .map(|(index, _)| PredicateOutcome {
            index: index as u64,
            status: VerificationStatus::Unknown,
            unknown_reason: Some(UnknownReason::ObservationUnavailable),
            observed_json: Some(
                json!({"reason": reason.chars().take(500).collect::<String>()}).to_string(),
            ),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    #[test]
    fn timeout_preserves_the_last_observed_unsatisfied_predicate() {
        let input: VerifyStateInput =
            serde_json::from_value(json!({"pid":1,"window_id":7,"timeout_ms":20,
            "expect":[{"window":{"title_equals":"expected"}}]}))
            .unwrap();
        let result = verify_with(&input, &AtomicBool::new(false), |_| {
            Ok(ObservationSnapshot {
                source: dojo_computer_verification::ObservationSource::Native,
                window: Some(json!({"title":"different"})),
                ..Default::default()
            })
        })
        .unwrap();
        assert_eq!(result.status, VerificationStatus::Unsatisfied);
        assert_eq!(result.samples, 1);
    }

    #[test]
    fn unbound_window_cannot_prove_a_missing_control() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unbound.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let pid = std::process::id();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut requests = Vec::new();
            for line in BufReader::new(stream).lines().take(2) {
                let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
                requests.push(request.clone());
                let reply = if request == "Windows" {
                    json!({"Ok":{"Windows":[{"id":7,"pid":pid,"title":"Fixture","app_id":"fixture","workspace_id":1,
                        "is_focused":false,"is_floating":false,"is_urgent":false,"focus_timestamp":null,
                        "layout":{"pos_in_scrolling_layout":null,"tile_size":[100.0,80.0],"window_size":[100,80],
                        "tile_pos_in_workspace_view":null,"window_offset_in_tile":[0.0,0.0]}}]}})
                } else {
                    json!({"Ok":{"AccessibilityTarget":{"window_id":7,"pid":pid,"generation":0,"endpoint":null}}})
                };
                writeln!(writer, "{reply}").unwrap();
            }
            requests
        });
        let input: VerifyStateInput = serde_json::from_value(json!({"pid":pid,"window_id":7,"timeout_ms":0,
            "expect":[{"element":{"selector":{"role":"button","label_contains":"Save"},"exists":false}}]})).unwrap();
        let result = verify(Some(&path), &input, &AtomicBool::new(false)).unwrap();
        assert_eq!(result.status, VerificationStatus::Unknown);
        assert_eq!(
            server.join().unwrap(),
            vec![
                json!("Windows"),
                json!({"AccessibilityTarget":{"window_id":7}})
            ]
        );
    }

    #[test]
    fn semantic_projection_preserves_ancestry_and_omits_sensitive_values() {
        let node = |index, parent, role: &str, redacted| dojo_accessibility::Node {
            index,
            parent,
            depth: index,
            bus_owner: ":1.1".into(),
            object_path: format!("/node{index}"),
            role: role.into(),
            name: Some("Save".into()),
            description: None,
            text: None,
            value: Some("private".into()),
            redacted,
            enabled: Some(true),
            checked: None,
            selected: Some(false),
            states: vec![],
            actions: vec![],
            interfaces: vec![],
            numeric_range: None,
            selection_parent: None,
            reported_screen_bounds: None,
            bounds_authoritative: false,
        };
        let tree = dojo_accessibility::Observation {
            bus_guid: None,
            pid: 1,
            process_start_time: 1,
            scope: "window",
            actionable: false,
            window_scoped: true,
            available: true,
            complete: false,
            query_filtered: false,
            issues: Default::default(),
            nodes_visited: 4,
            nodes_pending: 0,
            nodes: vec![
                node(0, None, "frame", false),
                node(1, Some(0), "document web", false),
                node(2, Some(1), "button", false),
                node(3, Some(0), "password text", true),
            ],
        };
        let elements = semantic_elements(&tree);
        assert_eq!(elements.len(), 3);
        assert_eq!(elements[2]["parent_index"], 1);
        assert_eq!(elements[2]["in_web_content"], true);
        assert_eq!(elements[0]["in_web_content"], false);
        assert!(!elements.iter().any(|element| element["element_index"] == 3));
    }

    #[test]
    fn native_bounds_verification_does_not_request_pixels() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("geometry.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let pid = std::process::id();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            for line in BufReader::new(stream).lines() {
                let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
                let reply = if request == "Windows" {
                    json!({"Ok":{"Windows":[{"id":7,"pid":pid,"title":"Fixture","app_id":"fixture","workspace_id":1,
                        "is_focused":true,"is_floating":false,"is_urgent":false,"focus_timestamp":null,
                        "layout":{"pos_in_scrolling_layout":[1,1],"tile_size":[104.0,84.0],"window_size":[100,80],
                            "tile_pos_in_workspace_view":[8.0,18.0],"window_offset_in_tile":[2.0,2.0]}}]}})
                } else if request == "Workspaces" {
                    json!({"Ok":{"Workspaces":[{"id":1,"idx":1,"name":null,"output":"DP-1","is_urgent":false,
                        "is_active":true,"is_focused":true,"active_window_id":7}]}})
                } else if request.get("WindowPlacements").is_some() {
                    json!({"Ok":{"WindowPlacements":{"observation_serial":1,"compositor_pid":pid,"output":"DP-1",
                        "width":1500,"height":900,"logical_output":{"x":1920,"y":0,"width":1000,"height":600,"scale":1.5,"transform":"Normal"},
                        "windows":[{"id":7,"pid":pid,"is_floating":false,"rect":[10.0,20.0,100.0,80.0],"output_clip":[10.0,20.0,100.0,80.0],"blocked_out":false}]}}})
                } else {
                    json!({"Err":"pixel_capture_forbidden"})
                };
                writeln!(writer, "{reply}").unwrap();
                if request != "Windows" && request != "Workspaces" {
                    break;
                }
            }
        });
        let input: VerifyStateInput =
            serde_json::from_value(json!({"pid":pid,"window_id":7,"timeout_ms":0,
            "expect":[{"window":{"bounds":{"x":10.0,"y":20.0,"width":100.0,"height":80.0}}}]}))
            .unwrap();
        let result = verify(Some(&path), &input, &AtomicBool::new(false)).unwrap();
        server.join().unwrap();
        assert_eq!(result.status, VerificationStatus::Satisfied, "{result:?}");
    }
}
