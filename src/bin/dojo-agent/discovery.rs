use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result, bail, ensure};
use dojo::application_catalogue::Catalogue;
use niri_ipc::{Output, Request, Response, Window, Workspace};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListWindowsInput {
    #[serde(
        default,
        deserialize_with = "optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    pid: Option<u32>,
    #[serde(
        default,
        deserialize_with = "optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    on_screen_only: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ScreenSizeInput {
    #[serde(
        default,
        deserialize_with = "optional_field",
        skip_serializing_if = "Option::is_none"
    )]
    session: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EmptyInput {}

fn optional_field<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(deserializer).map(Some)
}

macro_rules! pinned_input {
    ($type:ty, $tool:literal) => {
        impl schemars::JsonSchema for $type {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                $tool.into()
            }
            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                let manifest: Value = serde_json::from_str(include_str!(
                    "../../../provenance/contracts/cua-driver.manifest.json"
                ))
                .expect("pinned contract JSON");
                let tool = manifest["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|tool| tool["name"] == $tool)
                    .expect("pinned discovery tool");
                schemars::Schema::try_from(tool["input_schema"].clone())
                    .expect("pinned input object schema")
            }
        }
    };
}
pinned_input!(ListWindowsInput, "list_windows");
pinned_input!(ScreenSizeInput, "get_screen_size");
pinned_input!(EmptyInput, "list_apps");

fn windows(socket: &mut niri_ipc::socket::Socket) -> Result<Vec<Window>> {
    let Response::Windows(windows) = socket.send(Request::Windows)?.map_err(anyhow::Error::msg)?
    else {
        bail!("unexpected windows response");
    };
    ensure!(windows.len() <= 8192, "window discovery limit exceeded");
    Ok(windows)
}

fn layout(
    socket: &mut niri_ipc::socket::Socket,
) -> Result<(Vec<Workspace>, HashMap<String, Output>)> {
    let Response::Workspaces(workspaces) = socket
        .send(Request::Workspaces)?
        .map_err(anyhow::Error::msg)?
    else {
        bail!("unexpected workspaces response");
    };
    let Response::Outputs(outputs) = socket.send(Request::Outputs)?.map_err(anyhow::Error::msg)?
    else {
        bail!("unexpected outputs response");
    };
    ensure!(
        outputs.len() <= 64 && workspaces.len() <= 4096,
        "layout discovery limit exceeded"
    );
    Ok((workspaces, outputs))
}

pub(super) fn list_windows(socket_path: Option<&Path>, input: ListWindowsInput) -> Result<Value> {
    ensure!(input.pid != Some(0), "pid must be positive");
    let mut socket = super::desktop_socket(socket_path)?;
    let windows = windows(&mut socket)?;
    let (workspaces, outputs) = layout(&mut socket)?;
    project_windows(&windows, &workspaces, &outputs, &input)
}

fn project_windows(
    windows: &[Window],
    workspaces: &[Workspace],
    outputs: &HashMap<String, Output>,
    input: &ListWindowsInput,
) -> Result<Value> {
    let mut result = Vec::new();
    for window in windows {
        let pid = window
            .pid
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0);
        if input.pid.is_some_and(|wanted| pid != Some(wanted)) {
            continue;
        }
        let workspace = workspaces
            .iter()
            .find(|workspace| Some(workspace.id) == window.workspace_id);
        let output = workspace
            .and_then(|workspace| workspace.output.as_ref())
            .and_then(|name| outputs.get(name));
        let logical = output.and_then(|output| output.logical);
        if input.on_screen_only == Some(true)
            && (!workspace.is_some_and(|workspace| workspace.is_active) || logical.is_none())
        {
            continue;
        }
        let Some((tile_x, tile_y)) = window.layout.tile_pos_in_workspace_view else {
            bail!("window {} layout position unavailable", window.id);
        };
        let Some(logical) = logical else {
            bail!("window {} output geometry unavailable", window.id);
        };
        let x = tile_x + window.layout.window_offset_in_tile.0;
        let y = tile_y + window.layout.window_offset_in_tile.1;
        let (width, height) = window.layout.window_size;
        ensure!(
            x.is_finite() && y.is_finite() && width > 0 && height > 0,
            "invalid window geometry"
        );
        let on_screen = workspace.is_some_and(|workspace| workspace.is_active)
            && x < f64::from(logical.width)
            && y < f64::from(logical.height)
            && x + f64::from(width) > 0.0
            && y + f64::from(height) > 0.0;
        if input.on_screen_only == Some(true) && !on_screen {
            continue;
        }
        result.push(json!({
            "window_id": window.id, "pid": pid,
            "app_name": window.app_id.as_deref().unwrap_or(""),
            "title": window.title.as_deref().unwrap_or(""),
            "bounds": {"x": x + f64::from(logical.x), "y": y + f64::from(logical.y), "width": width, "height": height},
            "x": x + f64::from(logical.x), "y": y + f64::from(logical.y), "width": width, "height": height,
            "is_on_screen": on_screen, "z_index": null, "layer": null, "minimized": null,
            "current_space_id": workspace.map(|workspace| workspace.id),
            "space_ids": window.workspace_id.map(|id| vec![id]),
            "on_current_space": workspace.map(|workspace| workspace.is_active),
            "output": output.map(|output| &output.name),
        }));
    }
    result.sort_by_key(|window| window["window_id"].as_u64());
    Ok(
        json!({"windows": result, "current_space_id": workspaces.iter().find(|workspace| workspace.is_focused).map(|workspace| workspace.id),
        "coordinate_system": "global logical pixels", "bounds_source": "niri_workspace_layout", "snapshot_atomic": false,
        "visibility_semantics": "active workspace and output intersection; occlusion unknown", "action_authority": false}),
    )
}

pub(super) fn screen_size(socket_path: Option<&Path>, input: ScreenSizeInput) -> Result<Value> {
    ensure!(
        input.session.is_none(),
        "named lifecycle sessions are not supported; omit session for this transport"
    );
    let mut socket = super::desktop_socket(socket_path)?;
    let (workspaces, outputs) = layout(&mut socket)?;
    project_screen_size(&workspaces, &outputs)
}

fn project_screen_size(
    workspaces: &[Workspace],
    outputs: &HashMap<String, Output>,
) -> Result<Value> {
    let enabled = outputs
        .values()
        .filter(|output| output.current_mode.is_some() && output.logical.is_some())
        .collect::<Vec<_>>();
    let focused = workspaces
        .iter()
        .find(|workspace| workspace.is_focused)
        .and_then(|workspace| workspace.output.as_ref());
    let (output, selection) = if let Some(output) = focused
        .and_then(|name| enabled.iter().find(|output| &output.name == name))
        .copied()
    {
        (output, "focused_workspace_output")
    } else if enabled.len() == 1 {
        (enabled[0], "sole_enabled_output")
    } else {
        bail!("primary output is ambiguous or unavailable");
    };
    let logical = output.logical.unwrap();
    ensure!(
        logical.width > 0 && logical.height > 0 && logical.scale.is_finite() && logical.scale > 0.0,
        "invalid output geometry"
    );
    Ok(
        json!({"width": logical.width, "height": logical.height, "scale_factor": logical.scale,
        "output": output.name, "selection": selection, "coordinate_system": "output-relative logical pixels",
        "origin": {"x":logical.x,"y":logical.y}, "transform":logical.transform, "snapshot_atomic":false}),
    )
}

pub(super) fn list_apps(socket_path: Option<&Path>) -> Result<Value> {
    let catalogue = Catalogue::discover()?;
    ensure!(
        catalogue.applications.len() <= 8192,
        "installed application discovery limit exceeded"
    );
    let mut socket = super::desktop_socket(socket_path)?;
    let windows = windows(&mut socket)?;
    let (workspaces, outputs) = layout(&mut socket)?;
    let records = project_windows(
        &windows,
        &workspaces,
        &outputs,
        &ListWindowsInput {
            pid: None,
            on_screen_only: None,
        },
    )?;
    let records = records["windows"].as_array().context("window records")?;
    let installed = catalogue
        .applications
        .iter()
        .map(|app| InstalledApp {
            bundle_id: app
                .id
                .strip_suffix(".desktop")
                .unwrap_or(&app.id)
                .to_owned(),
            name: app.name.clone(),
            launch_path: app.launch_command.clone().unwrap_or_default(),
            startup_wm_class: app.startup_wm_class.clone(),
            last_used: app.last_used.clone(),
        })
        .collect::<Vec<_>>();
    Ok(project_apps(&installed, records, &running_processes()?))
}

struct InstalledApp {
    bundle_id: String,
    name: String,
    launch_path: String,
    startup_wm_class: Option<String>,
    last_used: Option<String>,
}
impl InstalledApp {
    fn owns_window_class(&self, class: &str) -> bool {
        if class.is_empty() {
            return false;
        }
        let stem = self.bundle_id.rsplit('.').next().unwrap_or(&self.bundle_id);
        let executable = self
            .launch_path
            .split_whitespace()
            .next()
            .and_then(|token| token.rsplit('/').next())
            .unwrap_or("");
        self.startup_wm_class
            .as_deref()
            .into_iter()
            .chain([self.bundle_id.as_str(), stem, executable])
            .any(|key| !key.is_empty() && key.eq_ignore_ascii_case(class))
    }
}

fn project_apps(installed: &[InstalledApp], windows: &[Value], processes: &[Process]) -> Value {
    let by_window: HashMap<usize, u32> = installed
        .iter()
        .enumerate()
        .filter_map(|(index, app)| {
            windows
                .iter()
                .find(|window| {
                    window["pid"].as_u64().is_some()
                        && app.owns_window_class(window["app_name"].as_str().unwrap_or(""))
                })
                .and_then(|window| window["pid"].as_u64())
                .and_then(|pid| u32::try_from(pid).ok())
                .map(|pid| (index, pid))
        })
        .collect();
    let mut by_exe: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, app) in installed.iter().enumerate() {
        let basename = exec_basename(&app.launch_path);
        if !basename.is_empty() {
            by_exe.entry(basename).or_default().push(index);
        }
    }
    let mut consumed = std::collections::HashSet::new();
    let mut apps = Vec::new();
    for process in processes {
        let key = if process.executable.is_empty() {
            &process.name
        } else {
            &process.executable
        };
        let basename = exec_basename(key);
        if basename.is_empty() {
            continue;
        }
        let candidates = by_exe.get(&basename).map(Vec::as_slice).unwrap_or(&[]);
        let merged = disambiguate_installed_match(candidates, installed, key).or_else(|| {
            by_window
                .iter()
                .filter(|(_, pid)| **pid == process.pid)
                .map(|(index, _)| *index)
                .min()
        });
        if let Some(index) = merged {
            consumed.insert(index);
        }
        let app = merged.map(|index| &installed[index]);
        let owned = windows
            .iter()
            .filter(|window| window["pid"].as_u64() == Some(u64::from(process.pid)))
            .filter(|window| {
                app.is_none_or(|app| {
                    app.owns_window_class(window["app_name"].as_str().unwrap_or(""))
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        apps.push(json!({"pid":process.pid,"name":app.map(|app| app.name.as_str()).unwrap_or(if process.name.is_empty() { &basename } else { &process.name }),
            "running":true,"active":false,"bundle_id":app.map(|app| &app.bundle_id),"launch_path":app.map(|app| &app.launch_path),
            "kind":app.map(|_| "desktop"),"last_used":app.and_then(|app| app.last_used.as_ref()),"windows":owned}));
    }
    for (index, app) in installed.iter().enumerate() {
        if consumed.contains(&index) {
            continue;
        }
        let pid = by_window.get(&index).copied();
        let owned = windows
            .iter()
            .filter(|window| {
                window["pid"].as_u64().is_some()
                    && window["pid"].as_u64() == pid.map(u64::from)
                    && app.owns_window_class(window["app_name"].as_str().unwrap_or(""))
            })
            .cloned()
            .collect::<Vec<_>>();
        apps.push(json!({"pid":pid.unwrap_or(0),"name":app.name,"running":pid.is_some(),"active":false,
            "bundle_id":app.bundle_id,"launch_path":app.launch_path,"kind":"desktop","last_used":app.last_used,"windows":owned}));
    }
    json!({"apps":apps,"snapshot_atomic":false})
}

fn disambiguate_installed_match(
    candidates: &[usize],
    installed: &[InstalledApp],
    key: &str,
) -> Option<usize> {
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates[0]);
    }
    let path = key.split_whitespace().next().unwrap_or("");
    if !path.is_empty() {
        if let Some(&index) = candidates
            .iter()
            .find(|&&index| installed[index].launch_path == path)
        {
            return Some(index);
        }
    }
    candidates
        .iter()
        .copied()
        .max_by(|&a, &b| {
            installed[a]
                .last_used
                .as_deref()
                .unwrap_or("")
                .cmp(installed[b].last_used.as_deref().unwrap_or(""))
        })
        .or_else(|| candidates.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: u64, pid: i32, workspace: u64, x: f64, focused: bool) -> Window {
        serde_json::from_value(
            json!({"id":id,"pid":pid,"workspace_id":workspace,"app_id":"editor","title":"Document",
            "is_focused":focused,"is_floating":false,"is_urgent":false,"focus_timestamp":null,
            "layout":{"pos_in_scrolling_layout":[1,1],"tile_size":[100,80],"window_size":[96,76],
                "tile_pos_in_workspace_view":[x,10],"window_offset_in_tile":[2,2]}}),
        )
        .unwrap()
    }
    fn workspace(id: u64, active: bool, focused: bool) -> Workspace {
        serde_json::from_value(
            json!({"id":id,"idx":id,"name":null,"output":"DP-1","is_urgent":false,
            "is_active":active,"is_focused":focused,"active_window_id":null}),
        )
        .unwrap()
    }
    fn output(name: &str, x: i32) -> Output {
        serde_json::from_value(json!({"name":name,"make":"Fixture","model":"Fixture","serial":null,"physical_size":null,
            "modes":[{"width":1500,"height":900,"refresh_rate":60000,"is_preferred":true}],"current_mode":0,
            "is_custom_mode":false,"vrr_supported":false,"vrr_enabled":false,"max_bpc":null,
            "logical":{"x":x,"y":-100,"width":1000,"height":600,"scale":1.5,"transform":"Normal"}})).unwrap()
    }
    #[test]
    fn window_filters_use_exact_pid_visibility_and_global_logical_bounds() {
        let windows = [
            window(1, 42, 1, 10.0, true),
            window(2, 43, 1, 10.0, false),
            window(3, 42, 2, 10.0, false),
            window(4, 42, 1, -200.0, false),
        ];
        let workspaces = [workspace(1, true, true), workspace(2, false, false)];
        let outputs = HashMap::from([("DP-1".into(), output("DP-1", 1920))]);
        let result = project_windows(
            &windows,
            &workspaces,
            &outputs,
            &ListWindowsInput {
                pid: Some(42),
                on_screen_only: Some(true),
            },
        )
        .unwrap();
        assert_eq!(result["windows"].as_array().unwrap().len(), 1);
        assert_eq!(result["windows"][0]["window_id"], 1);
        assert_eq!(
            result["windows"][0]["bounds"],
            json!({"x":1932.0,"y":-88.0,"width":96,"height":76})
        );
        assert_eq!(result["windows"][0]["z_index"], Value::Null);
        assert_eq!(result["action_authority"], false);
        let all = project_windows(
            &windows,
            &workspaces,
            &outputs,
            &ListWindowsInput {
                pid: None,
                on_screen_only: None,
            },
        )
        .unwrap();
        assert_eq!(all["windows"].as_array().unwrap().len(), 4);
        assert_eq!(all["windows"][2]["is_on_screen"], false);
        assert_eq!(all["windows"][3]["is_on_screen"], false);
    }
    #[test]
    fn missing_geometry_is_not_fabricated() {
        let mut window = window(1, 42, 1, 10.0, false);
        window.layout.tile_pos_in_workspace_view = None;
        assert!(
            project_windows(
                &[window],
                &[workspace(1, true, true)],
                &HashMap::from([("DP-1".into(), output("DP-1", 0))]),
                &ListWindowsInput {
                    pid: None,
                    on_screen_only: None
                }
            )
            .is_err()
        );
    }
    #[test]
    fn primary_selection_is_focused_or_unique_never_hashmap_order() {
        let mut outputs = HashMap::from([
            ("DP-1".into(), output("DP-1", 1920)),
            ("DP-2".into(), output("DP-2", 0)),
        ]);
        let selected = project_screen_size(&[workspace(1, true, true)], &outputs).unwrap();
        assert_eq!(selected["output"], "DP-1");
        assert_eq!(selected["width"], 1000);
        assert_eq!(selected["height"], 600);
        assert_eq!(selected["scale_factor"], 1.5);
        assert!(project_screen_size(&[], &outputs).is_err());
        outputs.remove("DP-2");
        assert_eq!(
            project_screen_size(&[], &outputs).unwrap()["selection"],
            "sole_enabled_output"
        );
        outputs.get_mut("DP-1").unwrap().current_mode = None;
        assert!(project_screen_size(&[], &outputs).is_err());
    }
    fn app(id: &str, name: &str, command: &str) -> InstalledApp {
        InstalledApp {
            bundle_id: id.into(),
            name: name.into(),
            launch_path: command.into(),
            startup_wm_class: None,
            last_used: None,
        }
    }
    #[test]
    fn application_projection_keeps_installed_and_exact_running_instances() {
        let installed = [
            app("editor", "Editor", "/fixture/editor"),
            app("idle", "Idle", "/fixture/idle"),
        ];
        let records = project_windows(
            &[
                window(1, 42, 1, 0.0, true),
                window(2, 42, 1, 0.0, false),
                window(3, 43, 1, 0.0, false),
            ],
            &[workspace(1, true, true)],
            &HashMap::from([("DP-1".into(), output("DP-1", 0))]),
            &ListWindowsInput {
                pid: None,
                on_screen_only: None,
            },
        )
        .unwrap();
        let processes = [
            Process {
                pid: 42,
                name: "editor".into(),
                executable: "/fixture/editor".into(),
            },
            Process {
                pid: 43,
                name: "editor".into(),
                executable: "/fixture/editor".into(),
            },
        ];
        let result = project_apps(
            &installed,
            records["windows"].as_array().unwrap(),
            &processes,
        );
        let apps = result["apps"].as_array().unwrap();
        assert_eq!(apps.len(), 3);
        assert_eq!(apps[0]["pid"], 42);
        assert_eq!(apps[0]["active"], false);
        assert_eq!(apps[0]["bundle_id"], "editor");
        assert_eq!(apps[0]["launch_path"], "/fixture/editor");
        assert_eq!(apps[0]["kind"], "desktop");
        assert_eq!(apps[0]["windows"].as_array().unwrap().len(), 2);
        assert_eq!(apps[1]["pid"], 43);
        assert_eq!(apps[1]["active"], false);
        assert_eq!(apps[2]["pid"], 0);
        assert_eq!(apps[2]["running"], false);
    }
    #[test]
    fn background_processes_merge_without_exporting_arguments() {
        let installed = [app("editor", "Editor", "env MODE=1 /usr/bin/editor")];
        let processes = [
            Process {
                pid: 77,
                name: "editor".into(),
                executable: "/usr/bin/editor".into(),
            },
            Process {
                pid: 78,
                name: "service".into(),
                executable: "/usr/bin/service".into(),
            },
        ];
        let result = project_apps(&installed, &[], &processes);
        let apps = result["apps"].as_array().unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0]["pid"], 77);
        assert_eq!(apps[0]["bundle_id"], "editor");
        assert_eq!(apps[1]["name"], "service");
        assert!(apps[1]["bundle_id"].is_null());
        assert!(apps.iter().all(|app| app.get("cmdline").is_none()));
    }
    #[test]
    fn launcher_ambiguity_prefers_exact_path_then_newest_timestamp() {
        let mut installed = [
            app("old", "Old", "/one/editor"),
            app("new", "New", "/two/editor"),
        ];
        installed[0].last_used = Some("2026-01-01T00:00:00Z".into());
        installed[1].last_used = Some("2026-02-01T00:00:00Z".into());
        assert_eq!(
            disambiguate_installed_match(&[0, 1], &installed, "/one/editor"),
            Some(0)
        );
        assert_eq!(
            disambiguate_installed_match(&[0, 1], &installed, "editor"),
            Some(1)
        );
        assert_eq!(
            disambiguate_installed_match(&[], &installed, "editor"),
            None
        );
    }
    #[test]
    fn one_process_can_own_two_launcher_entries_through_window_classes() {
        let mut installed = [
            app("calc", "Calc", "libreoffice"),
            app("writer", "Writer", "libreoffice"),
        ];
        installed[0].startup_wm_class = Some("LibreOffice-Calc".into());
        installed[1].startup_wm_class = Some("LibreOffice-Writer".into());
        let windows = [
            json!({"window_id":1,"pid":42,"app_name":"libreoffice-calc"}),
            json!({"window_id":2,"pid":42,"app_name":"libreoffice-writer"}),
        ];
        let result = project_apps(
            &installed,
            &windows,
            &[Process {
                pid: 42,
                name: "soffice.bin".into(),
                executable: "soffice.bin".into(),
            }],
        );
        let apps = result["apps"].as_array().unwrap();
        assert_eq!(apps.len(), 2);
        assert!(
            apps.iter()
                .all(|app| app["pid"] == 42 && app["running"] == true)
        );
        assert_eq!(apps[0]["windows"][0]["window_id"], 1);
        assert_eq!(apps[1]["windows"][0]["window_id"], 2);
    }
    #[test]
    fn invalid_filters_and_named_sessions_fail_before_transport() {
        assert!(
            list_windows(
                Some(Path::new("/missing/socket")),
                ListWindowsInput {
                    pid: Some(0),
                    on_screen_only: None
                }
            )
            .unwrap_err()
            .to_string()
            .contains("positive")
        );
        assert!(
            screen_size(
                Some(Path::new("/missing/socket")),
                ScreenSizeInput {
                    session: Some("another".into())
                }
            )
            .unwrap_err()
            .to_string()
            .contains("named lifecycle")
        );
        assert!(serde_json::from_value::<ListWindowsInput>(json!({"pid":-1})).is_err());
        assert!(serde_json::from_value::<ListWindowsInput>(json!({"pid":4294967296_u64})).is_err());
        assert!(serde_json::from_value::<ListWindowsInput>(json!({"command":"inject"})).is_err());
        assert!(serde_json::from_value::<ListWindowsInput>(json!({"pid":null})).is_err());
        assert!(serde_json::from_value::<ScreenSizeInput>(json!({"session":null})).is_err());
        assert!(serde_json::from_value::<EmptyInput>(json!({"unexpected":true})).is_err());
    }
}

fn exec_basename(s: &str) -> String {
    // Take the first whitespace-separated token that looks like a binary,
    // skipping `env`-style prefixes and `K=V` assignments.
    for tok in s.split_whitespace() {
        if tok == "env" {
            continue;
        }
        if tok.contains('=') && !tok.starts_with('/') && !tok.starts_with('-') {
            // `FOO=bar` env-var assignment — skip.
            continue;
        }
        let cleaned = tok.trim_matches(|c| c == '"' || c == '\'');
        let basename = std::path::Path::new(cleaned)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(cleaned);
        return basename.to_ascii_lowercase();
    }
    String::new()
}

struct Process {
    pid: u32,
    name: String,
    executable: String,
}

fn running_processes() -> Result<Vec<Process>> {
    use std::io::Read;
    fn bounded(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(limit + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "process metadata exceeds limit",
            ));
        }
        Ok(bytes)
    }
    let mut processes = Vec::new();
    let mut count = 0;
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        count += 1;
        ensure!(count <= 32768, "process discovery limit exceeded");
        let Ok(status) = bounded(&entry.path().join("status"), 65536) else {
            continue;
        };
        let status = String::from_utf8_lossy(&status);
        let state = status
            .lines()
            .find_map(|line| line.strip_prefix("State:"))
            .and_then(|state| state.trim().chars().next());
        if !state.is_some_and(|state| !matches!(state, 'Z' | 'X')) {
            continue;
        }
        let name = status
            .lines()
            .find_map(|line| line.strip_prefix("Name:"))
            .unwrap_or("")
            .trim()
            .to_owned();
        let executable = bounded(&entry.path().join("cmdline"), 65536)
            .ok()
            .map(|bytes| {
                String::from_utf8_lossy(bytes.split(|byte| *byte == 0).next().unwrap_or(&[]))
                    .trim()
                    .to_owned()
            })
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| name.clone());
        processes.push(Process {
            pid,
            name,
            executable,
        });
    }
    processes.sort_by_key(|process| process.pid);
    Ok(processes)
}
