use crate::ActionCompletion;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("invalid SDK configuration: {reason}")]
    Configuration { reason: String },
    #[error("invalid arguments for {tool}: {reason}")]
    InvalidArguments { tool: String, reason: String },
    #[error("daemon transport error on {socket_path}: {reason}")]
    Transport { socket_path: String, reason: String },
    #[error("invalid daemon response: {reason}")]
    Protocol { reason: String },
    #[error("{tool} failed: {message}")]
    Tool {
        tool: String,
        message: String,
        error_code: String,
    },
    #[error("the Cua Driver SDK has been shut down")]
    Shutdown,
    #[error(
        "runtime_already_exists: one direct Cua Driver runtime is already active in this process"
    )]
    RuntimeAlreadyExists,
    #[error("private worker error: {reason}")]
    Worker { reason: String },
    #[error("remote Driver error: {reason}")]
    Remote { reason: String },
    #[error("driver action was interrupted ({completion:?}): {reason}")]
    ActionInterrupted {
        completion: ActionCompletion,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverMetadata {
    pub driver_version: String,
    pub contract_version: String,
    pub tools_list_schema_version: String,
    pub capability_version: String,
    pub mcp_protocol_version: String,
    pub pid: u32,
    pub embedded: bool,
    pub host_bundle_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPermissionMode {
    Standard,
    Bounded,
    Unrestricted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAuthorizationOptions {
    pub allowed_modes: Vec<SessionPermissionMode>,
    /// Mode inherited by calls made through the released `CuaDriver` object
    /// rather than a trusted session-bound action surface.
    pub compatibility_mode: SessionPermissionMode,
    /// Optional in standard and unrestricted; required when compatibility mode is bounded.
    pub compatibility_capability_manifest_path: Option<String>,
    /// Deprecated alias for `compatibility_capability_manifest_path`.
    pub compatibility_bounded_manifest_path: Option<String>,
    pub unrestricted_acknowledged: bool,
    pub max_session_ttl_seconds: u64,
    pub max_idle_ttl_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredDriverOptions {
    pub claude_code_compatibility: bool,
    pub authorization: RuntimeAuthorizationOptions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedSessionOptions {
    pub public_session: String,
    pub mode: SessionPermissionMode,
    pub ttl_seconds: u64,
    pub idle_ttl_seconds: u64,
    pub capability_manifest_path: Option<String>,
    /// Deprecated alias for `capability_manifest_path`.
    pub bounded_manifest_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedEnvironmentVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerOptions {
    pub binary_path: String,
    pub host_bundle_id: String,
    pub startup_timeout_ms: Option<u64>,
    pub shutdown_timeout_ms: Option<u64>,
    pub configured_driver: ConfiguredDriverOptions,
    pub environment: Vec<EmbeddedEnvironmentVariable>,
    pub inherit_stderr: bool,
}
