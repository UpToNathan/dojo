use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use dojo_cua_worker::{
    ConfiguredDriverOptions, EmbeddedEnvironmentVariable, OwnedWorker, RuntimeAuthorizationOptions,
    SessionPermissionMode, TrustedSessionOptions, WorkerOptions, WorkerRetirement,
};
use serde_json::Value;

pub(super) struct Driver {
    worker: OwnedWorker,
    session: String,
}

impl Driver {
    pub(super) fn open(
        socket: &Path,
        name: &str,
        idle: Duration,
        cancelled: &Arc<AtomicBool>,
        retirement: &Arc<Mutex<Option<WorkerRetirement>>>,
    ) -> Result<Self> {
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "desktop connection closed"
        );
        let binary = std::fs::canonicalize(binary_path()).context(
            "forked CUA Driver is not built; run scripts/build-cua-driver.sh through devenv",
        )?;
        verify_binary(&binary)?;
        let mut environment = Vec::new();
        for key in [
            "PATH",
            "HOME",
            "USER",
            "LOGNAME",
            "LANG",
            "XDG_RUNTIME_DIR",
            "WAYLAND_DISPLAY",
            "DISPLAY",
            "XDG_SESSION_TYPE",
            "DBUS_SESSION_BUS_ADDRESS",
            "LD_LIBRARY_PATH",
        ] {
            if let Ok(value) = std::env::var(key) {
                environment.push(EmbeddedEnvironmentVariable {
                    name: key.into(),
                    value,
                });
            }
        }
        environment.push(EmbeddedEnvironmentVariable {
            name: "NIRI_SOCKET".into(),
            value: socket
                .to_str()
                .context("Niri socket path must be UTF-8")?
                .into(),
        });
        let worker = OwnedWorker::spawn(
            WorkerOptions {
                binary_path: binary.to_str().context("Driver path must be UTF-8")?.into(),
                host_bundle_id: "dojo".into(),
                startup_timeout_ms: Some(10_000),
                shutdown_timeout_ms: Some(2_000),
                configured_driver: ConfiguredDriverOptions {
                    claude_code_compatibility: false,
                    authorization: RuntimeAuthorizationOptions {
                        allowed_modes: vec![SessionPermissionMode::Unrestricted],
                        compatibility_mode: SessionPermissionMode::Unrestricted,
                        compatibility_capability_manifest_path: None,
                        compatibility_bounded_manifest_path: None,
                        unrestricted_acknowledged: true,
                        max_session_ttl_seconds: 86_400,
                        max_idle_ttl_seconds: 86_400,
                    },
                },
                environment,
                inherit_stderr: true,
            },
            |handle| {
                let mut slot = retirement
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                if cancelled.load(Ordering::Acquire) {
                    let _ = handle.retire();
                } else {
                    *slot = Some(handle);
                }
            },
        )?;
        ensure!(
            !cancelled.load(Ordering::Acquire),
            "desktop disconnected during Driver startup"
        );
        let session = worker.bind_session(TrustedSessionOptions {
            public_session: name.into(),
            mode: SessionPermissionMode::Unrestricted,
            ttl_seconds: 86_400,
            idle_ttl_seconds: idle.as_secs(),
            capability_manifest_path: None,
            bounded_manifest_path: None,
        })?;
        Ok(Self { worker, session })
    }
    pub(super) fn pid(&self) -> u32 {
        self.worker.retirement_handle().pid()
    }
    pub(super) fn tools(&self) -> Result<Value> {
        let mut catalog = self.worker.list_tools_blocking()?;
        let tools = catalog["tools"]
            .as_array_mut()
            .context("invalid Driver tool catalog")?;
        tools.retain(|tool| tool["name"].as_str().is_some_and(admitted_tool));
        Ok(catalog)
    }
    pub(super) fn call(&self, name: String, arguments: Value) -> Result<Value> {
        ensure!(
            admitted_tool(&name),
            "Driver tool has no Dojo host admission: {name}"
        );
        Ok(self
            .worker
            .invoke_blocking(name, arguments, Some(self.session.clone()))?)
    }
}

pub(super) fn binary_path() -> std::path::PathBuf {
    std::env::var_os("DOJO_CUA_DRIVER_BINARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_CACHE_HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                        .join(".cache")
                })
                .join("dojo/cua-niri-target/debug/cua-driver")
        })
}

pub(super) fn artifact_manifest() -> std::path::PathBuf {
    let mut path = binary_path().into_os_string();
    path.push(".dojo.json");
    path.into()
}

fn digest(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn verify_binary(binary: &Path) -> Result<()> {
    verify_artifact(binary, &artifact_manifest())
}

fn verify_artifact(binary: &Path, manifest_path: &Path) -> Result<()> {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(manifest_path).context(
            "Driver artifact pin missing; run scripts/build-cua-driver.sh through devenv",
        )?)?;
    ensure!(
        manifest["upstream_commit"] == "88aff7e5424e9ed45b93ded85eb4111472ca5190",
        "Driver upstream pin differs"
    );
    ensure!(
        manifest["binary_sha256"].as_str() == Some(digest(binary)?.as_str()),
        "Driver binary digest differs; rebuild"
    );
    for source in [
        "vendor/cua/copy-plan.json",
        "vendor/cua/copy-inventory.json",
        "vendor/cua/metadata/niri-driver.patch",
        "vendor/niri/niri-ipc/Cargo.toml",
        "vendor/niri/niri-ipc/src/lib.rs",
        "vendor/niri/niri-ipc/src/socket.rs",
        "vendor/niri/niri-ipc/src/state.rs",
        "crates/dojo-native-seat/Cargo.toml",
        "vendor/cua/upstream/libs/cua-driver/rust/Cargo.lock",
        "crates/dojo-native-seat/src/lib.rs",
    ] {
        ensure!(
            manifest["sources"][source].as_str()
                == Some(digest(&Path::new(env!("CARGO_MANIFEST_DIR")).join(source))?.as_str()),
            "Driver source pin differs: {source}; rebuild"
        );
    }
    let inventory: Value = serde_json::from_slice(&std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/cua/copy-inventory.json"),
    )?)?;
    for file in inventory["files"]
        .as_array()
        .context("invalid pinned source inventory")?
    {
        let source = file["path"]
            .as_str()
            .context("missing imported source path")?;
        if source.starts_with("vendor/cua/upstream/libs/cua-driver/rust/")
            || source.starts_with("vendor/cua/upstream/libs/cua/crates/cua-telemetry/")
            || source == "vendor/cua/upstream/libs/cua/crates/cua-teleport/src/ux/targets.json"
        {
            ensure!(
                file["sha256"].as_str()
                    == Some(digest(&Path::new(env!("CARGO_MANIFEST_DIR")).join(source))?.as_str()),
                "Driver implementation source differs: {source}; rebuild"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_admission_excludes_other_execution_and_mutation_routes() {
        for name in [
            "run_steps",
            "run_script",
            "replay_trajectory",
            "start_session",
            "escalate_session",
            "install_ffmpeg",
            "browser_navigate",
            "browser_click",
            "set_value",
            "set_config",
            "future_tool",
        ] {
            assert!(!admitted_tool(name), "unexpected host admission: {name}");
        }
        for name in [
            "click",
            "press_key",
            "type_text",
            "scroll",
            "bring_to_front",
            "get_window_state",
        ] {
            assert!(admitted_tool(name));
        }
    }
    #[test]
    fn artifact_requires_exact_binary_and_source_digests() {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("driver");
        let manifest_path = directory.path().join("driver.dojo.json");
        std::fs::write(&binary, b"original binary").unwrap();
        let mut sources = serde_json::Map::new();
        for source in [
            "vendor/cua/copy-plan.json",
            "vendor/cua/copy-inventory.json",
            "vendor/cua/metadata/niri-driver.patch",
            "vendor/niri/niri-ipc/Cargo.toml",
            "vendor/niri/niri-ipc/src/lib.rs",
            "vendor/niri/niri-ipc/src/socket.rs",
            "vendor/niri/niri-ipc/src/state.rs",
            "crates/dojo-native-seat/Cargo.toml",
            "vendor/cua/upstream/libs/cua-driver/rust/Cargo.lock",
            "crates/dojo-native-seat/src/lib.rs",
        ] {
            sources.insert(
                source.into(),
                Value::String(digest(&Path::new(env!("CARGO_MANIFEST_DIR")).join(source)).unwrap()),
            );
        }
        let mut manifest = serde_json::json!({"upstream_commit":"88aff7e5424e9ed45b93ded85eb4111472ca5190","binary_sha256":digest(&binary).unwrap(),"sources":sources});
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        verify_artifact(&binary, &manifest_path).unwrap();
        std::fs::write(&binary, b"different binary").unwrap();
        assert!(
            verify_artifact(&binary, &manifest_path)
                .unwrap_err()
                .to_string()
                .contains("binary digest differs")
        );
        std::fs::write(&binary, b"original binary").unwrap();
        manifest["sources"]["crates/dojo-native-seat/src/lib.rs"] = Value::String("wrong".into());
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(
            verify_artifact(&binary, &manifest_path)
                .unwrap_err()
                .to_string()
                .contains("source pin differs")
        );
    }
}

fn admitted_tool(name: &str) -> bool {
    matches!(
        name,
        "list_apps"
            | "list_windows"
            | "get_window_state"
            | "get_desktop_state"
            | "get_screen_size"
            | "get_config"
            | "check_permissions"
            | "health_report"
            | "get_accessibility_tree"
            | "get_agent_cursor_state"
            | "get_cursor_position"
            | "verify_state"
            | "zoom"
            | "click"
            | "press_key"
            | "type_text"
            | "scroll"
            | "bring_to_front"
    )
}
