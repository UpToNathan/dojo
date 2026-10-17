//! Bounded read-only AT-SPI observation, with optional compositor-bound roots.
//! Derived from CUA's native AT-SPI traversal, privacy, and query semantics.
pub mod actions;

use anyhow::{Context, Result, bail, ensure};
use atspi_common::{Role, State, StateSet};
use serde::Serialize;
use std::{
    collections::{BTreeSet, HashSet},
    future::Future,
    os::unix::fs::MetadataExt,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use zbus::{Connection, Proxy, proxy::CacheProperties, zvariant::OwnedObjectPath};

pub struct ObserveRequest<'a> {
    pub pid: u32,
    pub process_start_time: u64,
    pub deadline: Instant,
    pub cancelled: &'a AtomicBool,
    pub options: Options,
}

/// An accessibility endpoint announced on the selected native Wayland surface.
/// The caller must obtain this from the compositor, never from model arguments.
/// This is read evidence, not an action capability.
pub struct WindowRoot {
    owner: String,
    path: String,
}
impl WindowRoot {
    pub fn new(owner: String, path: String) -> Result<Self> {
        ensure!(
            owner.len() <= 255 && path.len() <= 4096,
            "invalid accessibility endpoint"
        );
        zbus::names::UniqueName::try_from(owner.as_str())
            .context("accessibility endpoint requires a unique bus owner")?;
        OwnedObjectPath::try_from(path.as_str()).context("invalid accessibility object path")?;
        ensure!(
            path != "/org/a11y/atspi/null",
            "null accessibility endpoint"
        );
        Ok(Self { owner, path })
    }
}
#[derive(Clone)]
pub struct Options {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_children: usize,
    pub max_text_chars: usize,
    pub query: Option<String>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            max_nodes: 5000,
            max_depth: 64,
            max_children: 2048,
            max_text_chars: 4096,
            query: None,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Observation {
    pub bus_guid: Option<String>,
    pub pid: u32,
    pub process_start_time: u64,
    pub scope: &'static str,
    pub actionable: bool,
    pub window_scoped: bool,
    pub available: bool,
    pub complete: bool,
    pub query_filtered: bool,
    pub issues: BTreeSet<String>,
    pub nodes_visited: usize,
    pub nodes_pending: usize,
    pub nodes: Vec<Node>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Node {
    /// Local display index. This is not an action token.
    pub index: usize,
    pub parent: Option<usize>,
    pub depth: usize,
    pub bus_owner: String,
    pub object_path: String,
    pub role: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub text: Option<String>,
    pub value: Option<String>,
    pub redacted: bool,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub states: Vec<String>,
    pub actions: Vec<String>,
    pub interfaces: Vec<String>,
    pub numeric_range: Option<(f64, f64)>,
    /// Read evidence for this node's direct Selection parent, not an action token.
    pub selection_parent: Option<SelectionParent>,
    /// Application-reported screen extents. No native window mapping is claimed.
    pub reported_screen_bounds: Option<(i32, i32, i32, i32)>,
    pub bounds_authoritative: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SelectionParent {
    pub bus_owner: String,
    pub object_path: String,
    pub index: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Object {
    owner: String,
    path: String,
}
#[derive(Debug)]
enum ReadError {
    Canceled,
    Deadline,
    Timeout,
    Unavailable,
}
struct Budget<'a> {
    request: &'a ObserveRequest<'a>,
}
impl Budget<'_> {
    async fn read<T, E>(
        &self,
        future: impl Future<Output = std::result::Result<T, E>>,
    ) -> std::result::Result<T, ReadError> {
        if self.request.cancelled.load(Ordering::Acquire) {
            return Err(ReadError::Canceled);
        }
        if Instant::now() >= self.request.deadline {
            return Err(ReadError::Deadline);
        }
        let call_deadline = tokio::time::Instant::from_std(
            self.request
                .deadline
                .min(Instant::now() + Duration::from_millis(500)),
        );
        tokio::pin!(future);
        loop {
            tokio::select! {
                biased;
                _=tokio::time::sleep_until(call_deadline)=>return Err(if Instant::now()>=self.request.deadline {ReadError::Deadline} else {ReadError::Timeout}),
                _=tokio::time::sleep(Duration::from_millis(20))=> {
                    if self.request.cancelled.load(Ordering::Acquire) {return Err(ReadError::Canceled);}
                },
                result=&mut future=>return result.map_err(|_|ReadError::Unavailable),
            }
        }
    }
    async fn proxy<'a>(
        &self,
        conn: &'a Connection,
        object: &'a Object,
        interface: &'a str,
    ) -> std::result::Result<Proxy<'a>, ReadError> {
        let builder = zbus::proxy::Builder::<Proxy<'a>>::new(conn)
            .cache_properties(CacheProperties::No)
            .destination(object.owner.as_str())
            .map_err(|_| ReadError::Unavailable)?
            .path(object.path.as_str())
            .map_err(|_| ReadError::Unavailable)?
            .interface(interface)
            .map_err(|_| ReadError::Unavailable)?;
        self.read(builder.build()).await
    }
}
fn process_identity(pid: u32) -> Result<(u64, u32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .context("process identity unavailable")?;
    let (_, fields) = stat.rsplit_once(')').context("invalid process stat")?;
    let start = fields
        .split_whitespace()
        .nth(19)
        .context("process start time unavailable")?
        .parse()?;
    let uid = std::fs::metadata(format!("/proc/{pid}"))
        .context("process owner unavailable")?
        .uid();
    Ok((start, uid))
}
fn check_process(request: &ObserveRequest<'_>, uid: u32) -> Result<()> {
    ensure!(
        !request.cancelled.load(Ordering::Acquire),
        "accessibility observation canceled"
    );
    ensure!(
        process_identity(request.pid)? == (request.process_start_time, uid),
        "process identity changed"
    );
    Ok(())
}
/// Observe every proven AT-SPI application root owned by exactly this process.
/// No input methods, event subscriptions, cache globals, snapshots, or action refs.
pub async fn observe_process(
    pid: u32,
    process_start_time: u64,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<Observation> {
    observe(&ObserveRequest {
        pid,
        process_start_time,
        deadline,
        cancelled,
        options: Options::default(),
    })
    .await
}
pub async fn observe(request: &ObserveRequest<'_>) -> Result<Observation> {
    observe_selected(request, None).await
}

/// Walk only the endpoint announced on the selected native surface, proving its
/// bus owner belongs to the exact process generation supplied by the caller.
pub async fn observe_window(
    request: &ObserveRequest<'_>,
    root: &WindowRoot,
) -> Result<Observation> {
    observe_selected(request, Some(root)).await
}

async fn observe_selected(
    request: &ObserveRequest<'_>,
    root: Option<&WindowRoot>,
) -> Result<Observation> {
    ensure!(
        request.pid > 0 && request.process_start_time > 0,
        "invalid process identity"
    );
    ensure!(
        request.options.max_nodes > 0
            && request.options.max_nodes <= 5000
            && request.options.max_depth <= 64
            && request.options.max_children > 0
            && request.options.max_children <= 2048
            && request.options.max_text_chars > 0
            && request.options.max_text_chars <= 4096
            && request
                .options
                .query
                .as_ref()
                .is_none_or(|q| q.len() <= 4096),
        "invalid accessibility budgets"
    );
    ensure!(
        !request.cancelled.load(Ordering::Acquire),
        "accessibility observation canceled"
    );
    ensure!(
        Instant::now() < request.deadline,
        "accessibility deadline exceeded"
    );
    let (start, uid) = process_identity(request.pid)?;
    ensure!(
        start == request.process_start_time,
        "process identity changed"
    );
    let budget = Budget { request };
    let mut observation = Observation {
        bus_guid: None,
        pid: request.pid,
        process_start_time: start,
        scope: if root.is_some() {
            "window"
        } else {
            "application"
        },
        actionable: false,
        window_scoped: false,
        available: false,
        complete: false,
        query_filtered: request.options.query.is_some(),
        issues: BTreeSet::new(),
        nodes_visited: 0,
        nodes_pending: 0,
        nodes: Vec::new(),
    };
    let session = match budget.read(Connection::session()).await {
        Ok(c) => c,
        Err(e) => {
            issue(&mut observation, e)?;
            return Ok(observation);
        }
    };
    match resolve_owner(&budget, &session, "org.a11y.Bus").await {
        Ok((_, _, owner_uid)) if owner_uid == uid => {}
        Ok(_) => {
            observation.issues.insert("bus_owner_mismatch".into());
            return Ok(observation);
        }
        Err(e) => {
            issue(&mut observation, e)?;
            return Ok(observation);
        }
    }
    let bus_object = Object {
        owner: "org.a11y.Bus".into(),
        path: "/org/a11y/bus".into(),
    };
    let address: String = match async {
        let proxy = budget.proxy(&session, &bus_object, "org.a11y.Bus").await?;
        budget.read(proxy.call("GetAddress", &())).await
    }
    .await
    {
        Ok(a) => a,
        Err(e) => {
            issue(&mut observation, e)?;
            return Ok(observation);
        }
    };
    ensure!(
        address.starts_with("unix:") && !address.contains(';'),
        "accessibility requires one local Unix bus address"
    );
    let builder = zbus::connection::Builder::address(address.as_str())
        .map_err(|_| anyhow::anyhow!("invalid accessibility bus address"))?;
    let conn = match budget.read(builder.build()).await {
        Ok(c) => c,
        Err(e) => {
            issue(&mut observation, e)?;
            return Ok(observation);
        }
    };
    let result = observe_selected_on_connection(request, &conn, uid, &mut observation, root).await;
    // Connection lifetime is local to this observation. Dropping proxies and
    // connections retires their internal reader; close does not wait for app RPC.
    let _ = budget.read(conn.close()).await;
    let _ = budget.read(session.close()).await;
    result?;
    check_process(request, uid)?;
    observation.complete = observation.available && observation.issues.is_empty();
    project(&mut observation.nodes, request.options.query.as_deref());
    Ok(observation)
}
fn issue(observation: &mut Observation, error: ReadError) -> Result<()> {
    if matches!(error, ReadError::Canceled) {
        bail!("accessibility observation canceled");
    }
    observation.issues.insert(
        match error {
            ReadError::Deadline => "deadline",
            ReadError::Timeout => "app_unresponsive",
            ReadError::Unavailable => "unavailable",
            ReadError::Canceled => unreachable!(),
        }
        .into(),
    );
    Ok(())
}
async fn resolve_owner(
    budget: &Budget<'_>,
    conn: &Connection,
    name: &str,
) -> std::result::Result<(String, u32, u32), ReadError> {
    let object = Object {
        owner: "org.freedesktop.DBus".into(),
        path: "/org/freedesktop/DBus".into(),
    };
    let proxy = budget.proxy(conn, &object, "org.freedesktop.DBus").await?;
    let owner: String = budget.read(proxy.call("GetNameOwner", &(name,))).await?;
    if !owner.starts_with(':') {
        return Err(ReadError::Unavailable);
    }
    let pid: u32 = budget
        .read(proxy.call("GetConnectionUnixProcessID", &(&owner,)))
        .await?;
    let uid: u32 = budget
        .read(proxy.call("GetConnectionUnixUser", &(&owner,)))
        .await?;
    Ok((owner, pid, uid))
}
async fn selection_reference_matches(
    budget: &Budget<'_>,
    conn: &Connection,
    actual: (String, OwnedObjectPath),
    expected: &Object,
    uid: u32,
    observation: &mut Observation,
) -> Result<bool> {
    if actual.1.as_str() != expected.path {
        return Ok(false);
    }
    if actual.0 == expected.owner {
        return Ok(true);
    }
    if actual.0.starts_with(':') {
        return Ok(false);
    }
    match resolve_owner(budget, conn, &actual.0).await {
        Ok((owner, pid, owner_uid)) => {
            Ok(owner == expected.owner && pid == budget.request.pid && owner_uid == uid)
        }
        Err(error) => {
            issue(observation, error)?;
            Ok(false)
        }
    }
}

async fn selection_parent_evidence(
    budget: &Budget<'_>,
    conn: &Connection,
    child_proxy: &Proxy<'_>,
    child: &Object,
    parent: &Object,
    uid: u32,
    observation: &mut Observation,
) -> Result<Option<SelectionParent>> {
    let index: i32 = match budget.read(child_proxy.call("GetIndexInParent", &())).await {
        Ok(index) => index,
        Err(error) => {
            issue(observation, error)?;
            return Ok(None);
        }
    };
    if !(0..2048).contains(&index) || index as usize >= budget.request.options.max_children {
        observation
            .issues
            .insert("invalid_selection_child_index".into());
        return Ok(None);
    }
    let actual_parent = match budget.read(child_proxy.get_property("Parent")).await {
        Ok(parent) => parent,
        Err(error) => {
            issue(observation, error)?;
            return Ok(None);
        }
    };
    if !selection_reference_matches(budget, conn, actual_parent, parent, uid, observation).await? {
        observation
            .issues
            .insert("selection_relationship_changed".into());
        return Ok(None);
    }
    let parent_proxy = match budget
        .proxy(conn, parent, "org.a11y.atspi.Accessible")
        .await
    {
        Ok(proxy) => proxy,
        Err(error) => {
            issue(observation, error)?;
            return Ok(None);
        }
    };
    let count: i32 = match budget.read(parent_proxy.get_property("ChildCount")).await {
        Ok(count) => count,
        Err(error) => {
            issue(observation, error)?;
            return Ok(None);
        }
    };
    if index >= count {
        observation
            .issues
            .insert("selection_relationship_changed".into());
        return Ok(None);
    }
    let actual_child = match budget
        .read(parent_proxy.call("GetChildAtIndex", &(index,)))
        .await
    {
        Ok(child) => child,
        Err(error) => {
            issue(observation, error)?;
            return Ok(None);
        }
    };
    if !selection_reference_matches(budget, conn, actual_child, child, uid, observation).await? {
        observation
            .issues
            .insert("selection_relationship_changed".into());
        return Ok(None);
    }
    Ok(Some(SelectionParent {
        bus_owner: parent.owner.clone(),
        object_path: parent.path.clone(),
        index,
    }))
}

async fn child_objects(
    budget: &Budget<'_>,
    proxy: &Proxy<'_>,
    count: usize,
) -> std::result::Result<Vec<(String, OwnedObjectPath)>, ReadError> {
    // Fetch one child at a time, avoiding unbounded GetChildren allocation and
    // huge spreadsheet-container expansion from the upstream traversal.
    let mut children = Vec::with_capacity(count);
    for index in 0..count {
        children.push(
            budget
                .read(proxy.call("GetChildAtIndex", &(index as i32,)))
                .await?,
        );
    }
    Ok(children)
}
#[cfg(test)]
async fn observe_on_connection(
    request: &ObserveRequest<'_>,
    conn: &Connection,
    uid: u32,
    observation: &mut Observation,
) -> Result<()> {
    observe_selected_on_connection(request, conn, uid, observation, None).await
}

async fn observe_selected_on_connection(
    request: &ObserveRequest<'_>,
    conn: &Connection,
    uid: u32,
    observation: &mut Observation,
    selected: Option<&WindowRoot>,
) -> Result<()> {
    observation.bus_guid = Some(conn.server_guid().as_str().to_owned());
    let budget = Budget { request };
    let mut roots = Vec::new();
    let mut owners = HashSet::new();
    if let Some(root) = selected {
        match resolve_owner(&budget, conn, &root.owner).await {
            Ok((owner, pid, owner_uid))
                if owner == root.owner && pid == request.pid && owner_uid == uid =>
            {
                owners.insert(owner.clone());
                roots.push(Object {
                    owner,
                    path: root.path.clone(),
                });
                observation.scope = "window";
                observation.window_scoped = true;
            }
            Ok(_) => {
                observation.issues.insert("window_owner_mismatch".into());
                return Ok(());
            }
            Err(e) => {
                issue(observation, e)?;
                return Ok(());
            }
        }
    } else {
        let (registry_owner, _, registry_uid) =
            match resolve_owner(&budget, conn, "org.a11y.atspi.Registry").await {
                Ok(r) => r,
                Err(e) => {
                    issue(observation, e)?;
                    return Ok(());
                }
            };
        if registry_uid != uid {
            observation.issues.insert("registry_owner_mismatch".into());
            return Ok(());
        }
        let registry = Object {
            owner: registry_owner,
            path: "/org/a11y/atspi/accessible/root".into(),
        };
        let accessible = match budget
            .proxy(conn, &registry, "org.a11y.atspi.Accessible")
            .await
        {
            Ok(p) => p,
            Err(e) => {
                issue(observation, e)?;
                return Ok(());
            }
        };
        let count: i32 = match budget.read(accessible.get_property("ChildCount")).await {
            Ok(n) => n,
            Err(e) => {
                issue(observation, e)?;
                return Ok(());
            }
        };
        if count < 0 || count as usize > request.options.max_children {
            observation.issues.insert("registry_child_budget".into());
            return Ok(());
        }
        let apps = match child_objects(&budget, &accessible, count as usize).await {
            Ok(a) => a,
            Err(e) => {
                issue(observation, e)?;
                return Ok(());
            }
        };
        for (name, path) in apps {
            match resolve_owner(&budget, conn, &name).await {
                Ok((owner, pid, owner_uid)) if pid == request.pid && owner_uid == uid => {
                    let object = Object {
                        owner,
                        path: path.to_string(),
                    };
                    if !roots.contains(&object) {
                        owners.insert(object.owner.clone());
                        roots.push(object);
                    }
                }
                Ok(_) => {}
                Err(e) => issue(observation, e)?,
            }
        }
    }
    if roots.is_empty() {
        observation.issues.insert("application_unavailable".into());
        return Ok(());
    }
    observation.available = true;
    let mut pending: Vec<_> = roots
        .into_iter()
        .rev()
        .map(|root| (root, 0, None))
        .collect();
    let mut seen = HashSet::new();
    let mut walk_budget =
        dojo_cua_primitives::walk_budget::WalkBudget::nodes_only(request.options.max_nodes);
    while let Some((object, depth, parent)) = pending.pop() {
        if !walk_budget.admit() {
            observation.issues.insert("node_budget".into());
            pending.push((object, depth, parent));
            break;
        }
        if Instant::now() >= request.deadline {
            observation.issues.insert("deadline".into());
            pending.push((object, depth, parent));
            break;
        }
        check_process(request, uid)?;
        if !seen.insert(object.clone()) {
            observation.issues.insert("repeated_object".into());
            continue;
        }
        let proxy = match budget
            .proxy(conn, &object, "org.a11y.atspi.Accessible")
            .await
        {
            Ok(p) => p,
            Err(e) => {
                issue(observation, e)?;
                continue;
            }
        };
        let index = observation.nodes_visited;
        observation.nodes_visited = walk_budget.outcome().nodes_visited;
        let role: u32 = match budget.read(proxy.call("GetRole", &())).await {
            Ok(r) => r,
            Err(e) => {
                issue(observation, e)?;
                continue;
            }
        };
        let role = Role::try_from(role).ok();
        let redacted = matches!(
            role,
            None | Some(Role::Invalid | Role::Unknown | Role::PasswordText)
        );
        if role.is_none() || matches!(role, Some(Role::Invalid | Role::Unknown)) {
            observation.issues.insert("unknown_role".into());
        }
        let states: Option<StateSet> = match budget.read(proxy.call("GetState", &())).await {
            Ok(s) => Some(s),
            Err(e) => {
                issue(observation, e)?;
                None
            }
        };
        let interfaces: Vec<String> = match budget.read(proxy.call("GetInterfaces", &())).await {
            Ok(s) => s,
            Err(e) => {
                issue(observation, e)?;
                Vec::new()
            }
        };
        let mut name = if redacted {
            None
        } else {
            read_string(
                &budget,
                &proxy,
                "Name",
                request.options.max_text_chars,
                observation,
            )
            .await?
        };
        let mut description = if redacted {
            None
        } else {
            read_string(
                &budget,
                &proxy,
                "Description",
                request.options.max_text_chars,
                observation,
            )
            .await?
        };
        let has_value = interfaces.iter().any(|i| i == "org.a11y.atspi.Value");
        let has_editable = interfaces
            .iter()
            .any(|i| i == "org.a11y.atspi.EditableText");
        if !redacted
            && name.as_deref().is_none_or(|n| n.trim().is_empty())
            && (has_value || has_editable)
        {
            name = labelled_by_name(&budget, conn, &proxy, request.pid, uid, observation).await?;
            if name.is_none() {
                name = preceding_sibling_label(&observation.nodes, parent, role.map(|r| r.name()));
            }
        }
        let sensitive = redacted || sensitive_label(name.as_deref(), description.as_deref());
        if sensitive {
            name = None;
            description = None;
        }
        let mut node = Node {
            index,
            parent,
            depth,
            bus_owner: object.owner.clone(),
            object_path: object.path.clone(),
            role: role.map_or("unknown".into(), |r| r.name().into()),
            name,
            description,
            text: None,
            value: None,
            redacted: sensitive,
            enabled: states.map(|s| s.contains(State::Enabled) || s.contains(State::Sensitive)),
            checked: states.map(|s| s.contains(State::Checked)),
            selected: states.map(|s| s.contains(State::Selected)),
            states: states.map_or(Vec::new(), |s| s.iter().map(|v| v.to_string()).collect()),
            actions: Vec::new(),
            interfaces: interfaces.clone(),
            numeric_range: None,
            selection_parent: None,
            reported_screen_bounds: None,
            bounds_authoritative: false,
        };
        if !sensitive && interfaces.iter().any(|i| i == "org.a11y.atspi.Text") {
            match budget.proxy(conn, &object, "org.a11y.atspi.Text").await {
                Ok(text) => {
                    let count: i32 = match budget.read(text.get_property("CharacterCount")).await {
                        Ok(count) => count,
                        Err(e) => {
                            issue(observation, e)?;
                            0
                        }
                    };
                    if count < 0 {
                        observation.issues.insert("invalid_text_count".into());
                    }
                    if count as usize > request.options.max_text_chars {
                        observation.issues.insert("text_budget".into());
                    }
                    match budget
                        .read::<String, _>(text.call(
                            "GetText",
                            &(
                                0i32,
                                (count.max(0) as usize).min(request.options.max_text_chars) as i32,
                            ),
                        ))
                        .await
                    {
                        Ok(value) => {
                            node.text = Some(sanitize(&value, request.options.max_text_chars))
                        }
                        Err(e) => issue(observation, e)?,
                    }
                }
                Err(e) => issue(observation, e)?,
            }
        }
        if !sensitive && has_value {
            match budget.proxy(conn, &object, "org.a11y.atspi.Value").await {
                Ok(value_proxy) => {
                    match budget
                        .read::<f64, _>(value_proxy.get_property("CurrentValue"))
                        .await
                    {
                        Ok(value) if value.is_finite() => node.value = Some(format!("{value:?}")),
                        Ok(_) => {
                            observation.issues.insert("invalid_numeric_value".into());
                        }
                        Err(e) => issue(observation, e)?,
                    }
                    let minimum = budget
                        .read::<f64, _>(value_proxy.get_property("MinimumValue"))
                        .await;
                    let maximum = budget
                        .read::<f64, _>(value_proxy.get_property("MaximumValue"))
                        .await;
                    match (minimum, maximum) {
                        (Ok(min), Ok(max)) if min.is_finite() && max.is_finite() && min <= max => {
                            node.numeric_range = Some((min, max));
                        }
                        _ => {
                            observation
                                .issues
                                .insert("numeric_range_unavailable".into());
                        }
                    }
                }
                Err(e) => issue(observation, e)?,
            }
        }
        node.value = editable_text_value(
            !sensitive && has_editable,
            has_value,
            node.value,
            node.text.as_deref().unwrap_or(""),
        );
        if interfaces.iter().any(|i| i == "org.a11y.atspi.Component") {
            match budget
                .proxy(conn, &object, "org.a11y.atspi.Component")
                .await
            {
                Ok(component) => match budget.read(component.call("GetExtents", &(0u32,))).await {
                    Ok(bounds) => node.reported_screen_bounds = plausible_bounds(bounds),
                    Err(e) => issue(observation, e)?,
                },
                Err(e) => issue(observation, e)?,
            }
        }
        if interfaces.iter().any(|i| i == "org.a11y.atspi.Action") {
            match budget.proxy(conn, &object, "org.a11y.atspi.Action").await {
                Ok(actions) => {
                    let count: i32 = match budget.read(actions.get_property("NActions")).await {
                        Ok(c) => c,
                        Err(e) => {
                            issue(observation, e)?;
                            0
                        }
                    };
                    if !(0..=32).contains(&count) {
                        observation.issues.insert("action_budget".into());
                    } else {
                        for action in 0..count {
                            match budget
                                .read::<String, _>(actions.call("GetName", &(action,)))
                                .await
                            {
                                Ok(name) => node.actions.push(sanitize(&name, 128)),
                                Err(e) => {
                                    issue(observation, e)?;
                                    node.actions.push(String::new());
                                }
                            }
                        }
                    }
                }
                Err(e) => issue(observation, e)?,
            }
        }
        if !sensitive && actions::is_direct_selection_role(&node.role) {
            let candidate = parent
                .and_then(|index| {
                    observation
                        .nodes
                        .iter()
                        .find(|parent| parent.index == index)
                })
                .filter(|parent| {
                    !parent.redacted
                        && parent.bus_owner == node.bus_owner
                        && parent.enabled == Some(true)
                        && !parent.states.iter().any(|state| state == "defunct")
                        && parent
                            .interfaces
                            .iter()
                            .any(|interface| interface == "org.a11y.atspi.Selection")
                })
                .map(|parent| Object {
                    owner: parent.bus_owner.clone(),
                    path: parent.object_path.clone(),
                });
            if let Some(parent) = candidate {
                node.selection_parent = selection_parent_evidence(
                    &budget,
                    conn,
                    &proxy,
                    &object,
                    &parent,
                    uid,
                    observation,
                )
                .await?;
            }
        }
        observation.nodes.push(node);
        let count: i32 = match budget.read(proxy.get_property("ChildCount")).await {
            Ok(c) => c,
            Err(e) => {
                issue(observation, e)?;
                continue;
            }
        };
        if count < 0 {
            observation.issues.insert("invalid_child_count".into());
            continue;
        }
        if count == 0 {
            continue;
        }
        if sensitive {
            observation.issues.insert("privacy_filtered_subtree".into());
            continue;
        }
        if depth >= request.options.max_depth {
            observation.issues.insert("depth_budget".into());
            continue;
        }
        if count as usize > request.options.max_children {
            observation.issues.insert("huge_container".into());
            continue;
        }
        let remaining = request
            .options
            .max_nodes
            .saturating_sub(observation.nodes_visited + pending.len());
        if count as usize > remaining {
            observation.issues.insert("node_budget".into());
        }
        let children = match child_objects(&budget, &proxy, (count as usize).min(remaining)).await {
            Ok(c) => c,
            Err(e) => {
                issue(observation, e)?;
                continue;
            }
        };
        for (name, path) in children.into_iter().rev() {
            if name.starts_with(':') && owners.contains(&name) {
                pending.push((
                    Object {
                        owner: name,
                        path: path.to_string(),
                    },
                    depth + 1,
                    Some(index),
                ));
                continue;
            }
            match resolve_owner(&budget, conn, &name).await {
                Ok((owner, pid, owner_uid)) if pid == request.pid && owner_uid == uid => {
                    owners.insert(owner.clone());
                    pending.push((
                        Object {
                            owner,
                            path: path.to_string(),
                        },
                        depth + 1,
                        Some(index),
                    ));
                }
                Ok(_) => {
                    observation.issues.insert("cross_process_subtree".into());
                }
                Err(e) => issue(observation, e)?,
            }
        }
    }
    observation.nodes_pending = pending.len();
    // Re-prove every observed owner after traversal; never accept PID reuse or
    // a changed D-Bus owner under a well-known alias.
    for owner in owners {
        match resolve_owner(&budget, conn, &owner).await {
            Ok((_, pid, owner_uid)) if pid == request.pid && owner_uid == uid => {}
            Ok(_) => bail!("accessibility application identity changed"),
            Err(e) => issue(observation, e)?,
        }
    }
    Ok(())
}
// CUA native.rs editable_text_value, retained with its value-interface precedence.
fn editable_text_value(
    has_editable: bool,
    has_value: bool,
    value: Option<String>,
    text_content: &str,
) -> Option<String> {
    if value.is_some() || has_value || !has_editable || text_content.is_empty() {
        return value;
    }
    Some(text_content.to_owned())
}
async fn labelled_by_name(
    budget: &Budget<'_>,
    conn: &Connection,
    accessible: &Proxy<'_>,
    pid: u32,
    uid: u32,
    observation: &mut Observation,
) -> Result<Option<String>> {
    let relations: Vec<(u32, Vec<(String, OwnedObjectPath)>)> =
        match budget.read(accessible.call("GetRelationSet", &())).await {
            Ok(r) => r,
            Err(e) => {
                issue(observation, e)?;
                return Ok(None);
            }
        };
    if relations.len() > 16 {
        observation.issues.insert("relation_budget".into());
    }
    for (kind, targets) in relations.into_iter().take(16) {
        if kind != atspi_common::RelationType::LabelledBy as u32 {
            continue;
        }
        if targets.len() > 16 {
            observation.issues.insert("relation_budget".into());
        }
        for (name, path) in targets.into_iter().take(16) {
            let owner = match resolve_owner(budget, conn, &name).await {
                Ok((owner, owner_pid, owner_uid)) if owner_pid == pid && owner_uid == uid => owner,
                Ok(_) => {
                    observation.issues.insert("cross_process_label".into());
                    continue;
                }
                Err(e) => {
                    issue(observation, e)?;
                    continue;
                }
            };
            let object = Object {
                owner,
                path: path.to_string(),
            };
            let proxy = match budget
                .proxy(conn, &object, "org.a11y.atspi.Accessible")
                .await
            {
                Ok(p) => p,
                Err(e) => {
                    issue(observation, e)?;
                    continue;
                }
            };
            let role: u32 = match budget.read(proxy.call("GetRole", &())).await {
                Ok(r) => r,
                Err(e) => {
                    issue(observation, e)?;
                    continue;
                }
            };
            if matches!(
                Role::try_from(role).ok(),
                None | Some(Role::Invalid | Role::Unknown | Role::PasswordText)
            ) {
                continue;
            }
            if let Some(label) = read_string(
                budget,
                &proxy,
                "Name",
                budget.request.options.max_text_chars,
                observation,
            )
            .await?
            {
                if !label.trim().is_empty() {
                    return Ok(Some(label.trim().to_owned()));
                }
            }
        }
    }
    Ok(None)
}
fn preceding_sibling_label(
    nodes: &[Node],
    parent: Option<usize>,
    role: Option<&str>,
) -> Option<String> {
    parent?;
    for node in nodes.iter().rev() {
        if node.parent != parent {
            continue;
        }
        if Some(node.role.as_str()) == role {
            break;
        }
        if node.role == "label" && !node.redacted {
            return node
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned);
        }
    }
    None
}
async fn read_string(
    budget: &Budget<'_>,
    proxy: &Proxy<'_>,
    property: &str,
    limit: usize,
    observation: &mut Observation,
) -> Result<Option<String>> {
    match budget.read::<String, _>(proxy.get_property(property)).await {
        Ok(value) => {
            if value.chars().count() > limit {
                observation.issues.insert("text_budget".into());
            }
            Ok(Some(sanitize(&value, limit)))
        }
        Err(e) => {
            issue(observation, e)?;
            Ok(None)
        }
    }
}
fn sanitize(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(limit)
        .collect()
}
fn sensitive_label(name: Option<&str>, description: Option<&str>) -> bool {
    let label = format!("{} {}", name.unwrap_or(""), description.unwrap_or("")).to_lowercase();
    [
        "password",
        "secret",
        "api key",
        "access token",
        "private key",
        "credential",
    ]
    .iter()
    .any(|needle| label.contains(needle))
}
fn plausible_bounds(bounds: (i32, i32, i32, i32)) -> Option<(i32, i32, i32, i32)> {
    let (x, y, w, h) = bounds;
    (x >= -16384 && y >= -16384 && w > 1 && h > 1).then_some(bounds)
}
fn project(nodes: &mut Vec<Node>, query: Option<&str>) {
    let Some(query) = query else {
        return;
    };
    let needle = query.to_lowercase();
    let mut selected = HashSet::new();
    for node in nodes.iter() {
        if format!(
            "{} {} {} {} {}",
            node.role,
            node.name.as_deref().unwrap_or(""),
            node.description.as_deref().unwrap_or(""),
            node.text.as_deref().unwrap_or(""),
            node.value.as_deref().unwrap_or("")
        )
        .to_lowercase()
        .contains(&needle)
        {
            selected.insert(node.index);
            let mut parent = node.parent;
            while let Some(index) = parent {
                selected.insert(index);
                parent = nodes
                    .iter()
                    .find(|n| n.index == index)
                    .and_then(|n| n.parent);
            }
        }
    }
    let markdown = selected
        .iter()
        .map(|index| format!("- [{index}] observed"))
        .collect::<Vec<_>>()
        .join("\n");
    let entries = nodes
        .iter()
        .map(|n| serde_json::json!({"element_index":n.index}))
        .collect();
    let projected = dojo_cua_primitives::element_query::project_elements_for_query(
        entries,
        Some(query),
        &markdown,
    );
    let indices: HashSet<_> = projected
        .iter()
        .filter_map(|n| n["element_index"].as_u64())
        .collect();
    nodes.retain(|n| indices.contains(&(n.index as u64)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicUsize};
    fn req<'a>(cancelled: &'a AtomicBool) -> ObserveRequest<'a> {
        let pid = std::process::id();
        ObserveRequest {
            pid,
            process_start_time: process_identity(pid).unwrap().0,
            deadline: Instant::now() + Duration::from_secs(5),
            cancelled,
            options: Options::default(),
        }
    }
    struct Bus {
        pid: u32,
        uid: u32,
    }
    #[zbus::interface(name = "org.freedesktop.DBus")]
    impl Bus {
        fn get_name_owner(&self, _name: &str) -> String {
            ":1.5".into()
        }
        #[zbus(name = "GetConnectionUnixProcessID")]
        fn get_connection_unix_process_id(&self, _name: &str) -> u32 {
            self.pid
        }
        fn get_connection_unix_user(&self, _name: &str) -> u32 {
            self.uid
        }
    }
    #[derive(Clone)]
    struct Accessible {
        role: u32,
        name: String,
        children: Vec<String>,
        private_reads: Arc<AtomicUsize>,
        password: bool,
        count_override: Option<i32>,
    }
    #[zbus::interface(name = "org.a11y.atspi.Accessible")]
    impl Accessible {
        fn get_role(&self) -> u32 {
            self.role
        }
        fn get_state(&self) -> StateSet {
            if self.name == "Disabled" {
                StateSet::empty()
            } else {
                StateSet::new(State::Enabled)
            }
        }
        fn get_interfaces(&self) -> Vec<String> {
            if self.role == Role::Slider as u32 {
                vec![
                    "org.a11y.atspi.Accessible".into(),
                    "org.a11y.atspi.Value".into(),
                ]
            } else if self.password {
                vec![
                    "org.a11y.atspi.Accessible".into(),
                    "org.a11y.atspi.Text".into(),
                ]
            } else {
                vec!["org.a11y.atspi.Accessible".into()]
            }
        }
        fn get_relation_set(&self) -> Vec<(u32, Vec<(String, OwnedObjectPath)>)> {
            let path = match self.name.as_str() {
                "__brightness" => "/label",
                "__credential" => "/tokenlabel",
                _ => return Vec::new(),
            };
            vec![(
                atspi_common::RelationType::LabelledBy as u32,
                vec![(":1.5".into(), OwnedObjectPath::try_from(path).unwrap())],
            )]
        }
        fn get_child_at_index(&self, index: i32) -> zbus::fdo::Result<(String, OwnedObjectPath)> {
            let path = self
                .children
                .get(index as usize)
                .ok_or_else(|| zbus::fdo::Error::Failed("out of range".into()))?;
            Ok((
                ":1.5".into(),
                OwnedObjectPath::try_from(path.as_str()).unwrap(),
            ))
        }
        #[zbus(property)]
        fn child_count(&self) -> i32 {
            self.count_override.unwrap_or(self.children.len() as i32)
        }
        #[zbus(property)]
        fn name(&self) -> String {
            if self.password {
                self.private_reads.fetch_add(1, Ordering::SeqCst);
            }
            if self.name.starts_with("__") {
                String::new()
            } else {
                self.name.clone()
            }
        }
        #[zbus(property)]
        fn description(&self) -> String {
            if self.password {
                self.private_reads.fetch_add(1, Ordering::SeqCst);
            }
            String::new()
        }
    }
    struct Text {
        reads: Arc<AtomicUsize>,
    }
    #[zbus::interface(name = "org.a11y.atspi.Text")]
    impl Text {
        fn get_text(&self, _start: i32, _end: i32) -> String {
            self.reads.fetch_add(1, Ordering::SeqCst);
            "secret password".into()
        }
    }
    struct Value {
        secret_reads: Arc<AtomicUsize>,
        secret: bool,
    }
    #[zbus::interface(name = "org.a11y.atspi.Value")]
    impl Value {
        #[zbus(property)]
        fn minimum_value(&self) -> f64 {
            0.0
        }
        #[zbus(property)]
        fn maximum_value(&self) -> f64 {
            10.0
        }
        #[zbus(property)]
        fn current_value(&self) -> f64 {
            if self.secret {
                self.secret_reads.fetch_add(1, Ordering::SeqCst);
            }
            1.0
        }
    }
    async fn fixture(count_override: Option<i32>) -> (Connection, Connection, Arc<AtomicUsize>) {
        fixture_for_pid(count_override, std::process::id(), false).await
    }
    async fn fixture_for_pid(
        count_override: Option<i32>,
        pid: u32,
        values: bool,
    ) -> (Connection, Connection, Arc<AtomicUsize>) {
        let reads = Arc::new(AtomicUsize::new(0));
        let make = |role, name: &str, children: Vec<&str>, password, count_override| Accessible {
            role,
            name: name.into(),
            children: children.into_iter().map(str::to_owned).collect(),
            private_reads: reads.clone(),
            password,
            count_override,
        };
        let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
        let guid = zbus::Guid::generate();
        let builder = zbus::connection::Builder::async_io_unix_stream(server)
            .p2p()
            .server(guid)
            .unwrap()
            .serve_at(
                "/org/freedesktop/DBus",
                Bus {
                    pid,
                    uid: process_identity(std::process::id()).unwrap().1,
                },
            )
            .unwrap()
            .serve_at(
                "/org/a11y/atspi/accessible/root",
                make(
                    Role::DesktopFrame as u32,
                    "registry",
                    vec!["/app"],
                    false,
                    None,
                ),
            )
            .unwrap()
            .serve_at(
                "/app",
                make(
                    Role::Application as u32,
                    "app",
                    if values {
                        vec!["/label", "/value", "/tokenlabel", "/token"]
                    } else {
                        vec!["/button1", "/button2", "/password", "/disabled"]
                    },
                    false,
                    count_override,
                ),
            )
            .unwrap()
            .serve_at(
                "/button1",
                make(Role::Button as u32, "Save", vec![], false, None),
            )
            .unwrap()
            .serve_at(
                "/button2",
                make(Role::Button as u32, "Save", vec![], false, None),
            )
            .unwrap()
            .serve_at(
                "/password",
                make(
                    Role::PasswordText as u32,
                    "secret password",
                    vec![],
                    true,
                    None,
                ),
            )
            .unwrap()
            .serve_at(
                "/disabled",
                make(Role::Button as u32, "Disabled", vec![], false, None),
            )
            .unwrap()
            .serve_at(
                "/password",
                Text {
                    reads: reads.clone(),
                },
            )
            .unwrap();
        let builder = builder
            .serve_at(
                "/label",
                make(Role::Label as u32, "Brightness", vec![], false, None),
            )
            .unwrap()
            .serve_at(
                "/value",
                make(Role::Slider as u32, "__brightness", vec![], false, None),
            )
            .unwrap()
            .serve_at(
                "/value",
                Value {
                    secret_reads: reads.clone(),
                    secret: false,
                },
            )
            .unwrap()
            .serve_at(
                "/tokenlabel",
                make(Role::Label as u32, "API key", vec![], false, None),
            )
            .unwrap()
            .serve_at(
                "/token",
                make(Role::Slider as u32, "__credential", vec![], false, None),
            )
            .unwrap()
            .serve_at(
                "/token",
                Value {
                    secret_reads: reads.clone(),
                    secret: true,
                },
            )
            .unwrap();
        let (server, client) = tokio::join!(
            builder.build(),
            zbus::connection::Builder::async_io_unix_stream(client)
                .p2p()
                .build()
        );
        (server.unwrap(), client.unwrap(), reads)
    }
    fn empty(request: &ObserveRequest<'_>) -> Observation {
        Observation {
            bus_guid: None,
            pid: request.pid,
            process_start_time: request.process_start_time,
            scope: "application",
            actionable: false,
            window_scoped: false,
            available: false,
            complete: false,
            query_filtered: request.options.query.is_some(),
            issues: BTreeSet::new(),
            nodes_visited: 0,
            nodes_pending: 0,
            nodes: Vec::new(),
        }
    }

    struct SelectionAccessible {
        container: bool,
        index: i32,
        parent_path: String,
    }
    #[zbus::interface(name = "org.a11y.atspi.Accessible")]
    impl SelectionAccessible {
        fn get_role(&self) -> u32 {
            if self.container {
                Role::List as u32
            } else {
                Role::ListItem as u32
            }
        }
        fn get_state(&self) -> StateSet {
            StateSet::new(State::Sensitive)
        }
        fn get_interfaces(&self) -> Vec<String> {
            let mut interfaces = vec!["org.a11y.atspi.Accessible".into()];
            if self.container {
                interfaces.push("org.a11y.atspi.Selection".into());
            }
            interfaces
        }
        fn get_index_in_parent(&self) -> i32 {
            self.index
        }
        fn get_child_at_index(&self, index: i32) -> zbus::fdo::Result<(String, OwnedObjectPath)> {
            let path = match index {
                0 => "/choice",
                1 => "/sibling",
                _ => return Err(zbus::fdo::Error::Failed("out of range".into())),
            };
            Ok((":1.5".into(), OwnedObjectPath::try_from(path).unwrap()))
        }
        #[zbus(property)]
        fn parent(&self) -> (String, OwnedObjectPath) {
            (
                ":1.5".into(),
                OwnedObjectPath::try_from(self.parent_path.as_str()).unwrap(),
            )
        }
        #[zbus(property)]
        fn name(&self) -> &str {
            if self.container { "Choices" } else { "Choice" }
        }
        #[zbus(property)]
        fn description(&self) -> &str {
            ""
        }
        #[zbus(property)]
        fn child_count(&self) -> i32 {
            if self.container { 2 } else { 0 }
        }
    }

    #[tokio::test]
    async fn direct_selection_requires_live_bidirectional_parent_membership() {
        for (index, parent_path, valid) in [
            (0, "/list", true),
            (1, "/list", false),
            (-1, "/list", false),
            (2048, "/list", false),
            (0, "/other_parent", false),
        ] {
            let (server, conn, _) = fixture(None).await;
            for (path, container, index) in [
                ("/list", true, -1),
                ("/choice", false, index),
                ("/sibling", false, 1),
            ] {
                server
                    .object_server()
                    .at(
                        path,
                        SelectionAccessible {
                            container,
                            index,
                            parent_path: parent_path.into(),
                        },
                    )
                    .await
                    .unwrap();
            }
            let cancelled = AtomicBool::new(false);
            let request = req(&cancelled);
            let root = WindowRoot::new(":1.5".into(), "/list".into()).unwrap();
            let mut result = empty(&request);
            observe_selected_on_connection(
                &request,
                &conn,
                process_identity(request.pid).unwrap().1,
                &mut result,
                Some(&root),
            )
            .await
            .unwrap();
            let choice = result
                .nodes
                .iter()
                .find(|node| node.object_path == "/choice")
                .unwrap();
            assert_eq!(
                choice.enabled,
                Some(true),
                "sensitive-only toolkit state is enabled"
            );
            assert_eq!(
                choice.selection_parent,
                valid.then(|| SelectionParent {
                    bus_owner: ":1.5".into(),
                    object_path: "/list".into(),
                    index: 0,
                })
            );
            assert_eq!(
                actions::select(choice, &actions::Action::Select).is_ok(),
                valid
            );
        }
    }
    #[tokio::test]
    async fn walks_real_dbus_fixture_and_never_reads_password_fields() {
        let (_server, conn, reads) = fixture(None).await;
        let cancelled = AtomicBool::new(false);
        let request = req(&cancelled);
        let mut result = empty(&request);
        observe_on_connection(
            &request,
            &conn,
            process_identity(request.pid).unwrap().1,
            &mut result,
        )
        .await
        .unwrap();
        assert!(result.available, "{:?}", result.issues);
        assert!(result.issues.is_empty(), "{:?}", result.issues);
        assert_eq!(
            result
                .nodes
                .iter()
                .map(|n| (n.index, n.parent, n.role.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (0, None, "application"),
                (1, Some(0), "button"),
                (2, Some(0), "button"),
                (3, Some(0), "password text"),
                (4, Some(0), "button")
            ]
        );
        assert_eq!(result.nodes[1].name.as_deref(), Some("Save"));
        assert_eq!(result.nodes[2].name.as_deref(), Some("Save"));
        assert_eq!(result.nodes[4].enabled, Some(false));
        assert!(result.nodes[3].redacted);
        assert!(result.nodes[3].name.is_none() && result.nodes[3].text.is_none());
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert!(!result.actionable && !result.window_scoped);
        project(&mut result.nodes, Some("password"));
        assert_eq!(
            result.nodes.iter().map(|n| n.index).collect::<Vec<_>>(),
            vec![0, 3]
        );
    }
    #[tokio::test]
    async fn selected_native_root_does_not_walk_same_process_siblings() {
        let (_server, conn, _) = fixture(None).await;
        let cancelled = AtomicBool::new(false);
        let request = req(&cancelled);
        for path in ["/button1", "/button2"] {
            let root = WindowRoot::new(":1.5".into(), path.into()).unwrap();
            let mut result = empty(&request);
            observe_selected_on_connection(
                &request,
                &conn,
                process_identity(request.pid).unwrap().1,
                &mut result,
                Some(&root),
            )
            .await
            .unwrap();
            assert!(result.available && result.window_scoped);
            assert!(!result.actionable);
            assert_eq!(result.scope, "window");
            assert_eq!(result.nodes.len(), 1);
            assert_eq!(result.nodes[0].object_path, path);
            assert_eq!(result.nodes[0].name.as_deref(), Some("Save"));
        }
    }
    #[tokio::test]
    async fn selected_root_with_different_process_is_not_an_application_fallback() {
        let (_server, conn, _) = fixture_for_pid(None, std::process::id() + 1, false).await;
        let cancelled = AtomicBool::new(false);
        let request = req(&cancelled);
        let root = WindowRoot::new(":1.5".into(), "/button1".into()).unwrap();
        let mut result = empty(&request);
        observe_selected_on_connection(
            &request,
            &conn,
            process_identity(request.pid).unwrap().1,
            &mut result,
            Some(&root),
        )
        .await
        .unwrap();
        assert!(!result.available && !result.window_scoped);
        assert!(result.nodes.is_empty());
        assert!(result.issues.contains("window_owner_mismatch"));
    }
    #[test]
    fn native_endpoints_require_unique_names_and_valid_non_null_paths() {
        for (owner, path) in [
            ("org.example.App", "/window"),
            (":1.5", "bad path"),
            (":1.5", "/org/a11y/atspi/null"),
        ] {
            assert!(WindowRoot::new(owner.into(), path.into()).is_err());
        }
    }
    #[tokio::test]
    async fn huge_container_is_incomplete_without_enumerating_children() {
        let (_server, conn, _) = fixture(Some(i32::MAX)).await;
        let cancelled = AtomicBool::new(false);
        let request = req(&cancelled);
        let mut result = empty(&request);
        observe_on_connection(
            &request,
            &conn,
            process_identity(request.pid).unwrap().1,
            &mut result,
        )
        .await
        .unwrap();
        assert_eq!(result.nodes.len(), 1);
        assert!(result.issues.contains("huge_container"));
    }
    #[tokio::test]
    async fn node_and_depth_budgets_keep_original_indices() {
        let (_server, conn, _) = fixture(None).await;
        let cancelled = AtomicBool::new(false);
        let mut request = req(&cancelled);
        request.options.max_nodes = 2;
        let mut result = empty(&request);
        observe_on_connection(
            &request,
            &conn,
            process_identity(request.pid).unwrap().1,
            &mut result,
        )
        .await
        .unwrap();
        assert_eq!(
            result.nodes.iter().map(|n| n.index).collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(result.issues.contains("node_budget"));
        request.options.max_nodes = 5000;
        request.options.max_depth = 0;
        let mut result = empty(&request);
        observe_on_connection(
            &request,
            &conn,
            process_identity(request.pid).unwrap().1,
            &mut result,
        )
        .await
        .unwrap();
        assert_eq!(result.nodes.len(), 1);
        assert!(result.issues.contains("depth_budget"));
    }
    #[tokio::test]
    async fn hanging_query_stops_on_deadline_and_cancel() {
        let cancelled = AtomicBool::new(false);
        let mut request = req(&cancelled);
        request.deadline = Instant::now() + Duration::from_millis(30);
        let budget = Budget { request: &request };
        assert!(matches!(
            budget.read(std::future::pending::<Result<(), ()>>()).await,
            Err(ReadError::Deadline)
        ));
        request.deadline = Instant::now() + Duration::from_secs(1);
        let budget = Budget { request: &request };
        let (answer, _) = tokio::join!(
            budget.read(std::future::pending::<Result<(), ()>>()),
            async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                cancelled.store(true, Ordering::Release);
            }
        );
        assert!(matches!(answer, Err(ReadError::Canceled)));
    }
    #[tokio::test]
    async fn reused_pid_is_refused_before_bus_discovery() {
        let cancelled = AtomicBool::new(false);
        let mut request = req(&cancelled);
        request.process_start_time += 1;
        assert!(
            observe(&request)
                .await
                .unwrap_err()
                .to_string()
                .contains("identity changed")
        );
    }
    #[test]
    fn sanitization_and_bounds_preserve_upstream_limits() {
        assert_eq!(sanitize("ab\0\u{1b}c🦀\nz", 5), "abc🦀\n");
        assert!(sensitive_label(Some("API key"), None));
        assert_eq!(sanitize("document </text>", 64), "document </text>");
        assert_eq!(plausible_bounds((0, 0, 20, 30)), Some((0, 0, 20, 30)));
        for bounds in [
            (i32::MIN, 0, 20, 30),
            (0, i32::MIN, 20, 30),
            (0, 0, 1, 30),
            (-16385, 0, 20, 30),
        ] {
            assert!(plausible_bounds(bounds).is_none());
        }
    }
    #[tokio::test]
    async fn wrong_bus_process_is_unavailable_without_name_fallback() {
        let (_server, conn, _) = fixture_for_pid(None, std::process::id() + 1, false).await;
        let cancelled = AtomicBool::new(false);
        let request = req(&cancelled);
        let mut result = empty(&request);
        observe_on_connection(
            &request,
            &conn,
            process_identity(request.pid).unwrap().1,
            &mut result,
        )
        .await
        .unwrap();
        assert!(!result.available);
        assert!(result.nodes.is_empty());
        assert!(result.issues.contains("application_unavailable"));
    }
    #[tokio::test]
    async fn labelled_numeric_values_preserve_format_and_secret_fields_are_not_read() {
        let (_server, conn, reads) = fixture_for_pid(None, std::process::id(), true).await;
        let cancelled = AtomicBool::new(false);
        let request = req(&cancelled);
        let mut result = empty(&request);
        observe_on_connection(
            &request,
            &conn,
            process_identity(request.pid).unwrap().1,
            &mut result,
        )
        .await
        .unwrap();
        assert!(result.issues.is_empty(), "{:?}", result.issues);
        assert_eq!(result.nodes[2].name.as_deref(), Some("Brightness"));
        assert_eq!(result.nodes[2].value.as_deref(), Some("1.0"));
        assert!(result.nodes[4].redacted);
        assert!(result.nodes[4].name.is_none() && result.nodes[4].value.is_none());
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        project(&mut result.nodes, Some("1.0"));
        assert_eq!(
            result.nodes.iter().map(|n| n.index).collect::<Vec<_>>(),
            vec![0, 2]
        );
    }

    #[test]
    fn editable_text_value_preserves_upstream_precedence() {
        assert_eq!(
            editable_text_value(true, false, None, "jev-use native note").as_deref(),
            Some("jev-use native note")
        );
        assert_eq!(editable_text_value(true, false, None, ""), None);
        assert_eq!(
            editable_text_value(false, false, None, "static label"),
            None
        );
        assert_eq!(
            editable_text_value(true, true, Some("3".into()), "3.0").as_deref(),
            Some("3")
        );
        assert_eq!(editable_text_value(true, true, None, "3.0"), None);
    }
}
