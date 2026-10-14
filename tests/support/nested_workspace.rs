use anyhow::{Context, Result, bail, ensure};
use niri_ipc::{Request, socket::Socket};
use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, Instant};

fn request(socket: &Path, value: Value) -> Result<Value> {
    let mut connection = Socket::connect_to(socket)?;
    connection.set_read_timeout(Some(Duration::from_secs(3)))?;
    let response = connection
        .send(serde_json::from_value::<Request>(value)?)?
        .map_err(anyhow::Error::msg)?;
    Ok(serde_json::to_value(response)?)
}
fn wait<T>(description: &str, mut check: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = check()? {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            bail!("timed out: {description}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn isolate(compositor_pid: u32) -> Result<()> {
    if let Some(outer_path) = std::env::var_os("NIRI_SOCKET").map(std::path::PathBuf::from) {
        let pid = compositor_pid;
        let host = wait("owned nested host window", || {
            let windows = request(&outer_path, json!("Windows"))?;
            Ok(windows["Windows"]
                .as_array()
                .and_then(|windows| windows.iter().find(|window| window["pid"] == pid))
                .and_then(|window| window["id"].as_u64()))
        })?;
        let workspaces = request(&outer_path, json!("Workspaces"))?;
        let workspaces = workspaces["Workspaces"]
            .as_array()
            .context("outer workspaces")?;
        let focused = workspaces
            .iter()
            .find(|workspace| workspace["is_focused"] == true)
            .context("focused outer workspace")?;
        let original_workspace = focused["id"].clone();
        let test_workspace = workspaces
            .iter()
            .find(|workspace| workspace["name"] == "dojo-tests")
            .or_else(|| {
                workspaces.iter().find(|workspace| {
                    workspace["output"] == focused["output"]
                        && workspace["is_active"] == false
                        && workspace["active_window_id"].is_null()
                        && workspace["name"].is_null()
                })
            })
            .context("no inactive empty workspace is available for isolated tests")?;
        let test_workspace_id = test_workspace["id"].clone();
        ensure!(
            test_workspace["is_active"] == false,
            "test workspace is visible on an active output"
        );
        ensure!(
            test_workspace_id != original_workspace,
            "test workspace is active"
        );
        if test_workspace["name"] != "dojo-tests" {
            request(
                &outer_path,
                json!({"Action":{"SetWorkspaceName":{"name":"dojo-tests","workspace":{"Id":test_workspace_id}}}}),
            )?;
        }
        request(
            &outer_path,
            json!({"Action":{"MoveWindowToWorkspace":{"window_id":host,"reference":{"Id":test_workspace_id},"focus":false}}}),
        )?;
        let after = request(&outer_path, json!("Workspaces"))?;
        ensure!(
            after["Workspaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|workspace| workspace["id"] == original_workspace
                    && workspace["is_focused"] == true),
            "isolating the test compositor changed the active workspace"
        );
        request(
            &outer_path,
            json!({"Action":{"MoveWindowToFloating":{"id":host}}}),
        )?;
        request(
            &outer_path,
            json!({"Action":{"SetWindowWidth":{"id":host,"change":{"SetFixed":1200}}}}),
        )?;
        request(
            &outer_path,
            json!({"Action":{"SetWindowHeight":{"id":host,"change":{"SetFixed":900}}}}),
        )?;
        std::thread::sleep(Duration::from_millis(700));
    }
    Ok(())
}
